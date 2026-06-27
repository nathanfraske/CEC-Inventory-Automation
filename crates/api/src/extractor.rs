//! Client seam to the Python extractor service (scope §11.3). The Rust backend POSTs
//! receipt text to `EXTRACTOR_URL/extract` and receives the §11.4 JSON, which Phase 1 maps
//! into draft `PurchaseLineItem`s for operator confirmation. The extractor runs on the
//! inference box, so this is a best-effort call: a 502 is returned when it is unreachable.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, NaiveDateTime, Utc};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{json, Value};
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

/// Extract a pasted receipt AND persist it as a draft purchase with unresolved line items
/// (scope §3: receipt → auto-populated line items, operator confirms). The line items come
/// back `resolution_status = unresolved` for the operator to map to products and scan into
/// units. The full extractor payload is stored on `purchase.raw_extract`.
pub async fn create_from_extraction(
    State(s): State<AppState>,
    Json(b): Json<FromExtractionReq>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let extraction = extract_text(&b.text, b.vendor_hint.as_deref()).await?;
    let confidence = extraction
        .get("field_confidence")
        .and_then(|c| c.get("total"))
        .and_then(|x| x.as_f64())
        .and_then(Decimal::from_f64_retain);

    let mut tx = s.db.begin().await?;
    let purchase_id: Uuid = sqlx::query_scalar(
        "INSERT INTO purchase \
         (vendor_id, purchase_datetime, order_number, invoice_number, currency, subtotal, tax, \
          shipping, total, source_type, raw_extract, extract_confidence, created_by) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13) RETURNING id",
    )
    .bind(b.vendor_id)
    .bind(parse_dt(&extraction))
    .bind(extraction.get("order_number").and_then(|x| x.as_str()))
    .bind(extraction.get("invoice_number").and_then(|x| x.as_str()))
    .bind(
        extraction
            .get("currency")
            .and_then(|x| x.as_str())
            .unwrap_or("USD"),
    )
    .bind(money(&extraction, "subtotal"))
    .bind(money(&extraction, "tax"))
    .bind(money(&extraction, "shipping"))
    .bind(money(&extraction, "total"))
    .bind(b.source_type)
    .bind(&extraction)
    .bind(confidence)
    .bind(b.created_by.as_deref())
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

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "purchase_id": purchase_id,
            "engine": extraction.get("engine"),
            "vendor": extraction.get("vendor"),
            "line_item_ids": line_item_ids,
            "line_item_count": line_item_ids.len(),
            "needs_resolution": true,
        })),
    ))
}
