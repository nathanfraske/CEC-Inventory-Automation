//! Bulk, non-serialized stock (cables, screws, paste, passives): quantity-on-hand, not
//! per-unit serials. Adjustments are guarded so on-hand never goes negative.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::AppState;

const STOCK_COLS: &str =
    "id, product_id, location_bin, asset_tag, quantity_on_hand, reorder_point, notes";

#[derive(Serialize, FromRow)]
pub struct StockItem {
    pub id: Uuid,
    pub product_id: Uuid,
    pub location_bin: Option<String>,
    pub asset_tag: Option<String>,
    pub quantity_on_hand: i32,
    pub reorder_point: Option<i32>,
    pub notes: Option<String>,
}

#[derive(Deserialize)]
pub struct CreateStock {
    pub product_id: Uuid,
    #[serde(default)]
    pub location_bin: Option<String>,
    #[serde(default)]
    pub asset_tag: Option<String>,
    #[serde(default)]
    pub quantity_on_hand: i32,
    #[serde(default)]
    pub reorder_point: Option<i32>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Deserialize)]
pub struct AdjustStock {
    /// Signed change applied to quantity_on_hand (e.g. +50 received, -3 consumed).
    pub delta: i32,
    #[serde(default)]
    pub note: Option<String>,
}

pub async fn create_stock(
    State(s): State<AppState>,
    Json(body): Json<CreateStock>,
) -> ApiResult<(StatusCode, Json<StockItem>)> {
    if body.quantity_on_hand < 0 {
        return Err(ApiError::BadRequest(
            "quantity_on_hand cannot be negative".into(),
        ));
    }
    let sql = format!(
        "INSERT INTO stock_item (product_id, location_bin, asset_tag, quantity_on_hand, reorder_point, notes) \
         VALUES ($1,$2,$3,$4,$5,$6) RETURNING {STOCK_COLS}"
    );
    let item = sqlx::query_as::<_, StockItem>(&sql)
        .bind(body.product_id)
        .bind(body.location_bin)
        .bind(body.asset_tag)
        .bind(body.quantity_on_hand)
        .bind(body.reorder_point)
        .bind(body.notes)
        .fetch_one(&s.db)
        .await?;
    Ok((StatusCode::CREATED, Json(item)))
}

pub async fn list_stock(State(s): State<AppState>) -> ApiResult<Json<Vec<StockItem>>> {
    let sql = format!("SELECT {STOCK_COLS} FROM stock_item ORDER BY id");
    let rows = sqlx::query_as::<_, StockItem>(&sql)
        .fetch_all(&s.db)
        .await?;
    Ok(Json(rows))
}

pub async fn adjust_stock(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<AdjustStock>,
) -> ApiResult<Json<StockItem>> {
    let mut tx = s.db.begin().await?;
    let current: Option<i32> =
        sqlx::query_scalar("SELECT quantity_on_hand FROM stock_item WHERE id = $1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    let current = current.ok_or_else(|| ApiError::NotFound("stock item not found".into()))?;

    let new_qty = current
        .checked_add(body.delta)
        .ok_or_else(|| ApiError::BadRequest("quantity overflow".into()))?;
    if new_qty < 0 {
        return Err(ApiError::BadRequest(format!(
            "adjustment would make on-hand negative ({current} + {} = {new_qty})",
            body.delta
        )));
    }

    let sql =
        format!("UPDATE stock_item SET quantity_on_hand = $1 WHERE id = $2 RETURNING {STOCK_COLS}");
    let item = sqlx::query_as::<_, StockItem>(&sql)
        .bind(new_qty)
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(item))
}
