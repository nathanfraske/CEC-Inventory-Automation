//! Purchases (the receipt/order/intake), their line items (the lots), and receipt-file
//! upload to the object store. Money is `numeric(12,2)` → `rust_decimal::Decimal`, sent
//! and received as JSON strings for exactness.

use axum::extract::{Multipart, Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{FromRow, PgExecutor};
use uuid::Uuid;

use cec_inventory_domain::{ResolutionStatus, SourceType};

use crate::error::{ApiError, ApiResult};
use crate::AppState;

const PURCHASE_COLS: &str = "id, vendor_id, purchase_datetime, order_number, invoice_number, \
    currency, subtotal, tax, shipping, discount_total, total, payment_method, source_type, \
    receipt_files, extract_confidence, created_by, created_at";

const LINE_COLS: &str = "id, purchase_id, product_id, description_as_printed, vendor_sku, \
    quantity, unit_price, line_total, currency, is_bundle, parent_line_id, \
    allocated_landed_cost, resolution_status";

#[derive(Serialize, FromRow)]
pub struct Purchase {
    pub id: Uuid,
    pub vendor_id: Option<Uuid>,
    pub purchase_datetime: Option<DateTime<Utc>>,
    pub order_number: Option<String>,
    pub invoice_number: Option<String>,
    pub currency: Option<String>,
    pub subtotal: Option<Decimal>,
    pub tax: Option<Decimal>,
    pub shipping: Option<Decimal>,
    pub discount_total: Option<Decimal>,
    pub total: Option<Decimal>,
    pub payment_method: Option<String>,
    pub source_type: SourceType,
    pub receipt_files: serde_json::Value,
    pub extract_confidence: Option<Decimal>,
    pub created_by: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Serialize, FromRow)]
pub struct LineItem {
    pub id: Uuid,
    pub purchase_id: Uuid,
    pub product_id: Option<Uuid>,
    pub description_as_printed: Option<String>,
    pub vendor_sku: Option<String>,
    pub quantity: i32,
    pub unit_price: Option<Decimal>,
    pub line_total: Option<Decimal>,
    pub currency: Option<String>,
    pub is_bundle: bool,
    pub parent_line_id: Option<Uuid>,
    pub allocated_landed_cost: Option<Decimal>,
    pub resolution_status: ResolutionStatus,
}

#[derive(Serialize)]
pub struct PurchaseWithItems {
    #[serde(flatten)]
    pub purchase: Purchase,
    pub line_items: Vec<LineItem>,
}

#[derive(Deserialize)]
pub struct CreateLineItem {
    #[serde(default)]
    pub product_id: Option<Uuid>,
    #[serde(default)]
    pub description_as_printed: Option<String>,
    #[serde(default)]
    pub vendor_sku: Option<String>,
    #[serde(default = "default_qty")]
    pub quantity: i32,
    #[serde(default)]
    pub unit_price: Option<Decimal>,
    #[serde(default)]
    pub line_total: Option<Decimal>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub is_bundle: bool,
}

fn default_qty() -> i32 {
    1
}

#[derive(Deserialize)]
pub struct CreatePurchase {
    #[serde(default)]
    pub vendor_id: Option<Uuid>,
    #[serde(default)]
    pub purchase_datetime: Option<DateTime<Utc>>,
    #[serde(default)]
    pub order_number: Option<String>,
    #[serde(default)]
    pub invoice_number: Option<String>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub subtotal: Option<Decimal>,
    #[serde(default)]
    pub tax: Option<Decimal>,
    #[serde(default)]
    pub shipping: Option<Decimal>,
    #[serde(default)]
    pub discount_total: Option<Decimal>,
    #[serde(default)]
    pub total: Option<Decimal>,
    #[serde(default)]
    pub payment_method: Option<String>,
    #[serde(default = "default_source")]
    pub source_type: SourceType,
    #[serde(default)]
    pub created_by: Option<String>,
    #[serde(default)]
    pub line_items: Vec<CreateLineItem>,
}

fn default_source() -> SourceType {
    SourceType::Manual
}

async fn insert_line_item<'e, E>(
    exec: E,
    purchase_id: Uuid,
    item: CreateLineItem,
) -> Result<LineItem, sqlx::Error>
where
    E: PgExecutor<'e>,
{
    let sql = format!(
        "INSERT INTO purchase_line_item \
         (purchase_id, product_id, description_as_printed, vendor_sku, quantity, unit_price, line_total, currency, is_bundle) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9) RETURNING {LINE_COLS}"
    );
    sqlx::query_as::<_, LineItem>(&sql)
        .bind(purchase_id)
        .bind(item.product_id)
        .bind(item.description_as_printed)
        .bind(item.vendor_sku)
        .bind(item.quantity)
        .bind(item.unit_price)
        .bind(item.line_total)
        .bind(item.currency.unwrap_or_else(|| "USD".into()))
        .bind(item.is_bundle)
        .fetch_one(exec)
        .await
}

pub async fn create_purchase(
    State(s): State<AppState>,
    Json(body): Json<CreatePurchase>,
) -> ApiResult<(StatusCode, Json<PurchaseWithItems>)> {
    let mut tx = s.db.begin().await?;

    let insert_sql = format!(
        "INSERT INTO purchase \
         (vendor_id, purchase_datetime, order_number, invoice_number, currency, subtotal, tax, shipping, discount_total, total, payment_method, source_type, created_by) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13) RETURNING {PURCHASE_COLS}"
    );
    let purchase = sqlx::query_as::<_, Purchase>(&insert_sql)
        .bind(body.vendor_id)
        .bind(body.purchase_datetime)
        .bind(body.order_number)
        .bind(body.invoice_number)
        .bind(body.currency.unwrap_or_else(|| "USD".into()))
        .bind(body.subtotal)
        .bind(body.tax)
        .bind(body.shipping)
        .bind(body.discount_total)
        .bind(body.total)
        .bind(body.payment_method)
        .bind(body.source_type)
        .bind(body.created_by)
        .fetch_one(&mut *tx)
        .await?;

    let mut line_items = Vec::with_capacity(body.line_items.len());
    for item in body.line_items {
        line_items.push(insert_line_item(&mut *tx, purchase.id, item).await?);
    }

    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(PurchaseWithItems {
            purchase,
            line_items,
        }),
    ))
}

pub async fn list_purchases(State(s): State<AppState>) -> ApiResult<Json<Vec<Purchase>>> {
    let sql = format!("SELECT {PURCHASE_COLS} FROM purchase ORDER BY created_at DESC");
    let rows = sqlx::query_as::<_, Purchase>(&sql).fetch_all(&s.db).await?;
    Ok(Json(rows))
}

pub async fn get_purchase(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<PurchaseWithItems>> {
    let sql = format!("SELECT {PURCHASE_COLS} FROM purchase WHERE id = $1");
    let purchase = sqlx::query_as::<_, Purchase>(&sql)
        .bind(id)
        .fetch_optional(&s.db)
        .await?
        .ok_or_else(|| ApiError::NotFound("purchase not found".into()))?;

    let lsql =
        format!("SELECT {LINE_COLS} FROM purchase_line_item WHERE purchase_id = $1 ORDER BY id");
    let line_items = sqlx::query_as::<_, LineItem>(&lsql)
        .bind(id)
        .fetch_all(&s.db)
        .await?;

    Ok(Json(PurchaseWithItems {
        purchase,
        line_items,
    }))
}

pub async fn add_line_item(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(item): Json<CreateLineItem>,
) -> ApiResult<(StatusCode, Json<LineItem>)> {
    // Confirm the purchase exists for a clean 404 (instead of a raw FK error).
    let exists: Option<Uuid> = sqlx::query_scalar("SELECT id FROM purchase WHERE id = $1")
        .bind(id)
        .fetch_optional(&s.db)
        .await?;
    if exists.is_none() {
        return Err(ApiError::NotFound("purchase not found".into()));
    }
    let li = insert_line_item(&s.db, id, item).await?;
    Ok((StatusCode::CREATED, Json(li)))
}

/// Upload a receipt file (multipart field `file`) into the object store and append a
/// reference to the purchase's `receipt_files` (scope Sections 10.5 and 17).
pub async fn upload_receipt(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    mut multipart: Multipart,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let exists: Option<Uuid> = sqlx::query_scalar("SELECT id FROM purchase WHERE id = $1")
        .bind(id)
        .fetch_optional(&s.db)
        .await?;
    if exists.is_none() {
        return Err(ApiError::NotFound("purchase not found".into()));
    }

    // Take the first field that carries a filename.
    let mut found: Option<(String, Option<String>, Vec<u8>)> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("malformed multipart: {e}")))?
    {
        if let Some(fname) = field.file_name().map(|s| s.to_string()) {
            let content_type = field.content_type().map(|s| s.to_string());
            let bytes = field
                .bytes()
                .await
                .map_err(|e| ApiError::BadRequest(format!("could not read upload: {e}")))?;
            found = Some((fname, content_type, bytes.to_vec()));
            break;
        }
    }

    let (filename, content_type, bytes) =
        found.ok_or_else(|| ApiError::BadRequest("no file field in upload".into()))?;
    if bytes.is_empty() {
        return Err(ApiError::BadRequest("uploaded file is empty".into()));
    }

    let safe = sanitize_filename(&filename);
    let stored_name = format!("{}_{safe}", Uuid::new_v4());
    let rel = format!("receipts/{id}/{stored_name}");
    let abs = s.storage_root.join(&rel);
    if let Some(parent) = abs.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
    }
    tokio::fs::write(&abs, &bytes)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    let entry = json!({
        "ref": rel,
        "filename": filename,
        "content_type": content_type,
        "bytes": bytes.len(),
        "uploaded_at": Utc::now(),
    });
    let receipt_files: serde_json::Value = sqlx::query_scalar(
        "UPDATE purchase SET receipt_files = receipt_files || $1::jsonb WHERE id = $2 RETURNING receipt_files",
    )
    .bind(json!([entry]))
    .bind(id)
    .fetch_one(&s.db)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({ "receipt_files": receipt_files })),
    ))
}

/// Keep only the basename and a safe character set; never let an upload escape the dir.
fn sanitize_filename(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('.').to_string();
    if trimmed.is_empty() {
        "file".to_string()
    } else {
        trimmed
    }
}
