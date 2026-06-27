//! Shipment capture and polling (scope Section 12). A purchase can have several shipments
//! (split shipment); each is polled for carrier status until delivered, with every update
//! written to `shipment_event`.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::FromRow;
use uuid::Uuid;

use cec_inventory_domain::{CarrierKind, PollState, ShipmentStatus};
use cec_inventory_tracking::{poll_shipment, provider_from_env};

use crate::error::{ApiError, ApiResult};
use crate::AppState;

const SHIPMENT_COLS: &str = "id, purchase_id, carrier, tracking_number, tracking_url, status, \
    expected_delivery_date, shipped_at, delivered_at, last_polled_at, poll_state, \
    line_item_ids, notes";

const EVENT_COLS: &str =
    "id, shipment_id, event_status, carrier_description, location, occurred_at, polled_at, raw";

#[derive(Serialize, FromRow)]
pub struct Shipment {
    pub id: Uuid,
    pub purchase_id: Uuid,
    pub carrier: Option<CarrierKind>,
    pub tracking_number: Option<String>,
    pub tracking_url: Option<String>,
    pub status: ShipmentStatus,
    pub expected_delivery_date: Option<NaiveDate>,
    pub shipped_at: Option<DateTime<Utc>>,
    pub delivered_at: Option<DateTime<Utc>>,
    pub last_polled_at: Option<DateTime<Utc>>,
    pub poll_state: PollState,
    pub line_item_ids: Option<Vec<Uuid>>,
    pub notes: Option<String>,
}

#[derive(Serialize, FromRow)]
pub struct ShipmentEvent {
    pub id: Uuid,
    pub shipment_id: Uuid,
    pub event_status: ShipmentStatus,
    pub carrier_description: Option<String>,
    pub location: Option<String>,
    pub occurred_at: Option<DateTime<Utc>>,
    pub polled_at: DateTime<Utc>,
    pub raw: Option<serde_json::Value>,
}

#[derive(Serialize)]
pub struct ShipmentWithEvents {
    #[serde(flatten)]
    pub shipment: Shipment,
    pub events: Vec<ShipmentEvent>,
}

#[derive(Deserialize)]
pub struct CreateShipment {
    #[serde(default)]
    pub carrier: Option<CarrierKind>,
    #[serde(default)]
    pub tracking_number: Option<String>,
    #[serde(default)]
    pub tracking_url: Option<String>,
    #[serde(default)]
    pub expected_delivery_date: Option<NaiveDate>,
    #[serde(default)]
    pub line_item_ids: Option<Vec<Uuid>>,
    #[serde(default)]
    pub notes: Option<String>,
}

pub async fn create_shipment(
    State(s): State<AppState>,
    Path(purchase_id): Path<Uuid>,
    Json(body): Json<CreateShipment>,
) -> ApiResult<(StatusCode, Json<Shipment>)> {
    let exists: Option<Uuid> = sqlx::query_scalar("SELECT id FROM purchase WHERE id = $1")
        .bind(purchase_id)
        .fetch_optional(&s.db)
        .await?;
    if exists.is_none() {
        return Err(ApiError::NotFound("purchase not found".into()));
    }
    let sql = format!(
        "INSERT INTO shipment \
         (purchase_id, carrier, tracking_number, tracking_url, expected_delivery_date, line_item_ids, notes) \
         VALUES ($1,$2,$3,$4,$5,$6,$7) RETURNING {SHIPMENT_COLS}"
    );
    let shipment = sqlx::query_as::<_, Shipment>(&sql)
        .bind(purchase_id)
        .bind(body.carrier)
        .bind(body.tracking_number)
        .bind(body.tracking_url)
        .bind(body.expected_delivery_date)
        .bind(body.line_item_ids)
        .bind(body.notes)
        .fetch_one(&s.db)
        .await?;
    Ok((StatusCode::CREATED, Json(shipment)))
}

#[derive(Deserialize)]
pub struct ListParams {
    #[serde(default)]
    pub active: bool,
}

pub async fn list_shipments(
    State(s): State<AppState>,
    Query(params): Query<ListParams>,
) -> ApiResult<Json<Vec<Shipment>>> {
    let sql = if params.active {
        format!("SELECT {SHIPMENT_COLS} FROM shipment WHERE poll_state = 'active' ORDER BY id")
    } else {
        format!("SELECT {SHIPMENT_COLS} FROM shipment ORDER BY id")
    };
    let rows = sqlx::query_as::<_, Shipment>(&sql).fetch_all(&s.db).await?;
    Ok(Json(rows))
}

pub async fn get_shipment(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<ShipmentWithEvents>> {
    let sql = format!("SELECT {SHIPMENT_COLS} FROM shipment WHERE id = $1");
    let shipment = sqlx::query_as::<_, Shipment>(&sql)
        .bind(id)
        .fetch_optional(&s.db)
        .await?
        .ok_or_else(|| ApiError::NotFound("shipment not found".into()))?;

    let esql = format!(
        "SELECT {EVENT_COLS} FROM shipment_event WHERE shipment_id = $1 ORDER BY occurred_at, polled_at"
    );
    let events = sqlx::query_as::<_, ShipmentEvent>(&esql)
        .bind(id)
        .fetch_all(&s.db)
        .await?;

    Ok(Json(ShipmentWithEvents { shipment, events }))
}

/// Run one poll tick against the configured carrier provider (`CARRIER_PROVIDER`).
pub async fn poll_now(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    let exists: Option<Uuid> = sqlx::query_scalar("SELECT id FROM shipment WHERE id = $1")
        .bind(id)
        .fetch_optional(&s.db)
        .await?;
    if exists.is_none() {
        return Err(ApiError::NotFound("shipment not found".into()));
    }

    let provider = provider_from_env();
    let outcome = poll_shipment(&s.db, provider.as_ref(), id)
        .await
        .map_err(ApiError::Internal)?;

    let sql = format!("SELECT {SHIPMENT_COLS} FROM shipment WHERE id = $1");
    let shipment = sqlx::query_as::<_, Shipment>(&sql)
        .bind(id)
        .fetch_one(&s.db)
        .await?;

    Ok(Json(
        json!({ "provider": provider.name(), "outcome": outcome, "shipment": shipment }),
    ))
}
