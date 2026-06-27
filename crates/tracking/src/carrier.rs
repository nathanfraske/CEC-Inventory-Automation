//! Carrier provider seam. The poll engine talks to a `CarrierProvider`; real carrier
//! APIs (USPS/UPS/FedEx/DHL) or an aggregator (EasyPost/AfterShip) plug in here later
//! (scope Section 12.3 / INV-OQ-30). Ships now with a `none` no-op and a deterministic
//! `mock` for tests and local demos.

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::json;

use cec_inventory_domain::{CarrierKind, ShipmentStatus};

/// One normalized carrier status update. `status` is already mapped to the common set.
#[derive(Debug, Clone)]
pub struct CarrierUpdate {
    pub status: ShipmentStatus,
    pub description: Option<String>,
    pub location: Option<String>,
    pub occurred_at: DateTime<Utc>,
    pub raw: serde_json::Value,
}

#[async_trait]
pub trait CarrierProvider: Send + Sync {
    /// Return the full known status history for a tracking number (poll dedups).
    async fn fetch(
        &self,
        carrier: CarrierKind,
        tracking_number: &str,
    ) -> anyhow::Result<Vec<CarrierUpdate>>;

    fn name(&self) -> &'static str;
}

/// Default safe provider: knows nothing, returns no updates. Used until a real carrier
/// integration or aggregator is configured (`CARRIER_PROVIDER=none`).
pub struct NoneProvider;

#[async_trait]
impl CarrierProvider for NoneProvider {
    async fn fetch(&self, _c: CarrierKind, _t: &str) -> anyhow::Result<Vec<CarrierUpdate>> {
        Ok(Vec::new())
    }
    fn name(&self) -> &'static str {
        "none"
    }
}

#[derive(Clone, Copy)]
pub enum MockMode {
    /// Return the whole timeline on every poll (one poll → delivered). Good for demos.
    Full,
    /// Return a growing prefix, advancing one stage per call. Good for progression tests.
    Stepwise,
}

/// Deterministic provider with a fixed pre_transit → in_transit → out_for_delivery →
/// delivered timeline. Timestamps are fixed so dedup is stable across polls.
pub struct MockProvider {
    timeline: Vec<CarrierUpdate>,
    mode: MockMode,
    step: Arc<AtomicUsize>,
}

impl MockProvider {
    pub fn new(mode: MockMode) -> Self {
        let base = DateTime::parse_from_rfc3339("2026-06-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let hours = |h: i64| base + chrono::Duration::hours(h);
        let timeline = vec![
            mk(
                ShipmentStatus::PreTransit,
                hours(0),
                "Shipping label created",
                "Origin",
            ),
            mk(
                ShipmentStatus::InTransit,
                hours(12),
                "Departed facility",
                "Hebron, KY",
            ),
            mk(
                ShipmentStatus::OutForDelivery,
                hours(36),
                "Out for delivery",
                "Local facility",
            ),
            mk(
                ShipmentStatus::Delivered,
                hours(40),
                "Delivered, front door",
                "Destination",
            ),
        ];
        Self {
            timeline,
            mode,
            step: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn full() -> Self {
        Self::new(MockMode::Full)
    }

    pub fn stepwise() -> Self {
        Self::new(MockMode::Stepwise)
    }
}

fn mk(status: ShipmentStatus, occurred_at: DateTime<Utc>, desc: &str, loc: &str) -> CarrierUpdate {
    CarrierUpdate {
        status,
        description: Some(desc.to_string()),
        location: Some(loc.to_string()),
        occurred_at,
        raw: json!({ "mock": true, "description": desc, "location": loc }),
    }
}

#[async_trait]
impl CarrierProvider for MockProvider {
    async fn fetch(&self, _c: CarrierKind, _t: &str) -> anyhow::Result<Vec<CarrierUpdate>> {
        let take = match self.mode {
            MockMode::Full => self.timeline.len(),
            MockMode::Stepwise => {
                let n = self.step.fetch_add(1, Ordering::SeqCst) + 1;
                n.min(self.timeline.len())
            }
        };
        Ok(self.timeline[..take].to_vec())
    }
    fn name(&self) -> &'static str {
        "mock"
    }
}

/// Build the provider named by `CARRIER_PROVIDER`. Real carriers are not implemented yet
/// and fall back to `none` with a warning so the poller is always safe to run.
pub fn provider_from_env() -> Box<dyn CarrierProvider> {
    match std::env::var("CARRIER_PROVIDER")
        .unwrap_or_default()
        .as_str()
    {
        "mock" => Box::new(MockProvider::full()),
        "none" | "" => Box::new(NoneProvider),
        other => {
            tracing::warn!(
                "CARRIER_PROVIDER='{other}' not implemented yet; using no-op provider (scope INV-OQ-30)"
            );
            Box::new(NoneProvider)
        }
    }
}
