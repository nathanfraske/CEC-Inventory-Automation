//! TrackingMore carrier provider (scope §12.3 / INV-OQ-30, D-025). A multi-carrier tracking
//! aggregator: one API key, carrier auto-detect, polling — which is exactly what the poll engine
//! does. Chosen for its free tier (covers the shop's volume) over the now-paid USPS direct API.
//!
//! API v4 (`https://api.trackingmore.com/v4`): auth via the `Tracking-Api-Key` header. We GET an
//! existing tracking first (cheap, no courier needed); if it isn't registered yet we POST
//! `/trackings/create` once (this consumes a tracking credit), detecting the courier via
//! `/couriers/detect` when the carrier is `Other`. Checkpoints live under
//! `data.origin_info.trackinfo[]` and `data.destination_info.trackinfo[]`.
//!
//! The HTTP surface is thin; the parse/normalize logic (the part that can be wrong) is in pure
//! functions unit-tested with a canned response, so we don't need the network or a credit to test.

use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::{json, Value};

use cec_inventory_domain::{CarrierKind, ShipmentStatus};

use crate::carrier::{CarrierProvider, CarrierUpdate};

const DEFAULT_BASE_URL: &str = "https://api.trackingmore.com/v4";

pub struct TrackingMoreProvider {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl TrackingMoreProvider {
    /// Build from env: `CARRIER_API_KEY` (required) + optional `TRACKINGMORE_BASE_URL`. Returns
    /// `None` when the key is absent so the caller can fall back to the no-op provider.
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("CARRIER_API_KEY")
            .ok()
            .filter(|k| !k.is_empty())?;
        let base_url = std::env::var("TRACKINGMORE_BASE_URL")
            .ok()
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .ok()?;
        Some(Self {
            api_key,
            base_url: base_url.trim_end_matches('/').to_string(),
            client,
        })
    }

    async fn get_tracking(&self, tn: &str, courier: Option<&str>) -> anyhow::Result<Value> {
        let mut query = vec![("tracking_numbers", tn.to_string())];
        if let Some(c) = courier {
            query.push(("courier_code", c.to_string()));
        }
        let resp = self
            .client
            .get(format!("{}/trackings/get", self.base_url))
            .query(&query)
            .header("Tracking-Api-Key", &self.api_key)
            .send()
            .await?;
        if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            anyhow::bail!("trackingmore rate limited (429); back off");
        }
        Ok(resp.json::<Value>().await.unwrap_or_else(|_| json!({})))
    }

    /// Free in v4 — used only when the carrier is `Other` and we must register the tracking.
    async fn detect_courier(&self, tn: &str) -> anyhow::Result<String> {
        let resp = self
            .client
            .post(format!("{}/couriers/detect", self.base_url))
            .header("Tracking-Api-Key", &self.api_key)
            .json(&json!({ "tracking_number": tn }))
            .send()
            .await?;
        let body = resp.json::<Value>().await?;
        body.get("data")
            .and_then(|d| d.as_array())
            .and_then(|a| a.first())
            .and_then(|c| c.get("courier_code"))
            .and_then(|x| x.as_str())
            .map(String::from)
            .ok_or_else(|| anyhow::anyhow!("trackingmore could not detect a courier for {tn}"))
    }

    async fn create_tracking(&self, tn: &str, courier: &str) -> anyhow::Result<Value> {
        let resp = self
            .client
            .post(format!("{}/trackings/create", self.base_url))
            .header("Tracking-Api-Key", &self.api_key)
            .json(&json!({ "tracking_number": tn, "courier_code": courier }))
            .send()
            .await?;
        let body = resp.json::<Value>().await?;
        let code = body
            .get("meta")
            .and_then(|m| m.get("code"))
            .and_then(|x| x.as_i64())
            .unwrap_or(0);
        if !(200..300).contains(&code) {
            let msg = body
                .get("meta")
                .and_then(|m| m.get("message"))
                .and_then(|x| x.as_str())
                .unwrap_or("unknown error");
            anyhow::bail!("trackingmore create failed ({code}): {msg}");
        }
        Ok(body)
    }
}

#[async_trait]
impl CarrierProvider for TrackingMoreProvider {
    async fn fetch(
        &self,
        carrier: CarrierKind,
        tracking_number: &str,
    ) -> anyhow::Result<Vec<CarrierUpdate>> {
        let code = courier_code(carrier);
        // GET first — cheap, and returns the full checkpoint history once the tracking exists.
        let got = self.get_tracking(tracking_number, code).await?;
        if let Some(tracking) = extract_tracking(&got) {
            return Ok(parse_tracking(tracking));
        }
        // Not registered yet → register once (needs a courier code; detect it for `Other`).
        let code = match code {
            Some(c) => c.to_string(),
            None => self.detect_courier(tracking_number).await?,
        };
        let created = self.create_tracking(tracking_number, &code).await?;
        Ok(extract_tracking(&created)
            .map(parse_tracking)
            .unwrap_or_default())
    }

    fn name(&self) -> &'static str {
        "trackingmore"
    }
}

/// Map our `CarrierKind` to TrackingMore's `courier_code`. `Other` → `None` (detect at runtime).
fn courier_code(c: CarrierKind) -> Option<&'static str> {
    match c {
        CarrierKind::Usps => Some("usps"),
        CarrierKind::Ups => Some("ups"),
        CarrierKind::Fedex => Some("fedex"),
        CarrierKind::Dhl => Some("dhl"),
        CarrierKind::Other => None,
    }
}

/// Map a TrackingMore status string (top-level `delivery_status` or per-checkpoint
/// `checkpoint_delivery_status`) to the normalized `ShipmentStatus`.
fn map_status(s: &str) -> ShipmentStatus {
    match s {
        "inforeceived" => ShipmentStatus::LabelCreated,
        "pending" => ShipmentStatus::PreTransit,
        "transit" => ShipmentStatus::InTransit,
        "pickup" => ShipmentStatus::OutForDelivery,
        "delivered" => ShipmentStatus::Delivered,
        "exception" | "undelivered" => ShipmentStatus::Exception,
        // "notfound" / "expired" / anything unrecognized.
        _ => ShipmentStatus::Unknown,
    }
}

/// TrackingMore timestamps come as `"2015-11-02 17:11:00"` (assume UTC) or ISO-8601.
fn parse_date(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    for fmt in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S"] {
        if let Ok(ndt) = NaiveDateTime::parse_from_str(s, fmt) {
            return Some(DateTime::from_naive_utc_and_offset(ndt, Utc));
        }
    }
    None
}

/// Pull the single tracking object out of a `/get` (array) or `/create` (object) response.
fn extract_tracking(resp: &Value) -> Option<&Value> {
    let data = resp.get("data")?;
    if let Some(arr) = data.as_array() {
        arr.first()
    } else if data.is_object() {
        Some(data)
    } else {
        None
    }
}

/// Normalize one tracking object's checkpoints (origin + destination legs) into `CarrierUpdate`s.
/// The poll engine dedups by `(status, occurred_at)`, so returning the full history is correct.
fn parse_tracking(tracking: &Value) -> Vec<CarrierUpdate> {
    let mut out = Vec::new();
    for leg in ["origin_info", "destination_info"] {
        let Some(events) = tracking
            .get(leg)
            .and_then(|i| i.get("trackinfo"))
            .and_then(|x| x.as_array())
        else {
            continue;
        };
        for cp in events {
            let Some(occurred_at) = cp.get("Date").and_then(|x| x.as_str()).and_then(parse_date)
            else {
                continue; // skip checkpoints without a parseable timestamp
            };
            let status = cp
                .get("checkpoint_delivery_status")
                .or_else(|| cp.get("checkpoint_status"))
                .and_then(|x| x.as_str())
                .map(map_status)
                .unwrap_or(ShipmentStatus::Unknown);
            out.push(CarrierUpdate {
                status,
                description: cp
                    .get("StatusDescription")
                    .and_then(|x| x.as_str())
                    .filter(|s| !s.is_empty())
                    .map(String::from),
                location: cp
                    .get("Details")
                    .and_then(|x| x.as_str())
                    .filter(|s| !s.is_empty())
                    .map(String::from),
                occurred_at,
                raw: cp.clone(),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // A trimmed but real-shaped TrackingMore v4 `/trackings/get` response (data as an array).
    fn fixture() -> Value {
        json!({
            "meta": { "code": 200, "type": "Success", "message": "ok" },
            "data": [{
                "tracking_number": "9400111899223817457635",
                "courier_code": "usps",
                "delivery_status": "delivered",
                "origin_info": {
                    "trackinfo": [
                        { "Date": "2026-06-01 09:00:00", "StatusDescription": "Shipping label created",
                          "Details": "Origin, CA", "checkpoint_delivery_status": "inforeceived" }
                    ]
                },
                "destination_info": {
                    "trackinfo": [
                        { "Date": "2026-06-03 17:11:00", "StatusDescription": "Delivered",
                          "Details": "RENTON, WA 98056", "checkpoint_delivery_status": "delivered",
                          "checkpoint_delivery_substatus": "delivered001" },
                        { "Date": "2026-06-03 08:07:00", "StatusDescription": "Out for delivery",
                          "Details": "RENTON, WA 98059", "checkpoint_delivery_status": "pickup" }
                    ]
                }
            }]
        })
    }

    #[test]
    fn parses_checkpoints_across_legs() {
        let resp = fixture();
        let tracking = extract_tracking(&resp).expect("tracking object");
        let ups = parse_tracking(tracking);
        assert_eq!(ups.len(), 3, "origin (1) + destination (2) checkpoints");

        // origin leg → label created
        assert_eq!(ups[0].status, ShipmentStatus::LabelCreated);
        assert_eq!(ups[0].location.as_deref(), Some("Origin, CA"));
        // destination leg, in document order: delivered then out-for-delivery
        assert_eq!(ups[1].status, ShipmentStatus::Delivered);
        assert_eq!(ups[2].status, ShipmentStatus::OutForDelivery);

        let delivered = ups
            .iter()
            .find(|u| u.status == ShipmentStatus::Delivered)
            .unwrap();
        assert_eq!(
            delivered.occurred_at,
            DateTime::parse_from_rfc3339("2026-06-03T17:11:00Z").unwrap()
        );
        assert_eq!(delivered.description.as_deref(), Some("Delivered"));
    }

    #[test]
    fn status_mapping() {
        assert_eq!(map_status("inforeceived"), ShipmentStatus::LabelCreated);
        assert_eq!(map_status("transit"), ShipmentStatus::InTransit);
        assert_eq!(map_status("pickup"), ShipmentStatus::OutForDelivery);
        assert_eq!(map_status("delivered"), ShipmentStatus::Delivered);
        assert_eq!(map_status("exception"), ShipmentStatus::Exception);
        assert_eq!(map_status("undelivered"), ShipmentStatus::Exception);
        assert_eq!(map_status("notfound"), ShipmentStatus::Unknown);
        assert_eq!(map_status("whatever"), ShipmentStatus::Unknown);
    }

    #[test]
    fn extract_handles_array_object_and_empty() {
        assert!(extract_tracking(&fixture()).is_some());
        assert!(extract_tracking(
            &json!({ "data": { "tracking_number": "x", "origin_info": {} } })
        )
        .is_some());
        assert!(extract_tracking(&json!({ "data": [] })).is_none()); // unregistered → create path
        assert!(extract_tracking(&json!({ "meta": { "code": 200 } })).is_none());
    }

    #[test]
    fn date_formats() {
        assert!(parse_date("2026-06-03 17:11:00").is_some());
        assert!(parse_date("2026-06-03T17:11:00Z").is_some());
        assert!(parse_date("not a date").is_none());
    }

    #[test]
    fn courier_codes() {
        assert_eq!(courier_code(CarrierKind::Usps), Some("usps"));
        assert_eq!(courier_code(CarrierKind::Dhl), Some("dhl"));
        assert_eq!(courier_code(CarrierKind::Other), None);
    }
}
