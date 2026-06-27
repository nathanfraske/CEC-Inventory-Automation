// Shipment tracking worker (scope Section 12). Loads active shipments on a cadence, polls
// the configured carrier provider, writes ShipmentEvents, and stops on delivery. The poll
// engine is shared with the API (`POST /shipments/{id}/poll`) in crate cec-inventory-tracking.
use std::{env, time::Duration};

use sqlx::postgres::PgPoolOptions;

use cec_inventory_tracking::{poll_active_shipments, provider_from_env};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let interval_secs: u64 = env::var("SHIPMENT_POLL_INTERVAL_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10_800);
    let database_url =
        env::var("DATABASE_URL").expect("DATABASE_URL must be set (see .env, never commit it)");

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await?;
    let provider = provider_from_env();

    tracing::info!(
        "poller starting: provider={}, interval={interval_secs}s",
        provider.name()
    );
    let mut tick = tokio::time::interval(Duration::from_secs(interval_secs));
    loop {
        tick.tick().await;
        match poll_active_shipments(&pool, provider.as_ref()).await {
            Ok(outcomes) if !outcomes.is_empty() => {
                let new: usize = outcomes.iter().map(|o| o.new_events).sum();
                tracing::info!(
                    "polled {} active shipment(s), {new} new event(s)",
                    outcomes.len()
                );
            }
            Ok(_) => tracing::debug!("no active shipments"),
            Err(e) => tracing::error!("poll cycle failed: {e}"),
        }
    }
}
