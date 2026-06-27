//! CEC warranty policy CRUD and the per-unit warranty computation/view (scope Section 5).
//! `recompute_warranty` loads a unit's full warranty inputs, runs the pure rules in
//! `crate::warranty`, and persists the results; `warranty_view` reads them back with the
//! live remaining/active state.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::FromRow;
use uuid::Uuid;

use cec_inventory_domain::{CecWarrantyClass, MfrWarrantyBasis, UnitEventType};

use crate::error::{ApiError, ApiResult};
use crate::events::log_unit_event;
use crate::warranty;
use crate::AppState;

// ---------------- CEC warranty policy ----------------

#[derive(Serialize, FromRow)]
pub struct CecWarrantyPolicy {
    pub id: Uuid,
    pub warranty_class: CecWarrantyClass,
    pub category: Option<String>,
    pub term_months: i32,
    pub transferable: bool,
    pub reset_on_transfer: bool,
    pub clock_pauses_when_invalidated: bool,
    pub notes: Option<String>,
}

#[derive(Deserialize)]
pub struct CreatePolicy {
    pub warranty_class: CecWarrantyClass,
    #[serde(default)]
    pub category: Option<String>,
    pub term_months: i32,
    #[serde(default)]
    pub transferable: bool,
    #[serde(default)]
    pub reset_on_transfer: bool,
    #[serde(default)]
    pub clock_pauses_when_invalidated: bool,
    #[serde(default)]
    pub notes: Option<String>,
}

pub async fn create_policy(
    State(s): State<AppState>,
    Json(b): Json<CreatePolicy>,
) -> ApiResult<(StatusCode, Json<CecWarrantyPolicy>)> {
    let p = sqlx::query_as::<_, CecWarrantyPolicy>(
        "INSERT INTO cec_warranty_policy \
         (warranty_class, category, term_months, transferable, reset_on_transfer, clock_pauses_when_invalidated, notes) \
         VALUES ($1,$2,$3,$4,$5,$6,$7) RETURNING *",
    )
    .bind(b.warranty_class)
    .bind(b.category)
    .bind(b.term_months)
    .bind(b.transferable)
    .bind(b.reset_on_transfer)
    .bind(b.clock_pauses_when_invalidated)
    .bind(b.notes)
    .fetch_one(&s.db)
    .await?;
    Ok((StatusCode::CREATED, Json(p)))
}

pub async fn list_policies(State(s): State<AppState>) -> ApiResult<Json<Vec<CecWarrantyPolicy>>> {
    let rows = sqlx::query_as::<_, CecWarrantyPolicy>(
        "SELECT * FROM cec_warranty_policy ORDER BY warranty_class, category NULLS FIRST",
    )
    .fetch_all(&s.db)
    .await?;
    Ok(Json(rows))
}

// ---------------- per-unit warranty ----------------

#[derive(FromRow)]
struct RecomputeRow {
    serial_number: Option<String>,
    product_id: Option<Uuid>,
    line_item_id: Option<Uuid>,
    cec_warranty_class: CecWarrantyClass,
    cec_warranty_start: Option<DateTime<Utc>>,
    registered: bool,
    system_id: Option<Uuid>,
    mfr_warranty_basis: MfrWarrantyBasis,
    replaces_unit_id: Option<Uuid>,
    product_category: Option<String>,
    product_term: Option<i32>,
    mfr_term: Option<i32>,
    replacement_warranty_days: Option<i32>,
    purchase_datetime: Option<DateTime<Utc>>,
}

#[derive(Serialize)]
pub struct WarrantyView {
    pub unit_id: Uuid,
    pub mfr_warranty_expires: Option<NaiveDate>,
    pub mfr_days_left: Option<i64>,
    pub cec_warranty_class: CecWarrantyClass,
    pub cec_warranty_expires: Option<NaiveDate>,
    pub cec_days_left: Option<i64>,
    pub cec_warranty_active: bool,
    pub rma_eligible: bool,
    pub rma_block_reason: Option<String>,
}

async fn system_validated(s: &AppState, system_id: Option<Uuid>) -> Result<bool, sqlx::Error> {
    match system_id {
        None => Ok(false),
        Some(sid) => {
            let st: Option<String> =
                sqlx::query_scalar("SELECT validation_state::text FROM system WHERE id = $1")
                    .bind(sid)
                    .fetch_optional(&s.db)
                    .await?;
            Ok(st.as_deref() == Some("validated"))
        }
    }
}

/// Recompute and persist both warranty clocks and RMA readiness for a unit.
pub async fn recompute_warranty(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<WarrantyView>> {
    let row = sqlx::query_as::<_, RecomputeRow>(
        "SELECT u.serial_number, u.product_id, u.line_item_id, u.cec_warranty_class, \
                u.cec_warranty_start, u.registered, u.system_id, u.mfr_warranty_basis, \
                u.replaces_unit_id, p.category AS product_category, \
                p.default_warranty_months AS product_term, m.default_warranty_months AS mfr_term, \
                m.replacement_warranty_days, pur.purchase_datetime \
         FROM inventory_unit u \
         LEFT JOIN product p ON p.id = u.product_id \
         LEFT JOIN manufacturer m ON m.id = p.manufacturer_id \
         LEFT JOIN purchase_line_item li ON li.id = u.line_item_id \
         LEFT JOIN purchase pur ON pur.id = li.purchase_id \
         WHERE u.id = $1",
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| ApiError::NotFound("unit not found".into()))?;

    let term = row.product_term.or(row.mfr_term);
    let start = row.purchase_datetime.map(|d| d.date_naive());

    let predecessor_expiry =
        if matches!(row.mfr_warranty_basis, MfrWarrantyBasis::OriginalRemainder) {
            match row.replaces_unit_id {
                Some(pred) => sqlx::query_scalar(
                    "SELECT mfr_warranty_expires FROM inventory_unit WHERE id = $1",
                )
                .bind(pred)
                .fetch_optional(&s.db)
                .await?
                .flatten(),
                None => None,
            }
        } else {
            None
        };

    let mfr_expires = warranty::mfr_expiry(
        start,
        term,
        row.mfr_warranty_basis,
        row.replacement_warranty_days,
        predecessor_expiry,
        None,
    );

    // CEC term: prefer a category-specific policy over the default (NULL category).
    let cec_term: Option<i32> = sqlx::query_scalar(
        "SELECT term_months FROM cec_warranty_policy \
         WHERE warranty_class = $1 AND (category = $2 OR category IS NULL) \
         ORDER BY category NULLS LAST LIMIT 1",
    )
    .bind(row.cec_warranty_class)
    .bind(row.product_category.as_deref())
    .fetch_optional(&s.db)
    .await?;
    let cec_start = row.cec_warranty_start.map(|d| d.date_naive());
    let cec_expires = warranty::cec_expiry(cec_start, row.cec_warranty_class, cec_term);

    let today = Utc::now().date_naive();
    let (rma_eligible, block) = warranty::rma_eligibility(
        row.serial_number.is_some(),
        row.product_id.is_some(),
        row.line_item_id.is_some(),
        mfr_expires,
        today,
    );
    let validated = system_validated(&s, row.system_id).await?;
    let cec_active =
        warranty::cec_warranty_active(row.cec_warranty_class, cec_expires, validated, today);

    let mut tx = s.db.begin().await?;
    sqlx::query(
        "UPDATE inventory_unit SET mfr_warranty_expires = $1, cec_warranty_expires = $2, \
         rma_eligible = $3, rma_block_reason = $4 WHERE id = $5",
    )
    .bind(mfr_expires)
    .bind(cec_expires)
    .bind(rma_eligible)
    .bind(block)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    log_unit_event(
        &mut *tx,
        id,
        UnitEventType::Note,
        None,
        None,
        Some("warranty_recompute"),
        row.system_id,
        Some(json!({
            "mfr_warranty_expires": mfr_expires,
            "cec_warranty_expires": cec_expires,
            "rma_eligible": rma_eligible,
            "rma_block_reason": block,
        })),
    )
    .await?;
    tx.commit().await?;

    let _ = row.registered; // reserved for §7.9 registration extension
    Ok(Json(WarrantyView {
        unit_id: id,
        mfr_warranty_expires: mfr_expires,
        mfr_days_left: mfr_expires.map(|e| (e - today).num_days()),
        cec_warranty_class: row.cec_warranty_class,
        cec_warranty_expires: cec_expires,
        cec_days_left: cec_expires.map(|e| (e - today).num_days()),
        cec_warranty_active: cec_active,
        rma_eligible,
        rma_block_reason: block.map(|b| b.to_string()),
    }))
}

#[derive(FromRow)]
struct StoredWarranty {
    serial_number: Option<String>,
    cec_warranty_class: CecWarrantyClass,
    mfr_warranty_expires: Option<NaiveDate>,
    cec_warranty_expires: Option<NaiveDate>,
    rma_eligible: Option<bool>,
    rma_block_reason: Option<String>,
    system_id: Option<Uuid>,
}

/// Read the stored two-clock view plus the live `cec_warranty_active` (which depends on the
/// system's current validation state).
pub async fn warranty_view(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<WarrantyView>> {
    let w = sqlx::query_as::<_, StoredWarranty>(
        "SELECT serial_number, cec_warranty_class, mfr_warranty_expires, cec_warranty_expires, \
         rma_eligible, rma_block_reason, system_id FROM inventory_unit WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| ApiError::NotFound("unit not found".into()))?;

    let today = Utc::now().date_naive();
    let validated = system_validated(&s, w.system_id).await?;
    let cec_active = warranty::cec_warranty_active(
        w.cec_warranty_class,
        w.cec_warranty_expires,
        validated,
        today,
    );
    let _ = w.serial_number;
    Ok(Json(WarrantyView {
        unit_id: id,
        mfr_warranty_expires: w.mfr_warranty_expires,
        mfr_days_left: w.mfr_warranty_expires.map(|e| (e - today).num_days()),
        cec_warranty_class: w.cec_warranty_class,
        cec_warranty_expires: w.cec_warranty_expires,
        cec_days_left: w.cec_warranty_expires.map(|e| (e - today).num_days()),
        cec_warranty_active: cec_active,
        rma_eligible: w.rma_eligible.unwrap_or(false),
        rma_block_reason: w.rma_block_reason,
    }))
}
