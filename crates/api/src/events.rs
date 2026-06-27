//! Append-only unit event log (scope Section 16). Every unit mutation writes a row
//! here, so a unit's provenance is reconstructable for RMA and transfer disputes.

use cec_inventory_domain::UnitEventType;
use sqlx::PgExecutor;
use uuid::Uuid;

/// Insert one `unit_event`. Pass a transaction (`&mut *tx`) so the event commits
/// atomically with the mutation it records.
#[allow(clippy::too_many_arguments)]
pub async fn log_unit_event<'e, E>(
    exec: E,
    unit_id: Uuid,
    event_type: UnitEventType,
    from_value: Option<&str>,
    to_value: Option<&str>,
    actor: Option<&str>,
    system_id: Option<Uuid>,
    detail: Option<serde_json::Value>,
) -> Result<(), sqlx::Error>
where
    E: PgExecutor<'e>,
{
    sqlx::query(
        "INSERT INTO unit_event \
         (unit_id, event_type, from_value, to_value, actor, system_id, detail) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(unit_id)
    .bind(event_type)
    .bind(from_value)
    .bind(to_value)
    .bind(actor)
    .bind(system_id)
    .bind(detail)
    .execute(exec)
    .await?;
    Ok(())
}
