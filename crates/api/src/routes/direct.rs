//! cec.direct seam (scope §19): the two operations a build platform calls — read
//! availability, and reserve/consume parts as a build pulls them, attaching units to a
//! System whose `build_id` references the cec.direct build. Kept thin and event-logged.

use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::FromRow;
use uuid::Uuid;

use cec_inventory_domain::UnitEventType;

use crate::error::{ApiError, ApiResult};
use crate::events::log_unit_event;
use crate::AppState;

#[derive(Serialize, FromRow)]
pub struct ProductAvailability {
    pub product_id: Uuid,
    pub model: Option<String>,
    pub in_stock: i64,
}

#[derive(Serialize, FromRow)]
pub struct BulkAvailability {
    pub product_id: Uuid,
    pub quantity_on_hand: i64,
}

#[derive(Serialize)]
pub struct Availability {
    pub serialized: Vec<ProductAvailability>,
    pub bulk: Vec<BulkAvailability>,
}

/// Availability read: `in_stock` serialized units per product plus bulk quantity-on-hand.
pub async fn availability(State(s): State<AppState>) -> ApiResult<Json<Availability>> {
    let serialized = sqlx::query_as::<_, ProductAvailability>(
        "SELECT u.product_id, p.model, count(*) AS in_stock \
         FROM inventory_unit u LEFT JOIN product p ON p.id = u.product_id \
         WHERE u.status = 'in_stock' AND u.product_id IS NOT NULL \
         GROUP BY u.product_id, p.model ORDER BY p.model",
    )
    .fetch_all(&s.db)
    .await?;
    let bulk = sqlx::query_as::<_, BulkAvailability>(
        "SELECT product_id, COALESCE(sum(quantity_on_hand),0)::bigint AS quantity_on_hand \
         FROM stock_item GROUP BY product_id",
    )
    .fetch_all(&s.db)
    .await?;
    Ok(Json(Availability { serialized, bulk }))
}

/// Transition a unit's status with a guard on the allowed predecessor, log the event, and
/// (optionally) attach it to a system. Shared by reserve and consume.
async fn transition(
    s: &AppState,
    unit_id: Uuid,
    allowed_from: &[&str],
    to: &str,
    event: UnitEventType,
    system_id: Option<Uuid>,
    actor: Option<&str>,
) -> ApiResult<()> {
    let current: Option<String> =
        sqlx::query_scalar("SELECT status::text FROM inventory_unit WHERE id = $1 FOR UPDATE")
            .bind(unit_id)
            .fetch_optional(&s.db)
            .await?;
    let current = current.ok_or_else(|| ApiError::NotFound("unit not found".into()))?;
    if !allowed_from.contains(&current.as_str()) {
        return Err(ApiError::BadRequest(format!(
            "unit is '{current}'; expected one of {allowed_from:?} for this transition"
        )));
    }

    let mut tx = s.db.begin().await?;
    sqlx::query("UPDATE inventory_unit SET status = $2::unit_status, system_id = COALESCE($3, system_id) WHERE id = $1")
        .bind(unit_id)
        .bind(to)
        .bind(system_id)
        .execute(&mut *tx)
        .await?;
    // Attaching a unit to a system is a membership change (scope §6.4).
    if let Some(sys) = system_id {
        sqlx::query("UPDATE system SET validation_state = 'invalidated' WHERE id = $1")
            .bind(sys)
            .execute(&mut *tx)
            .await?;
    }
    log_unit_event(
        &mut *tx,
        unit_id,
        event,
        Some(&current),
        Some(to),
        actor,
        system_id,
        None,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

#[derive(Deserialize, Default)]
pub struct ReserveReq {
    #[serde(default)]
    pub actor: Option<String>,
}

pub async fn reserve_unit(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    body: Option<Json<ReserveReq>>,
) -> ApiResult<Json<serde_json::Value>> {
    let b = body.map(|j| j.0).unwrap_or_default();
    transition(
        &s,
        id,
        &["in_stock"],
        "reserved",
        UnitEventType::Reserve,
        None,
        b.actor.as_deref(),
    )
    .await?;
    Ok(Json(json!({ "unit_id": id, "status": "reserved" })))
}

#[derive(Deserialize)]
pub struct ConsumeReq {
    /// The System (build) the unit is consumed into; its `build_id` references cec.direct.
    pub system_id: Uuid,
    #[serde(default)]
    pub actor: Option<String>,
}

/// Consume a reserved/in-stock unit into a build: status → installed, attach to the system.
pub async fn consume_unit(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(b): Json<ConsumeReq>,
) -> ApiResult<Json<serde_json::Value>> {
    // Confirm the target system exists for a clean 404.
    let exists: Option<Uuid> = sqlx::query_scalar("SELECT id FROM system WHERE id = $1")
        .bind(b.system_id)
        .fetch_optional(&s.db)
        .await?;
    if exists.is_none() {
        return Err(ApiError::NotFound("system not found".into()));
    }
    transition(
        &s,
        id,
        &["in_stock", "reserved"],
        "installed",
        UnitEventType::Install,
        Some(b.system_id),
        b.actor.as_deref(),
    )
    .await?;
    Ok(Json(
        json!({ "unit_id": id, "status": "installed", "system_id": b.system_id }),
    ))
}
