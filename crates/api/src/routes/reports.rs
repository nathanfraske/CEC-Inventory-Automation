//! Cross-cutting worklists and exports (scope §12.5, §18, §20): the reorder list, receiving
//! reconciliation, and a no-lock-in full inventory export (JSON + CSV).

use axum::extract::State;
use axum::http::header;
use axum::response::IntoResponse;
use axum::Json;
use chrono::Utc;
use serde::Serialize;
use serde_json::{json, Value};
use sqlx::FromRow;
use uuid::Uuid;

use crate::error::ApiResult;
use crate::AppState;

#[derive(Serialize, FromRow)]
pub struct ReorderItem {
    pub stock_id: Uuid,
    pub product_id: Uuid,
    pub model: Option<String>,
    pub location_bin: Option<String>,
    pub quantity_on_hand: i32,
    pub reorder_point: i32,
}

/// Stock at or below its reorder point (scope §20 cross-cutting).
pub async fn reorder_list(State(s): State<AppState>) -> ApiResult<Json<Vec<ReorderItem>>> {
    let rows = sqlx::query_as::<_, ReorderItem>(
        "SELECT si.id AS stock_id, si.product_id, p.model, si.location_bin, \
                si.quantity_on_hand, si.reorder_point \
         FROM stock_item si LEFT JOIN product p ON p.id = si.product_id \
         WHERE si.reorder_point IS NOT NULL AND si.quantity_on_hand <= si.reorder_point \
         ORDER BY si.quantity_on_hand",
    )
    .fetch_all(&s.db)
    .await?;
    Ok(Json(rows))
}

#[derive(Serialize, FromRow)]
pub struct Discrepancy {
    pub shipment_id: Uuid,
    pub purchase_id: Uuid,
    pub tracking_number: Option<String>,
}

/// Receiving reconciliation (scope §12.5): shipments the carrier marks delivered but whose
/// purchase has no intaked units yet — the "to receive" worklist.
pub async fn receiving_reconciliation(State(s): State<AppState>) -> ApiResult<Json<Value>> {
    let delivered_not_received = sqlx::query_as::<_, Discrepancy>(
        "SELECT sh.id AS shipment_id, sh.purchase_id, sh.tracking_number \
         FROM shipment sh \
         WHERE sh.status = 'delivered' AND NOT EXISTS ( \
           SELECT 1 FROM purchase_line_item li JOIN inventory_unit u ON u.line_item_id = li.id \
           WHERE li.purchase_id = sh.purchase_id) \
         ORDER BY sh.delivered_at",
    )
    .fetch_all(&s.db)
    .await?;
    Ok(Json(json!({
        "delivered_not_received": delivered_not_received,
        "count": delivered_not_received.len(),
    })))
}

async fn dump(s: &AppState, table: &str) -> Result<Vec<Value>, sqlx::Error> {
    sqlx::query_scalar::<_, Value>(&format!("SELECT to_jsonb(t) FROM {table} t"))
        .fetch_all(&s.db)
        .await
}

/// Full inventory export as JSON (scope §18) — a portable, no-lock-in snapshot of every
/// business table. `app_user` is deliberately excluded: it holds credentials (password hashes),
/// which must not ride in a portability export. Pair with `pg_dump` + the object-store archive
/// for an authoritative backup (the receipt files are not inlined here).
pub async fn export_json(State(s): State<AppState>) -> ApiResult<Json<Value>> {
    Ok(Json(json!({
        "exported_at": Utc::now(),
        "vendors": dump(&s, "vendor").await?,
        "vendor_return_policies": dump(&s, "vendor_return_policy").await?,
        "manufacturers": dump(&s, "manufacturer").await?,
        "products": dump(&s, "product").await?,
        "purchases": dump(&s, "purchase").await?,
        "line_items": dump(&s, "purchase_line_item").await?,
        "shipments": dump(&s, "shipment").await?,
        "shipment_events": dump(&s, "shipment_event").await?,
        "units": dump(&s, "inventory_unit").await?,
        "stock": dump(&s, "stock_item").await?,
        "systems": dump(&s, "system").await?,
        "system_validations": dump(&s, "system_validation").await?,
        "system_transfers": dump(&s, "system_transfer").await?,
        "cec_warranty_policies": dump(&s, "cec_warranty_policy").await?,
        "rma_cases": dump(&s, "rma_case").await?,
        "trade_ins": dump(&s, "trade_in").await?,
        "trade_in_units": dump(&s, "trade_in_unit").await?,
        "unit_events": dump(&s, "unit_event").await?,
    })))
}

fn csv_field(v: &str) -> String {
    if v.contains([',', '"', '\n']) {
        format!("\"{}\"", v.replace('"', "\"\""))
    } else {
        v.to_string()
    }
}

#[derive(FromRow)]
struct UnitCsvRow {
    id: Uuid,
    serial_number: Option<String>,
    product_id: Option<Uuid>,
    status: String,
    owner: String,
    condition: String,
    asset_tag: Option<String>,
    location_bin: Option<String>,
    unit_cost: Option<rust_decimal::Decimal>,
}

/// Units as CSV (scope §18 portability).
pub async fn export_units_csv(State(s): State<AppState>) -> ApiResult<impl IntoResponse> {
    let rows = sqlx::query_as::<_, UnitCsvRow>(
        "SELECT id, serial_number, product_id, status::text AS status, owner::text AS owner, \
                condition::text AS condition, asset_tag, location_bin, unit_cost \
         FROM inventory_unit ORDER BY intake_at",
    )
    .fetch_all(&s.db)
    .await?;

    let mut out = String::from(
        "id,serial_number,product_id,status,owner,condition,asset_tag,location_bin,unit_cost\n",
    );
    for r in rows {
        let fields = [
            r.id.to_string(),
            r.serial_number.unwrap_or_default(),
            r.product_id.map(|p| p.to_string()).unwrap_or_default(),
            r.status,
            r.owner,
            r.condition,
            r.asset_tag.unwrap_or_default(),
            r.location_bin.unwrap_or_default(),
            r.unit_cost.map(|c| c.to_string()).unwrap_or_default(),
        ];
        let line: Vec<String> = fields.iter().map(|f| csv_field(f)).collect();
        out.push_str(&line.join(","));
        out.push('\n');
    }

    Ok((
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"units.csv\"",
            ),
        ],
        out,
    ))
}
