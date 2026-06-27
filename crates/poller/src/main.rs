// Shipment tracking worker (stub). Phase 1 fills in carrier polling per scope Section 12.
// Reads active shipments, calls the carrier provider, writes ShipmentEvents, stops on delivery.
use std::{env, time::Duration};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let interval_secs: u64 = env::var("SHIPMENT_POLL_INTERVAL_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10_800);
    let provider = env::var("CARRIER_PROVIDER").unwrap_or_else(|_| "none".to_string());

    tracing::info!("poller starting: provider={provider}, interval={interval_secs}s");
    let mut tick = tokio::time::interval(Duration::from_secs(interval_secs));
    loop {
        tick.tick().await;
        // TODO Phase 1: load active shipments, poll provider, persist events, stop on delivery.
        tracing::debug!("poll cycle (noop stub)");
    }
}
