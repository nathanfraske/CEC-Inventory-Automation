//! Client seam to the Python extractor service (scope §11.3). The Rust backend POSTs
//! receipt text to `EXTRACTOR_URL/extract` and receives the §11.4 JSON, which Phase 1 maps
//! into draft `PurchaseLineItem`s for operator confirmation. The extractor runs on the
//! inference box, so this is a best-effort call: a 502 is returned when it is unreachable.

use axum::extract::{Multipart, State};
use axum::http::StatusCode;
use axum::Json;
use base64::Engine;
use chrono::{DateTime, NaiveDateTime, Utc};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use cec_inventory_domain::SourceType;

use crate::error::{ApiError, ApiResult};
use crate::AppState;

fn extractor_url() -> String {
    std::env::var("EXTRACTOR_URL").unwrap_or_else(|_| "http://inference-box:8900".to_string())
}

/// POST text to the extractor and return its structured JSON.
pub async fn extract_text(text: &str, vendor_hint: Option<&str>) -> ApiResult<Value> {
    let url = format!("{}/extract", extractor_url().trim_end_matches('/'));
    let body = serde_json::json!({ "text": text, "vendor_hint": vendor_hint });
    let resp = reqwest::Client::new()
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| ApiError::Upstream(format!("extractor unreachable at {url}: {e}")))?;
    if !resp.status().is_success() {
        return Err(ApiError::Upstream(format!(
            "extractor returned {}",
            resp.status()
        )));
    }
    resp.json::<Value>()
        .await
        .map_err(|e| ApiError::Upstream(format!("extractor sent invalid JSON: {e}")))
}

/// POST a receipt **image** to the extractor's vision backend and return its structured JSON.
/// The image is base64-encoded into the JSON body; the extractor's backend (`stub` by default,
/// `claude` for the interim hosted-vision path) does the actual read (scope §11.2).
pub async fn extract_image(
    image: &[u8],
    media_type: &str,
    vendor_hint: Option<&str>,
) -> ApiResult<Value> {
    let url = format!("{}/extract-image", extractor_url().trim_end_matches('/'));
    let b64 = base64::engine::general_purpose::STANDARD.encode(image);
    let body = serde_json::json!({
        "image_base64": b64,
        "media_type": media_type,
        "vendor_hint": vendor_hint,
    });
    let resp = reqwest::Client::new()
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| ApiError::Upstream(format!("extractor unreachable at {url}: {e}")))?;
    if !resp.status().is_success() {
        return Err(ApiError::Upstream(format!(
            "extractor returned {}",
            resp.status()
        )));
    }
    resp.json::<Value>()
        .await
        .map_err(|e| ApiError::Upstream(format!("extractor sent invalid JSON: {e}")))
}

#[derive(Deserialize)]
pub struct ExtractPreviewReq {
    pub text: String,
    #[serde(default)]
    pub vendor_hint: Option<String>,
}

/// Preview extraction for a pasted receipt (does not persist). The operator confirms before
/// line items are created (scope §3).
pub async fn extract_preview(
    State(_s): State<AppState>,
    Json(b): Json<ExtractPreviewReq>,
) -> ApiResult<Json<Value>> {
    Ok(Json(extract_text(&b.text, b.vendor_hint.as_deref()).await?))
}

fn money(v: &Value, key: &str) -> Option<Decimal> {
    v.get(key)
        .and_then(|x| x.as_f64())
        .and_then(Decimal::from_f64_retain)
        .map(|d| d.round_dp(2))
}

fn parse_dt(v: &Value) -> Option<DateTime<Utc>> {
    let s = v.get("purchase_datetime")?.as_str()?;
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    for fmt in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M", "%Y-%m-%d"] {
        if let Ok(ndt) = NaiveDateTime::parse_from_str(s, fmt) {
            return Some(DateTime::from_naive_utc_and_offset(ndt, Utc));
        }
        if let Ok(d) = chrono::NaiveDate::parse_from_str(s, fmt) {
            return Some(DateTime::from_naive_utc_and_offset(
                d.and_hms_opt(0, 0, 0)?,
                Utc,
            ));
        }
    }
    None
}

#[derive(Deserialize)]
pub struct FromExtractionReq {
    pub text: String,
    #[serde(default)]
    pub vendor_hint: Option<String>,
    #[serde(default)]
    pub vendor_id: Option<Uuid>,
    #[serde(default)]
    pub created_by: Option<String>,
    #[serde(default = "default_source")]
    pub source_type: SourceType,
}

fn default_source() -> SourceType {
    SourceType::Manual
}

/// Persist an extractor payload (the §11.4 JSON, whatever produced it — template, image VLM,
/// or an external/operator vision pass) as a draft purchase with **unresolved** line items
/// (scope §3: receipt → auto-populated line items, operator confirms). Lines come back
/// `resolution_status = unresolved` for the operator to map to products and scan into units.
/// The full payload is stored on `purchase.raw_extract`. Shared by every `from-*` handler.
pub async fn persist_extraction(
    db: &PgPool,
    extraction: &Value,
    vendor_id: Option<Uuid>,
    created_by: Option<&str>,
    source_type: SourceType,
) -> ApiResult<Value> {
    let confidence = extraction
        .get("field_confidence")
        .and_then(|c| c.get("total"))
        .and_then(|x| x.as_f64())
        .and_then(Decimal::from_f64_retain);

    let mut tx = db.begin().await?;
    let purchase_id: Uuid = sqlx::query_scalar(
        "INSERT INTO purchase \
         (vendor_id, purchase_datetime, order_number, invoice_number, currency, subtotal, tax, \
          shipping, total, source_type, raw_extract, extract_confidence, created_by) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13) RETURNING id",
    )
    .bind(vendor_id)
    .bind(parse_dt(extraction))
    .bind(extraction.get("order_number").and_then(|x| x.as_str()))
    .bind(extraction.get("invoice_number").and_then(|x| x.as_str()))
    .bind(
        extraction
            .get("currency")
            .and_then(|x| x.as_str())
            .unwrap_or("USD"),
    )
    .bind(money(extraction, "subtotal"))
    .bind(money(extraction, "tax"))
    .bind(money(extraction, "shipping"))
    .bind(money(extraction, "total"))
    .bind(source_type)
    .bind(extraction)
    .bind(confidence)
    .bind(created_by)
    .fetch_one(&mut *tx)
    .await?;

    let empty = vec![];
    let lines = extraction
        .get("line_items")
        .and_then(|x| x.as_array())
        .unwrap_or(&empty);
    let mut line_item_ids = Vec::new();
    for li in lines {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO purchase_line_item \
             (purchase_id, description_as_printed, vendor_sku, quantity, unit_price, line_total, is_bundle) \
             VALUES ($1,$2,$3,$4,$5,$6,$7) RETURNING id",
        )
        .bind(purchase_id)
        .bind(li.get("description").and_then(|x| x.as_str()))
        .bind(li.get("vendor_sku").and_then(|x| x.as_str()))
        .bind(li.get("quantity").and_then(|x| x.as_i64()).unwrap_or(1) as i32)
        .bind(money(li, "unit_price"))
        .bind(money(li, "line_total"))
        .bind(li.get("is_bundle").and_then(|x| x.as_bool()).unwrap_or(false))
        .fetch_one(&mut *tx)
        .await?;
        line_item_ids.push(id);
    }
    tx.commit().await?;

    Ok(json!({
        "purchase_id": purchase_id,
        "engine": extraction.get("engine"),
        "vendor": extraction.get("vendor"),
        "line_item_ids": line_item_ids,
        "line_item_count": line_item_ids.len(),
        "needs_resolution": true,
    }))
}

/// Extract a pasted receipt's text AND persist it as a draft purchase (scope §3).
pub async fn create_from_extraction(
    State(s): State<AppState>,
    Json(b): Json<FromExtractionReq>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let extraction = extract_text(&b.text, b.vendor_hint.as_deref()).await?;
    let summary = persist_extraction(
        &s.db,
        &extraction,
        b.vendor_id,
        b.created_by.as_deref(),
        b.source_type,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(summary)))
}

/// Extract an uploaded receipt **image** via the vision backend AND persist it as a draft
/// purchase (scope §11.2). Multipart: the first file field is the image; optional text fields
/// `vendor_hint` / `created_by`. The source type is recorded as `physical_photo`.
pub async fn create_from_image(
    State(s): State<AppState>,
    mut multipart: Multipart,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let mut image: Option<(String, Vec<u8>)> = None; // (media_type, bytes)
    let mut vendor_hint: Option<String> = None;
    let mut created_by: Option<String> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("malformed multipart: {e}")))?
    {
        let name = field.name().map(|s| s.to_string());
        if field.file_name().is_some() {
            let media_type = field
                .content_type()
                .map(|s| s.to_string())
                .unwrap_or_else(|| "image/jpeg".to_string());
            let bytes = field
                .bytes()
                .await
                .map_err(|e| ApiError::BadRequest(format!("could not read image: {e}")))?;
            image = Some((media_type, bytes.to_vec()));
        } else {
            let text = field.text().await.unwrap_or_default();
            match name.as_deref() {
                Some("vendor_hint") if !text.is_empty() => vendor_hint = Some(text),
                Some("created_by") if !text.is_empty() => created_by = Some(text),
                _ => {}
            }
        }
    }
    let (media_type, bytes) =
        image.ok_or_else(|| ApiError::BadRequest("no image file field in upload".into()))?;
    if bytes.is_empty() {
        return Err(ApiError::BadRequest("uploaded image is empty".into()));
    }

    let extraction = extract_image(&bytes, &media_type, vendor_hint.as_deref()).await?;
    let summary = persist_extraction(
        &s.db,
        &extraction,
        None,
        created_by.as_deref(),
        SourceType::PhysicalPhoto,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(summary)))
}

#[derive(Deserialize)]
pub struct FromPayloadReq {
    /// A §11.4 extraction payload produced out-of-band (e.g. an operator/agent vision pass —
    /// the interim path while the local VLM is unavailable). Persisted as-is into a draft.
    pub extraction: Value,
    #[serde(default)]
    pub vendor_id: Option<Uuid>,
    #[serde(default)]
    pub created_by: Option<String>,
    #[serde(default = "default_source")]
    pub source_type: SourceType,
}

/// Persist a caller-supplied §11.4 extraction payload as a draft purchase (scope §3/§11). This
/// is the seam any external extractor — including a human or an in-the-loop vision pass — uses
/// to feed the receipt→inventory loop without the Python service.
pub async fn create_from_payload(
    State(s): State<AppState>,
    Json(b): Json<FromPayloadReq>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    if !b.extraction.is_object() {
        return Err(ApiError::BadRequest(
            "`extraction` must be a §11.4 JSON object".into(),
        ));
    }
    let summary = persist_extraction(
        &s.db,
        &b.extraction,
        b.vendor_id,
        b.created_by.as_deref(),
        b.source_type,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(summary)))
}
