//! No-receipt intakes (scope §8 trade-in, §9 opening-balance). Both resolve units to
//! `owner = shop` and set RMA readiness from the proof situation, never blocking the
//! operator from taking a part in. Opening-balance rides a synthetic `Purchase`.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgConnection;
use uuid::Uuid;

use cec_inventory_domain::{
    AcquisitionMethod, ConditionKind, ProofStatus, SerialSource, UnitEventType,
};

use crate::error::ApiResult;
use crate::events::log_unit_event;
use crate::AppState;

#[derive(Deserialize)]
pub struct IntakeUnitReq {
    pub product_id: Uuid,
    #[serde(default)]
    pub serial_number: Option<String>,
    #[serde(default)]
    pub serial_source: Option<SerialSource>,
    #[serde(default = "default_condition")]
    pub condition: ConditionKind,
    #[serde(default)]
    pub location_bin: Option<String>,
    #[serde(default)]
    pub unit_cost: Option<Decimal>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub intake_by: Option<String>,
}

fn default_condition() -> ConditionKind {
    ConditionKind::Unknown
}

/// Insert one intake unit (owner = shop) and log its `intake` event. Returns the unit id.
#[allow(clippy::too_many_arguments)]
async fn insert_intake_unit(
    conn: &mut PgConnection,
    u: &IntakeUnitReq,
    line_item_id: Option<Uuid>,
    acquisition: AcquisitionMethod,
    rma_eligible: Option<bool>,
    rma_block_reason: Option<&str>,
) -> Result<Uuid, sqlx::Error> {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO inventory_unit \
         (product_id, line_item_id, owner, serial_number, serial_source, condition, \
          acquisition_method, location_bin, unit_cost, notes, intake_by, rma_eligible, rma_block_reason) \
         VALUES ($1,$2,'shop',$3,$4,$5,$6,$7,$8,$9,$10,$11,$12) RETURNING id",
    )
    .bind(u.product_id)
    .bind(line_item_id)
    .bind(u.serial_number.as_deref())
    .bind(u.serial_source)
    .bind(u.condition)
    .bind(acquisition)
    .bind(u.location_bin.as_deref())
    .bind(u.unit_cost)
    .bind(u.notes.as_deref())
    .bind(u.intake_by.as_deref())
    .bind(rma_eligible)
    .bind(rma_block_reason)
    .fetch_one(&mut *conn)
    .await?;

    log_unit_event(
        &mut *conn,
        id,
        UnitEventType::Intake,
        None,
        Some("in_stock"),
        u.intake_by.as_deref(),
        None,
        Some(json!({
            "acquisition_method": serde_json::to_value(acquisition).ok(),
            "serial_number": u.serial_number,
            "rma_block_reason": rma_block_reason,
        })),
    )
    .await?;
    Ok(id)
}

/// Map a proof situation to RMA readiness (scope §8). `provided` is left pending (None) so
/// a later warranty recompute can decide; missing proof blocks with a recorded reason.
fn readiness_for_proof(p: ProofStatus) -> (Option<bool>, Option<&'static str>) {
    match p {
        ProofStatus::Provided => (None, None),
        ProofStatus::CustomerHasWillSend => (Some(false), Some("awaiting_proof_from_customer")),
        ProofStatus::CustomerLacks | ProofStatus::None => {
            (Some(false), Some("no_proof_of_purchase"))
        }
    }
}

// ---------------- trade-in ----------------

#[derive(Deserialize)]
pub struct CreateTradeIn {
    #[serde(default)]
    pub customer_ref: Option<String>,
    #[serde(default)]
    pub source_notes: Option<String>,
    pub proof_of_purchase_status: ProofStatus,
    #[serde(default = "empty_array")]
    pub proof_files: Value,
    pub units: Vec<IntakeUnitReq>,
}

fn empty_array() -> Value {
    json!([])
}

#[derive(Serialize)]
pub struct IntakeResult {
    pub trade_in_id: Option<Uuid>,
    pub purchase_id: Option<Uuid>,
    pub unit_ids: Vec<Uuid>,
}

pub async fn create_trade_in(
    State(s): State<AppState>,
    Json(body): Json<CreateTradeIn>,
) -> ApiResult<(StatusCode, Json<IntakeResult>)> {
    let (elig, reason) = readiness_for_proof(body.proof_of_purchase_status);
    let mut tx = s.db.begin().await?;

    let trade_in_id: Uuid = sqlx::query_scalar(
        "INSERT INTO trade_in (customer_ref, source_notes, proof_of_purchase_status, proof_files) \
         VALUES ($1,$2,$3,$4) RETURNING id",
    )
    .bind(body.customer_ref.as_deref())
    .bind(body.source_notes.as_deref())
    .bind(body.proof_of_purchase_status)
    .bind(&body.proof_files)
    .fetch_one(&mut *tx)
    .await?;

    let mut unit_ids = Vec::with_capacity(body.units.len());
    for u in &body.units {
        let uid =
            insert_intake_unit(&mut tx, u, None, AcquisitionMethod::TradeIn, elig, reason).await?;
        sqlx::query("INSERT INTO trade_in_unit (trade_in_id, unit_id) VALUES ($1,$2)")
            .bind(trade_in_id)
            .bind(uid)
            .execute(&mut *tx)
            .await?;
        unit_ids.push(uid);
    }
    tx.commit().await?;

    Ok((
        StatusCode::CREATED,
        Json(IntakeResult {
            trade_in_id: Some(trade_in_id),
            purchase_id: None,
            unit_ids,
        }),
    ))
}

// ---------------- opening balance ----------------

#[derive(Deserialize)]
pub struct CreateOpeningBalance {
    #[serde(default)]
    pub vendor_id: Option<Uuid>,
    #[serde(default)]
    pub purchase_datetime: Option<DateTime<Utc>>,
    /// True when the origin (datetime/vendor/cost) has been reconstructed. Unknown origin
    /// stays not-RMA-able (`no_proof_of_purchase`) until reconstructed (scope §9).
    #[serde(default)]
    pub origin_known: bool,
    pub units: Vec<IntakeUnitReq>,
}

pub async fn create_opening_balance(
    State(s): State<AppState>,
    Json(body): Json<CreateOpeningBalance>,
) -> ApiResult<(StatusCode, Json<IntakeResult>)> {
    let (elig, reason) = if body.origin_known {
        (None, None)
    } else {
        (Some(false), Some("no_proof_of_purchase"))
    };

    let mut tx = s.db.begin().await?;
    let purchase_id: Uuid = sqlx::query_scalar(
        "INSERT INTO purchase (vendor_id, purchase_datetime, source_type, created_by) \
         VALUES ($1,$2,'opening_balance','opening_balance') RETURNING id",
    )
    .bind(body.vendor_id)
    .bind(body.purchase_datetime)
    .fetch_one(&mut *tx)
    .await?;

    // Known origin gets a synthetic line item so the unit has a proof chain.
    let line_item_id: Option<Uuid> = if body.origin_known {
        let lid: Uuid = sqlx::query_scalar(
            "INSERT INTO purchase_line_item (purchase_id, quantity, description_as_printed) \
             VALUES ($1,$2,'opening balance') RETURNING id",
        )
        .bind(purchase_id)
        .bind(body.units.len() as i32)
        .fetch_one(&mut *tx)
        .await?;
        Some(lid)
    } else {
        None
    };

    let mut unit_ids = Vec::with_capacity(body.units.len());
    for u in &body.units {
        let uid = insert_intake_unit(
            &mut tx,
            u,
            line_item_id,
            AcquisitionMethod::OpeningBalance,
            elig,
            reason,
        )
        .await?;
        unit_ids.push(uid);
    }
    tx.commit().await?;

    Ok((
        StatusCode::CREATED,
        Json(IntakeResult {
            trade_in_id: None,
            purchase_id: Some(purchase_id),
            unit_ids,
        }),
    ))
}
