//! Catalog + vendor reference data: the identities purchases and units hang off of.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::AppState;

// ---------------- vendors ----------------

#[derive(Serialize, FromRow)]
pub struct Vendor {
    pub id: Uuid,
    pub name: String,
    pub address: Option<String>,
    pub website: Option<String>,
    pub rma_url: Option<String>,
    pub rma_contact: Option<String>,
    pub account_number: Option<String>,
    pub notes: Option<String>,
}

#[derive(Deserialize)]
pub struct CreateVendor {
    pub name: String,
    #[serde(default)]
    pub address: Option<String>,
    #[serde(default)]
    pub website: Option<String>,
    #[serde(default)]
    pub rma_url: Option<String>,
    #[serde(default)]
    pub rma_contact: Option<String>,
    #[serde(default)]
    pub account_number: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

pub async fn create_vendor(
    State(s): State<AppState>,
    Json(body): Json<CreateVendor>,
) -> ApiResult<(StatusCode, Json<Vendor>)> {
    if body.name.trim().is_empty() {
        return Err(ApiError::BadRequest("name is required".into()));
    }
    let v = sqlx::query_as::<_, Vendor>(
        "INSERT INTO vendor (name, address, website, rma_url, rma_contact, account_number, notes) \
         VALUES ($1,$2,$3,$4,$5,$6,$7) RETURNING *",
    )
    .bind(body.name.trim())
    .bind(body.address)
    .bind(body.website)
    .bind(body.rma_url)
    .bind(body.rma_contact)
    .bind(body.account_number)
    .bind(body.notes)
    .fetch_one(&s.db)
    .await?;
    Ok((StatusCode::CREATED, Json(v)))
}

pub async fn list_vendors(State(s): State<AppState>) -> ApiResult<Json<Vec<Vendor>>> {
    let rows = sqlx::query_as::<_, Vendor>("SELECT * FROM vendor ORDER BY name")
        .fetch_all(&s.db)
        .await?;
    Ok(Json(rows))
}

pub async fn get_vendor(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vendor>> {
    let v = sqlx::query_as::<_, Vendor>("SELECT * FROM vendor WHERE id = $1")
        .bind(id)
        .fetch_optional(&s.db)
        .await?
        .ok_or_else(|| ApiError::NotFound("vendor not found".into()))?;
    Ok(Json(v))
}

// ---------------- manufacturers ----------------
// The two enum columns (warranty_basis_default, warranty_start_basis) use their DB
// defaults on insert and are read back cast to text, so no extra domain enums are needed.

#[derive(Serialize, FromRow)]
pub struct Manufacturer {
    pub id: Uuid,
    pub name: String,
    pub rma_url: Option<String>,
    pub rma_contact: Option<String>,
    pub warranty_policy_url: Option<String>,
    pub default_warranty_months: Option<i32>,
    pub replacement_warranty_days: Option<i32>,
    pub warranty_basis_default: Option<String>,
    pub warranty_transferable: Option<bool>,
    pub warranty_start_basis: Option<String>,
    pub notes: Option<String>,
}

const MANUFACTURER_COLS: &str = "id, name, rma_url, rma_contact, warranty_policy_url, \
    default_warranty_months, replacement_warranty_days, \
    warranty_basis_default::text AS warranty_basis_default, warranty_transferable, \
    warranty_start_basis::text AS warranty_start_basis, notes";

#[derive(Deserialize)]
pub struct CreateManufacturer {
    pub name: String,
    #[serde(default)]
    pub rma_url: Option<String>,
    #[serde(default)]
    pub rma_contact: Option<String>,
    #[serde(default)]
    pub warranty_policy_url: Option<String>,
    #[serde(default)]
    pub default_warranty_months: Option<i32>,
    #[serde(default)]
    pub replacement_warranty_days: Option<i32>,
    #[serde(default)]
    pub notes: Option<String>,
}

pub async fn create_manufacturer(
    State(s): State<AppState>,
    Json(body): Json<CreateManufacturer>,
) -> ApiResult<(StatusCode, Json<Manufacturer>)> {
    if body.name.trim().is_empty() {
        return Err(ApiError::BadRequest("name is required".into()));
    }
    let sql = format!(
        "INSERT INTO manufacturer \
         (name, rma_url, rma_contact, warranty_policy_url, default_warranty_months, replacement_warranty_days, notes) \
         VALUES ($1,$2,$3,$4,$5,$6,$7) RETURNING {MANUFACTURER_COLS}"
    );
    let m = sqlx::query_as::<_, Manufacturer>(&sql)
        .bind(body.name.trim())
        .bind(body.rma_url)
        .bind(body.rma_contact)
        .bind(body.warranty_policy_url)
        .bind(body.default_warranty_months)
        .bind(body.replacement_warranty_days)
        .bind(body.notes)
        .fetch_one(&s.db)
        .await?;
    Ok((StatusCode::CREATED, Json(m)))
}

pub async fn list_manufacturers(State(s): State<AppState>) -> ApiResult<Json<Vec<Manufacturer>>> {
    let sql = format!("SELECT {MANUFACTURER_COLS} FROM manufacturer ORDER BY name");
    let rows = sqlx::query_as::<_, Manufacturer>(&sql)
        .fetch_all(&s.db)
        .await?;
    Ok(Json(rows))
}

// ---------------- products ----------------

#[derive(Serialize, FromRow)]
pub struct Product {
    pub id: Uuid,
    pub manufacturer_id: Option<Uuid>,
    pub model: String,
    pub mpn: Option<String>,
    pub upc_ean: Option<String>,
    pub category: Option<String>,
    pub serialized: bool,
    pub default_warranty_months: Option<i32>,
    pub serial_format_regex: Option<String>,
    pub datasheet_url: Option<String>,
    pub notes: Option<String>,
}

#[derive(Deserialize)]
pub struct CreateProduct {
    pub model: String,
    #[serde(default)]
    pub manufacturer_id: Option<Uuid>,
    #[serde(default)]
    pub mpn: Option<String>,
    #[serde(default)]
    pub upc_ean: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default = "default_true")]
    pub serialized: bool,
    #[serde(default)]
    pub default_warranty_months: Option<i32>,
    #[serde(default)]
    pub serial_format_regex: Option<String>,
    #[serde(default)]
    pub datasheet_url: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

fn default_true() -> bool {
    true
}

pub async fn create_product(
    State(s): State<AppState>,
    Json(body): Json<CreateProduct>,
) -> ApiResult<(StatusCode, Json<Product>)> {
    if body.model.trim().is_empty() {
        return Err(ApiError::BadRequest("model is required".into()));
    }
    let p = sqlx::query_as::<_, Product>(
        "INSERT INTO product \
         (manufacturer_id, model, mpn, upc_ean, category, serialized, default_warranty_months, serial_format_regex, datasheet_url, notes) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) RETURNING *",
    )
    .bind(body.manufacturer_id)
    .bind(body.model.trim())
    .bind(body.mpn)
    .bind(body.upc_ean)
    .bind(body.category)
    .bind(body.serialized)
    .bind(body.default_warranty_months)
    .bind(body.serial_format_regex)
    .bind(body.datasheet_url)
    .bind(body.notes)
    .fetch_one(&s.db)
    .await?;
    Ok((StatusCode::CREATED, Json(p)))
}

pub async fn list_products(State(s): State<AppState>) -> ApiResult<Json<Vec<Product>>> {
    let rows = sqlx::query_as::<_, Product>("SELECT * FROM product ORDER BY model")
        .fetch_all(&s.db)
        .await?;
    Ok(Json(rows))
}

pub async fn get_product(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Product>> {
    let p = sqlx::query_as::<_, Product>("SELECT * FROM product WHERE id = $1")
        .bind(id)
        .fetch_optional(&s.db)
        .await?
        .ok_or_else(|| ApiError::NotFound("product not found".into()))?;
    Ok(Json(p))
}
