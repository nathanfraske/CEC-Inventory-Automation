//! Poll engine (scope Section 12.2): fetch carrier updates, write the new normalized
//! `shipment_event` rows, advance the shipment's status, and stop polling once delivered
//! or returned. Idempotent — re-polling inserts only genuinely new events.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use cec_inventory_domain::{CarrierKind, PollState, ShipmentStatus};

use crate::carrier::CarrierProvider;

#[derive(Debug, Serialize)]
pub struct PollOutcome {
    pub shipment_id: Uuid,
    pub new_events: usize,
    pub status: ShipmentStatus,
    pub poll_state: PollState,
}

#[derive(FromRow)]
struct ShipRow {
    carrier: Option<CarrierKind>,
    tracking_number: Option<String>,
    poll_state: PollState,
    status: ShipmentStatus,
}

/// Poll one shipment. Returns early (no-op) if it is already stopped or has no carrier
/// handle yet.
pub async fn poll_shipment(
    pool: &PgPool,
    provider: &dyn CarrierProvider,
    shipment_id: Uuid,
) -> anyhow::Result<PollOutcome> {
    let ship = sqlx::query_as::<_, ShipRow>(
        "SELECT carrier, tracking_number, poll_state, status FROM shipment WHERE id = $1",
    )
    .bind(shipment_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| anyhow::anyhow!("shipment {shipment_id} not found"))?;

    if matches!(ship.poll_state, PollState::Stopped) {
        return Ok(PollOutcome {
            shipment_id,
            new_events: 0,
            status: ship.status,
            poll_state: PollState::Stopped,
        });
    }

    let (carrier, tracking) = match (ship.carrier, ship.tracking_number) {
        (Some(c), Some(t)) => (c, t),
        _ => {
            // Nothing to poll yet; just record that we looked.
            sqlx::query("UPDATE shipment SET last_polled_at = now() WHERE id = $1")
                .bind(shipment_id)
                .execute(pool)
                .await?;
            return Ok(PollOutcome {
                shipment_id,
                new_events: 0,
                status: ship.status,
                poll_state: ship.poll_state,
            });
        }
    };

    let updates = provider.fetch(carrier, &tracking).await?;

    // Existing (status, occurred_at) pairs, for dedup.
    let existing: Vec<(ShipmentStatus, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT event_status, occurred_at FROM shipment_event WHERE shipment_id = $1",
    )
    .bind(shipment_id)
    .fetch_all(pool)
    .await?;

    let mut tx = pool.begin().await?;
    let mut new_events = 0usize;
    for u in &updates {
        let dup = existing
            .iter()
            .any(|(st, at)| *st == u.status && *at == Some(u.occurred_at));
        if dup {
            continue;
        }
        sqlx::query(
            "INSERT INTO shipment_event \
             (shipment_id, event_status, carrier_description, location, occurred_at, raw) \
             VALUES ($1,$2,$3,$4,$5,$6)",
        )
        .bind(shipment_id)
        .bind(u.status)
        .bind(u.description.as_deref())
        .bind(u.location.as_deref())
        .bind(u.occurred_at)
        .bind(&u.raw)
        .execute(&mut *tx)
        .await?;
        new_events += 1;
    }

    // Latest status drives the shipment row; delivery/return stops polling.
    let latest = updates.iter().max_by_key(|u| u.occurred_at);
    let (status, poll_state) = match latest {
        Some(u) => {
            let ps = if matches!(
                u.status,
                ShipmentStatus::Delivered | ShipmentStatus::Returned
            ) {
                PollState::Stopped
            } else {
                PollState::Active
            };
            (u.status, ps)
        }
        None => (ship.status, ship.poll_state),
    };
    let shipped_at = updates
        .iter()
        .filter(|u| {
            matches!(
                u.status,
                ShipmentStatus::InTransit
                    | ShipmentStatus::OutForDelivery
                    | ShipmentStatus::Delivered
            )
        })
        .map(|u| u.occurred_at)
        .min();
    let delivered_at = updates
        .iter()
        .find(|u| matches!(u.status, ShipmentStatus::Delivered))
        .map(|u| u.occurred_at);

    sqlx::query(
        "UPDATE shipment SET status = $1, last_polled_at = now(), \
         shipped_at = COALESCE($2, shipped_at), delivered_at = COALESCE($3, delivered_at), \
         poll_state = $4 WHERE id = $5",
    )
    .bind(status)
    .bind(shipped_at)
    .bind(delivered_at)
    .bind(poll_state)
    .bind(shipment_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(PollOutcome {
        shipment_id,
        new_events,
        status,
        poll_state,
    })
}

/// Poll every active shipment once. Errors on a single shipment are logged and skipped so
/// one bad tracking number does not stall the worker.
pub async fn poll_active_shipments(
    pool: &PgPool,
    provider: &dyn CarrierProvider,
) -> anyhow::Result<Vec<PollOutcome>> {
    let ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM shipment WHERE poll_state = 'active'")
        .fetch_all(pool)
        .await?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        match poll_shipment(pool, provider, id).await {
            Ok(o) => out.push(o),
            Err(e) => tracing::error!("poll failed for shipment {id}: {e}"),
        }
    }
    Ok(out)
}
