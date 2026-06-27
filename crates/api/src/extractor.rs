//! Client seam to the Python extractor service (scope §11.3). The Rust backend POSTs
//! receipt text to `EXTRACTOR_URL/extract` and receives the §11.4 JSON, which Phase 1 maps
//! into draft `PurchaseLineItem`s for operator confirmation. The extractor runs on the
//! inference box, so this is a best-effort call: a 502 is returned when it is unreachable.

use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use serde_json::Value;

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
