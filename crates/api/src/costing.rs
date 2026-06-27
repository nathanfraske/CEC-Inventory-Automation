//! Landed-cost allocation (scope Section 14). `unit_cost` is the landed cost, not the
//! sticker price: the line price plus the line's share of order-level shipping + tax,
//! net of order-level discount. Default rule: weight the order-level extra by line total,
//! then per-unit cost = line landed total / quantity (INV-OQ-20).

use axum::extract::{Path, Query, State};
use axum::Json;
use rust_decimal::{Decimal, RoundingStrategy};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use cec_inventory_domain::UnitEventType;

use crate::error::{ApiError, ApiResult};
use crate::events::log_unit_event;
use crate::AppState;

const MONEY: RoundingStrategy = RoundingStrategy::MidpointAwayFromZero;

pub struct LineInput {
    pub id: Uuid,
    pub quantity: i32,
    pub line_total: Decimal,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct LineAllocation {
    pub line_id: Uuid,
    pub line_total: Decimal,
    pub allocated_extra: Decimal,
    pub allocated_landed_cost: Decimal,
    pub per_unit_cost: Decimal,
}

/// Pure allocation. `extra = shipping + tax - discount` is spread across lines weighted by
/// line total (equal weights if all line totals are zero); the rounding remainder lands on
/// the last line so the parts sum to the whole.
pub fn allocate_landed_cost(
    lines: &[LineInput],
    shipping: Decimal,
    tax: Decimal,
    discount: Decimal,
) -> Vec<LineAllocation> {
    let n = lines.len();
    if n == 0 {
        return Vec::new();
    }
    let extra_total = shipping + tax - discount;
    let sum: Decimal = lines.iter().map(|l| l.line_total).sum();
    let mut running = Decimal::ZERO;
    let mut out = Vec::with_capacity(n);

    for (i, l) in lines.iter().enumerate() {
        let extra = if i == n - 1 {
            // Last line absorbs the rounding remainder.
            (extra_total - running).round_dp_with_strategy(2, MONEY)
        } else {
            let weight = if sum.is_zero() {
                Decimal::ONE / Decimal::from(n)
            } else {
                l.line_total / sum
            };
            let e = (extra_total * weight).round_dp_with_strategy(2, MONEY);
            running += e;
            e
        };
        let landed = (l.line_total + extra).round_dp_with_strategy(2, MONEY);
        let qty = if l.quantity <= 0 { 1 } else { l.quantity };
        let per_unit = (landed / Decimal::from(qty)).round_dp_with_strategy(2, MONEY);
        out.push(LineAllocation {
            line_id: l.id,
            line_total: l.line_total,
            allocated_extra: extra,
            allocated_landed_cost: landed,
            per_unit_cost: per_unit,
        });
    }
    out
}

#[derive(Deserialize)]
pub struct AllocateParams {
    /// Also write each line's per-unit landed cost onto its bound units. Default true.
    #[serde(default = "default_true")]
    pub apply_to_units: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Serialize)]
pub struct AllocLineOut {
    #[serde(flatten)]
    pub alloc: LineAllocation,
    pub units_updated: u64,
}

#[derive(Serialize)]
pub struct AllocationResult {
    pub purchase_id: Uuid,
    pub shipping: Decimal,
    pub tax: Decimal,
    pub discount_total: Decimal,
    pub extra_total: Decimal,
    pub lines: Vec<AllocLineOut>,
}

#[derive(sqlx::FromRow)]
struct PurchaseCosts {
    shipping: Option<Decimal>,
    tax: Option<Decimal>,
    discount_total: Option<Decimal>,
}

#[derive(sqlx::FromRow)]
struct LineRow {
    id: Uuid,
    quantity: i32,
    line_total: Option<Decimal>,
    unit_price: Option<Decimal>,
}

pub async fn allocate_costs(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Query(params): Query<AllocateParams>,
) -> ApiResult<Json<AllocationResult>> {
    let costs = sqlx::query_as::<_, PurchaseCosts>(
        "SELECT shipping, tax, discount_total FROM purchase WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| ApiError::NotFound("purchase not found".into()))?;

    let rows = sqlx::query_as::<_, LineRow>(
        "SELECT id, quantity, line_total, unit_price FROM purchase_line_item \
         WHERE purchase_id = $1 ORDER BY id",
    )
    .bind(id)
    .fetch_all(&s.db)
    .await?;
    if rows.is_empty() {
        return Err(ApiError::BadRequest(
            "purchase has no line items to allocate".into(),
        ));
    }

    let lines: Vec<LineInput> = rows
        .iter()
        .map(|r| {
            let effective = r
                .line_total
                .or_else(|| r.unit_price.map(|p| p * Decimal::from(r.quantity.max(0))))
                .unwrap_or(Decimal::ZERO);
            LineInput {
                id: r.id,
                quantity: r.quantity,
                line_total: effective,
            }
        })
        .collect();

    let shipping = costs.shipping.unwrap_or(Decimal::ZERO);
    let tax = costs.tax.unwrap_or(Decimal::ZERO);
    let discount = costs.discount_total.unwrap_or(Decimal::ZERO);
    let allocations = allocate_landed_cost(&lines, shipping, tax, discount);

    let mut tx = s.db.begin().await?;
    let mut out_lines = Vec::with_capacity(allocations.len());
    for alloc in allocations {
        sqlx::query("UPDATE purchase_line_item SET allocated_landed_cost = $1 WHERE id = $2")
            .bind(alloc.allocated_landed_cost)
            .bind(alloc.line_id)
            .execute(&mut *tx)
            .await?;

        let mut units_updated = 0u64;
        if params.apply_to_units {
            let unit_ids: Vec<Uuid> =
                sqlx::query_scalar("SELECT id FROM inventory_unit WHERE line_item_id = $1")
                    .bind(alloc.line_id)
                    .fetch_all(&mut *tx)
                    .await?;
            for uid in &unit_ids {
                sqlx::query("UPDATE inventory_unit SET unit_cost = $1 WHERE id = $2")
                    .bind(alloc.per_unit_cost)
                    .bind(uid)
                    .execute(&mut *tx)
                    .await?;
                log_unit_event(
                    &mut *tx,
                    *uid,
                    UnitEventType::Note,
                    None,
                    Some(&alloc.per_unit_cost.to_string()),
                    Some("landed_cost_allocation"),
                    None,
                    Some(json!({ "field": "unit_cost", "source": "landed_cost_allocation" })),
                )
                .await?;
            }
            units_updated = unit_ids.len() as u64;
        }
        out_lines.push(AllocLineOut {
            alloc,
            units_updated,
        });
    }
    tx.commit().await?;

    Ok(Json(AllocationResult {
        purchase_id: id,
        shipping,
        tax,
        discount_total: discount,
        extra_total: shipping + tax - discount,
        lines: out_lines,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    #[test]
    fn weights_extra_by_line_total_and_divides_by_qty() {
        // Two lines: 1000 (qty 4) and 0... use 1500 (qty 1). Shipping 50, tax 100, no discount.
        let lines = vec![
            LineInput {
                id: Uuid::nil(),
                quantity: 4,
                line_total: d("1000.00"),
            },
            LineInput {
                id: Uuid::from_u128(1),
                quantity: 1,
                line_total: d("1500.00"),
            },
        ];
        let allocs = allocate_landed_cost(&lines, d("50.00"), d("100.00"), d("0.00"));
        // extra_total = 150; line1 weight 0.4 -> 60, line2 -> 90 (remainder).
        assert_eq!(allocs[0].allocated_extra, d("60.00"));
        assert_eq!(allocs[1].allocated_extra, d("90.00"));
        assert_eq!(allocs[0].allocated_landed_cost, d("1060.00"));
        assert_eq!(allocs[1].allocated_landed_cost, d("1590.00"));
        // per-unit: line1 1060/4 = 265.00, line2 1590/1 = 1590.00
        assert_eq!(allocs[0].per_unit_cost, d("265.00"));
        assert_eq!(allocs[1].per_unit_cost, d("1590.00"));
        // the parts sum to the whole
        let total_extra: Decimal = allocs.iter().map(|a| a.allocated_extra).sum();
        assert_eq!(total_extra, d("150.00"));
    }

    #[test]
    fn discount_reduces_landed_cost() {
        let lines = vec![LineInput {
            id: Uuid::nil(),
            quantity: 2,
            line_total: d("200.00"),
        }];
        let allocs = allocate_landed_cost(&lines, d("0.00"), d("0.00"), d("40.00"));
        assert_eq!(allocs[0].allocated_extra, d("-40.00"));
        assert_eq!(allocs[0].allocated_landed_cost, d("160.00"));
        assert_eq!(allocs[0].per_unit_cost, d("80.00"));
    }

    #[test]
    fn zero_line_totals_split_evenly() {
        let lines = vec![
            LineInput {
                id: Uuid::nil(),
                quantity: 1,
                line_total: d("0.00"),
            },
            LineInput {
                id: Uuid::from_u128(1),
                quantity: 1,
                line_total: d("0.00"),
            },
        ];
        let allocs = allocate_landed_cost(&lines, d("10.00"), d("0.00"), d("0.00"));
        assert_eq!(allocs[0].allocated_extra, d("5.00"));
        assert_eq!(allocs[1].allocated_extra, d("5.00"));
    }
}
