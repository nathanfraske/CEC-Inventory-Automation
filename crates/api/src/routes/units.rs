//! Serialized inventory units. Every mutation writes a `unit_event` (scope Section 16):
//! creation logs `intake`, a status change logs `status_change`. The unit's event
//! timeline is the integrity backbone for RMA and transfer disputes.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use cec_inventory_domain::{
    AcquisitionMethod, CecWarrantyClass, ConditionKind, OwnerKind, SerialSource, UnitEventType,
    UnitStatus,
};

use crate::error::{ApiError, ApiResult};
use crate::events::log_unit_event;
use crate::AppState;

const UNIT_COLS: &str = "id, product_id, line_item_id, system_id, owner, customer_ref, \
    serial_number, serial_source, verified, asset_tag, condition, acquisition_method, status, \
    location_bin, unit_cost, mfr_warranty_expires, cec_warranty_class, cec_warranty_start, \
    cec_warranty_expires, registered, rma_eligible, rma_block_reason, notes, intake_at, intake_by";

#[derive(Serialize, FromRow)]
pub struct Unit {
    pub id: Uuid,
    pub product_id: Option<Uuid>,
    pub line_item_id: Option<Uuid>,
    pub system_id: Option<Uuid>,
    pub owner: OwnerKind,
    pub customer_ref: Option<String>,
    pub serial_number: Option<String>,
    pub serial_source: Option<SerialSource>,
    pub verified: bool,
    pub asset_tag: Option<String>,
    pub condition: ConditionKind,
    pub acquisition_method: AcquisitionMethod,
    pub status: UnitStatus,
    pub location_bin: Option<String>,
    pub unit_cost: Option<Decimal>,
    pub mfr_warranty_expires: Option<chrono::NaiveDate>,
    pub cec_warranty_class: Option<CecWarrantyClass>,
    pub cec_warranty_start: Option<DateTime<Utc>>,
    pub cec_warranty_expires: Option<chrono::NaiveDate>,
    pub registered: bool,
    pub rma_eligible: Option<bool>,
    pub rma_block_reason: Option<String>,
    pub notes: Option<String>,
    pub intake_at: DateTime<Utc>,
    pub intake_by: Option<String>,
}

#[derive(Serialize, FromRow)]
pub struct UnitEvent {
    pub id: Uuid,
    pub unit_id: Uuid,
    pub event_type: UnitEventType,
    pub from_value: Option<String>,
    pub to_value: Option<String>,
    pub actor: Option<String>,
    pub at: DateTime<Utc>,
    pub system_id: Option<Uuid>,
    pub rma_case_id: Option<Uuid>,
    pub detail: Option<serde_json::Value>,
}

#[derive(Deserialize)]
pub struct CreateUnit {
    pub product_id: Uuid,
    #[serde(default)]
    pub line_item_id: Option<Uuid>,
    #[serde(default)]
    pub system_id: Option<Uuid>,
    #[serde(default = "default_owner")]
    pub owner: OwnerKind,
    #[serde(default)]
    pub customer_ref: Option<String>,
    #[serde(default)]
    pub serial_number: Option<String>,
    #[serde(default)]
    pub serial_source: Option<SerialSource>,
    #[serde(default)]
    pub asset_tag: Option<String>,
    #[serde(default = "default_condition")]
    pub condition: ConditionKind,
    #[serde(default = "default_acq")]
    pub acquisition_method: AcquisitionMethod,
    #[serde(default = "default_status")]
    pub status: UnitStatus,
    #[serde(default)]
    pub location_bin: Option<String>,
    #[serde(default)]
    pub unit_cost: Option<Decimal>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub intake_by: Option<String>,
}

fn default_owner() -> OwnerKind {
    OwnerKind::Shop
}
fn default_condition() -> ConditionKind {
    ConditionKind::New
}
fn default_acq() -> AcquisitionMethod {
    AcquisitionMethod::Purchase
}
fn default_status() -> UnitStatus {
    UnitStatus::InStock
}

#[derive(Deserialize)]
pub struct ChangeStatus {
    pub status: UnitStatus,
    #[serde(default)]
    pub actor: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

pub async fn create_unit(
    State(s): State<AppState>,
    Json(body): Json<CreateUnit>,
) -> ApiResult<(StatusCode, Json<Unit>)> {
    let mut tx = s.db.begin().await?;

    let sql = format!(
        "INSERT INTO inventory_unit \
         (product_id, line_item_id, system_id, owner, customer_ref, serial_number, serial_source, \
          asset_tag, condition, acquisition_method, status, location_bin, unit_cost, notes, intake_by) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15) RETURNING {UNIT_COLS}"
    );
    let unit = sqlx::query_as::<_, Unit>(&sql)
        .bind(body.product_id)
        .bind(body.line_item_id)
        .bind(body.system_id)
        .bind(body.owner)
        .bind(body.customer_ref)
        .bind(body.serial_number)
        .bind(body.serial_source)
        .bind(body.asset_tag)
        .bind(body.condition)
        .bind(body.acquisition_method)
        .bind(body.status)
        .bind(body.location_bin)
        .bind(body.unit_cost)
        .bind(body.notes)
        .bind(body.intake_by)
        .fetch_one(&mut *tx)
        .await?;

    let to_status = enum_to_str(&unit.status);
    let detail = serde_json::json!({
        "serial_number": unit.serial_number,
        "asset_tag": unit.asset_tag,
        "condition": enum_to_str(&unit.condition),
        "acquisition_method": enum_to_str(&unit.acquisition_method),
    });
    log_unit_event(
        &mut *tx,
        unit.id,
        UnitEventType::Intake,
        None,
        to_status.as_deref(),
        unit.intake_by.as_deref(),
        unit.system_id,
        Some(detail),
    )
    .await?;

    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(unit)))
}

pub async fn list_units(State(s): State<AppState>) -> ApiResult<Json<Vec<Unit>>> {
    let sql = format!("SELECT {UNIT_COLS} FROM inventory_unit ORDER BY intake_at DESC");
    let rows = sqlx::query_as::<_, Unit>(&sql).fetch_all(&s.db).await?;
    Ok(Json(rows))
}

pub async fn get_unit(State(s): State<AppState>, Path(id): Path<Uuid>) -> ApiResult<Json<Unit>> {
    let sql = format!("SELECT {UNIT_COLS} FROM inventory_unit WHERE id = $1");
    let unit = sqlx::query_as::<_, Unit>(&sql)
        .bind(id)
        .fetch_optional(&s.db)
        .await?
        .ok_or_else(|| ApiError::NotFound("unit not found".into()))?;
    Ok(Json(unit))
}

pub async fn change_status(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<ChangeStatus>,
) -> ApiResult<Json<Unit>> {
    let mut tx = s.db.begin().await?;

    let current: Option<String> =
        sqlx::query_scalar("SELECT status::text FROM inventory_unit WHERE id = $1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    let current = current.ok_or_else(|| ApiError::NotFound("unit not found".into()))?;

    let sql = format!("UPDATE inventory_unit SET status = $1 WHERE id = $2 RETURNING {UNIT_COLS}");
    let unit = sqlx::query_as::<_, Unit>(&sql)
        .bind(body.status)
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;

    let to_status = enum_to_str(&unit.status);
    let detail = body.note.map(|n| serde_json::json!({ "note": n }));
    log_unit_event(
        &mut *tx,
        unit.id,
        UnitEventType::StatusChange,
        Some(current.as_str()),
        to_status.as_deref(),
        body.actor.as_deref(),
        unit.system_id,
        detail,
    )
    .await?;

    tx.commit().await?;
    Ok(Json(unit))
}

pub async fn list_events(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<UnitEvent>>> {
    let exists: Option<Uuid> = sqlx::query_scalar("SELECT id FROM inventory_unit WHERE id = $1")
        .bind(id)
        .fetch_optional(&s.db)
        .await?;
    if exists.is_none() {
        return Err(ApiError::NotFound("unit not found".into()));
    }
    let rows = sqlx::query_as::<_, UnitEvent>(
        "SELECT id, unit_id, event_type, from_value, to_value, actor, at, system_id, rma_case_id, detail \
         FROM unit_event WHERE unit_id = $1 ORDER BY at, id",
    )
    .bind(id)
    .fetch_all(&s.db)
    .await?;
    Ok(Json(rows))
}

/// Snake-case string for a unit-variant enum (e.g. `UnitStatus::InStock` -> "in_stock"),
/// used to record human-readable from/to values in the event log.
fn enum_to_str<T: Serialize>(v: &T) -> Option<String> {
    serde_json::to_value(v)
        .ok()
        .and_then(|x| x.as_str().map(|s| s.to_string()))
}
