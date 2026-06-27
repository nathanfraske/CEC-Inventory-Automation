//! Systems: the as-built/as-delivered machine (scope §6). Membership, the validation
//! primitive (a change invalidates; a passing EOL/post-change validation restores), and
//! delivery (shop→customer) which starts the CEC warranty clock per member unit.

use std::collections::HashSet;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{FromRow, PgConnection};
use uuid::Uuid;

use cec_inventory_domain::{
    CecWarrantyClass, CecWarrantyOutcome, OwnerKind, SystemStatus, TransferResult, UnitEventType,
    ValidationResult, ValidationState, ValidationTrigger, ValidationType,
};

use crate::error::{ApiError, ApiResult};
use crate::events::log_unit_event;
use crate::warranty::cec_expiry;
use crate::AppState;

const SYSTEM_COLS: &str = "id, label, asset_tag, build_id, current_owner, customer_ref, status, \
    delivery_datetime, cec_warranty_class, validation_state, provenance_stale, last_validated_at, \
    last_validated_by, notes";

#[derive(Serialize, FromRow)]
pub struct System {
    pub id: Uuid,
    pub label: Option<String>,
    pub asset_tag: Option<String>,
    pub build_id: Option<Uuid>,
    pub current_owner: OwnerKind,
    pub customer_ref: Option<String>,
    pub status: SystemStatus,
    pub delivery_datetime: Option<DateTime<Utc>>,
    pub cec_warranty_class: Option<CecWarrantyClass>,
    pub validation_state: ValidationState,
    pub provenance_stale: bool,
    pub last_validated_at: Option<DateTime<Utc>>,
    pub last_validated_by: Option<String>,
    pub notes: Option<String>,
}

#[derive(Serialize, FromRow)]
pub struct MemberUnit {
    pub id: Uuid,
    pub serial_number: Option<String>,
    pub owner: OwnerKind,
    pub status: String,
    pub cec_warranty_class: Option<CecWarrantyClass>,
    pub cec_warranty_expires: Option<NaiveDate>,
}

#[derive(Serialize)]
pub struct SystemWithMembers {
    #[serde(flatten)]
    pub system: System,
    pub members: Vec<MemberUnit>,
}

#[derive(Deserialize)]
pub struct CreateSystem {
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub asset_tag: Option<String>,
    #[serde(default)]
    pub build_id: Option<Uuid>,
    #[serde(default)]
    pub cec_warranty_class: Option<CecWarrantyClass>,
    #[serde(default)]
    pub notes: Option<String>,
}

pub async fn create_system(
    State(s): State<AppState>,
    Json(b): Json<CreateSystem>,
) -> ApiResult<(StatusCode, Json<System>)> {
    let sql = format!(
        "INSERT INTO system (label, asset_tag, build_id, cec_warranty_class, notes) \
         VALUES ($1,$2,$3,$4,$5) RETURNING {SYSTEM_COLS}"
    );
    let sys = sqlx::query_as::<_, System>(&sql)
        .bind(b.label)
        .bind(b.asset_tag)
        .bind(b.build_id)
        .bind(b.cec_warranty_class)
        .bind(b.notes)
        .fetch_one(&s.db)
        .await?;
    Ok((StatusCode::CREATED, Json(sys)))
}

pub async fn list_systems(State(s): State<AppState>) -> ApiResult<Json<Vec<System>>> {
    let sql = format!("SELECT {SYSTEM_COLS} FROM system ORDER BY id");
    Ok(Json(
        sqlx::query_as::<_, System>(&sql).fetch_all(&s.db).await?,
    ))
}

async fn load_members(s: &AppState, system_id: Uuid) -> Result<Vec<MemberUnit>, sqlx::Error> {
    sqlx::query_as::<_, MemberUnit>(
        "SELECT id, serial_number, owner, status::text AS status, cec_warranty_class, \
         cec_warranty_expires FROM inventory_unit WHERE system_id = $1 ORDER BY id",
    )
    .bind(system_id)
    .fetch_all(&s.db)
    .await
}

async fn fetch_system(s: &AppState, id: Uuid) -> ApiResult<System> {
    let sql = format!("SELECT {SYSTEM_COLS} FROM system WHERE id = $1");
    sqlx::query_as::<_, System>(&sql)
        .bind(id)
        .fetch_optional(&s.db)
        .await?
        .ok_or_else(|| ApiError::NotFound("system not found".into()))
}

/// Lock the system row inside a transaction and return its current state. Every handler that
/// gates on or mutates `validation_state`/ownership/membership takes this FIRST, so the
/// check-then-act is atomic and concurrent ops (deliver vs add_member vs sweep) serialize on
/// the row rather than racing on a stale pre-transaction read (audit: system-gating TOCTOU).
async fn lock_system(conn: &mut PgConnection, id: Uuid) -> ApiResult<System> {
    let sql = format!("SELECT {SYSTEM_COLS} FROM system WHERE id = $1 FOR UPDATE");
    sqlx::query_as::<_, System>(&sql)
        .bind(id)
        .fetch_optional(conn)
        .await?
        .ok_or_else(|| ApiError::NotFound("system not found".into()))
}

pub async fn get_system(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<SystemWithMembers>> {
    let system = fetch_system(&s, id).await?;
    let members = load_members(&s, id).await?;
    Ok(Json(SystemWithMembers { system, members }))
}

#[derive(Deserialize)]
pub struct MemberReq {
    pub unit_id: Uuid,
    #[serde(default)]
    pub actor: Option<String>,
}

/// Add a unit to a system. A membership change invalidates the system (scope §6.4).
pub async fn add_member(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(b): Json<MemberReq>,
) -> ApiResult<Json<SystemWithMembers>> {
    let mut tx = s.db.begin().await?;
    let _ = lock_system(&mut tx, id).await?;
    let updated = sqlx::query("UPDATE inventory_unit SET system_id = $1 WHERE id = $2")
        .bind(id)
        .bind(b.unit_id)
        .execute(&mut *tx)
        .await?;
    if updated.rows_affected() == 0 {
        return Err(ApiError::NotFound("unit not found".into()));
    }
    invalidate(&mut tx, id).await?;
    log_unit_event(
        &mut *tx,
        b.unit_id,
        UnitEventType::Note,
        None,
        None,
        b.actor.as_deref(),
        Some(id),
        Some(json!({ "action": "added_to_system" })),
    )
    .await?;
    tx.commit().await?;
    let system = fetch_system(&s, id).await?;
    let members = load_members(&s, id).await?;
    Ok(Json(SystemWithMembers { system, members }))
}

pub async fn remove_member(
    State(s): State<AppState>,
    Path((id, unit_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<SystemWithMembers>> {
    let mut tx = s.db.begin().await?;
    let _ = lock_system(&mut tx, id).await?;
    let updated =
        sqlx::query("UPDATE inventory_unit SET system_id = NULL WHERE id = $1 AND system_id = $2")
            .bind(unit_id)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    if updated.rows_affected() == 0 {
        return Err(ApiError::NotFound(
            "unit is not a member of this system".into(),
        ));
    }
    invalidate(&mut tx, id).await?;
    log_unit_event(
        &mut *tx,
        unit_id,
        UnitEventType::Note,
        None,
        None,
        None,
        Some(id),
        Some(json!({ "action": "removed_from_system" })),
    )
    .await?;
    tx.commit().await?;
    let system = fetch_system(&s, id).await?;
    let members = load_members(&s, id).await?;
    Ok(Json(SystemWithMembers { system, members }))
}

async fn invalidate(tx: &mut PgConnection, system_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE system SET validation_state = 'invalidated' WHERE id = $1")
        .bind(system_id)
        .execute(tx)
        .await?;
    Ok(())
}

/// Snapshot the current member set (unit_id + serial) for a validation record. Takes the tx
/// connection so the snapshot is consistent with the locked system row.
async fn parts_snapshot(conn: &mut PgConnection, system_id: Uuid) -> Result<Value, sqlx::Error> {
    let rows: Vec<(Uuid, Option<String>)> =
        sqlx::query_as("SELECT id, serial_number FROM inventory_unit WHERE system_id = $1")
            .bind(system_id)
            .fetch_all(conn)
            .await?;
    Ok(Value::Array(
        rows.into_iter()
            .map(|(id, serial)| json!({ "unit_id": id, "serial": serial }))
            .collect(),
    ))
}

#[derive(Deserialize)]
pub struct ValidateReq {
    pub validation_type: ValidationType,
    #[serde(default)]
    pub trigger: Option<ValidationTrigger>,
    pub result: ValidationResult,
    #[serde(default)]
    pub performed_by: Option<String>,
    #[serde(default = "empty_array")]
    pub evidence_refs: Value,
    #[serde(default)]
    pub notes: Option<String>,
}

fn empty_array() -> Value {
    json!([])
}

#[derive(Serialize)]
pub struct ValidationOut {
    pub validation_id: Uuid,
    pub validation_state: ValidationState,
}

/// Record a SystemValidation (scope §6.4). A passing EOL/post-change/periodic validation
/// restores `validated` and CEC coverage; a fail sets `invalidated`.
pub async fn validate_system(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(b): Json<ValidateReq>,
) -> ApiResult<Json<ValidationOut>> {
    let mut tx = s.db.begin().await?;
    let system = lock_system(&mut tx, id).await?;
    let snapshot = parts_snapshot(&mut tx, id).await?;

    let validation_id: Uuid = sqlx::query_scalar(
        "INSERT INTO system_validation \
         (system_id, validation_type, trigger, performed_by, result, parts_snapshot, evidence_refs, notes) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8) RETURNING id",
    )
    .bind(id)
    .bind(b.validation_type)
    .bind(b.trigger)
    .bind(b.performed_by.as_deref())
    .bind(b.result)
    .bind(&snapshot)
    .bind(&b.evidence_refs)
    .bind(b.notes.as_deref())
    .fetch_one(&mut *tx)
    .await?;

    let restores = matches!(
        b.validation_type,
        ValidationType::Eol | ValidationType::PostChange | ValidationType::Periodic
    );
    let new_state = match (b.result, restores) {
        (ValidationResult::Pass, true) => {
            sqlx::query(
                "UPDATE system SET validation_state = 'validated', last_validated_at = now(), \
                 last_validated_by = $2 WHERE id = $1",
            )
            .bind(id)
            .bind(b.performed_by.as_deref())
            .execute(&mut *tx)
            .await?;
            ValidationState::Validated
        }
        (ValidationResult::Fail, _) => {
            invalidate(&mut tx, id).await?;
            ValidationState::Invalidated
        }
        _ => system.validation_state,
    };
    tx.commit().await?;

    Ok(Json(ValidationOut {
        validation_id,
        validation_state: new_state,
    }))
}

#[derive(Deserialize)]
pub struct DeliverReq {
    pub customer_ref: String,
    #[serde(default = "default_class")]
    pub cec_warranty_class: CecWarrantyClass,
    #[serde(default)]
    pub performed_by: Option<String>,
}

fn default_class() -> CecWarrantyClass {
    CecWarrantyClass::Full
}

#[derive(FromRow)]
struct DeliverUnit {
    id: Uuid,
    condition: String,
    category: Option<String>,
}

#[derive(Serialize)]
pub struct DeliverOut {
    pub system_id: Uuid,
    pub delivery_datetime: DateTime<Utc>,
    pub units_delivered: usize,
}

/// Deliver a system to a customer (scope §6.2): flips ownership, stamps delivery time, and
/// starts the CEC clock per member unit. Requires the system to be validated at handoff.
pub async fn deliver_system(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(b): Json<DeliverReq>,
) -> ApiResult<Json<DeliverOut>> {
    let now = Utc::now();
    let delivery_date = now.date_naive();

    let mut tx = s.db.begin().await?;
    let system = lock_system(&mut tx, id).await?;
    if !matches!(system.validation_state, ValidationState::Validated) {
        return Err(ApiError::BadRequest(
            "system must be validated before delivery (scope §6.2)".into(),
        ));
    }
    if matches!(system.current_owner, OwnerKind::Customer) {
        return Err(ApiError::BadRequest("system already delivered".into()));
    }

    let units = sqlx::query_as::<_, DeliverUnit>(
        "SELECT u.id, u.condition::text AS condition, p.category \
         FROM inventory_unit u LEFT JOIN product p ON p.id = u.product_id \
         WHERE u.system_id = $1",
    )
    .bind(id)
    .fetch_all(&mut *tx)
    .await?;

    sqlx::query(
        "UPDATE system SET current_owner = 'customer', customer_ref = $2, delivery_datetime = $3, \
         status = 'delivered', cec_warranty_class = $4 WHERE id = $1",
    )
    .bind(id)
    .bind(&b.customer_ref)
    .bind(now)
    .bind(b.cec_warranty_class)
    .execute(&mut *tx)
    .await?;

    for u in &units {
        // Refurb parts get the refurb class/term; everything else the system default.
        let class = if u.condition == "refurb" {
            CecWarrantyClass::Refurb
        } else {
            b.cec_warranty_class
        };
        let term: Option<i32> = sqlx::query_scalar(
            "SELECT term_months FROM cec_warranty_policy \
             WHERE warranty_class = $1 AND (category = $2 OR category IS NULL) \
             ORDER BY category NULLS LAST LIMIT 1",
        )
        .bind(class)
        .bind(u.category.as_deref())
        .fetch_optional(&mut *tx)
        .await?;
        let expires = cec_expiry(Some(delivery_date), class, term);

        sqlx::query(
            "UPDATE inventory_unit SET owner = 'customer', customer_ref = $2, status = 'with_customer', \
             cec_warranty_start = $3, cec_warranty_class = $4, cec_warranty_expires = $5 WHERE id = $1",
        )
        .bind(u.id)
        .bind(&b.customer_ref)
        .bind(now)
        .bind(class)
        .bind(expires)
        .execute(&mut *tx)
        .await?;

        log_unit_event(
            &mut *tx,
            u.id,
            UnitEventType::Deliver,
            None,
            Some(&b.customer_ref),
            b.performed_by.as_deref(),
            Some(id),
            Some(json!({ "cec_warranty_class": serde_json::to_value(class).ok(), "cec_warranty_expires": expires })),
        )
        .await?;
        log_unit_event(
            &mut *tx,
            u.id,
            UnitEventType::OwnerChange,
            Some("shop"),
            Some("customer"),
            b.performed_by.as_deref(),
            Some(id),
            None,
        )
        .await?;
    }

    // The handoff EOL snapshot (scope §6.2).
    let snapshot = Value::Array(units.iter().map(|u| json!({ "unit_id": u.id })).collect());
    sqlx::query(
        "INSERT INTO system_validation (system_id, validation_type, trigger, performed_by, result, parts_snapshot) \
         VALUES ($1,'eol','build_complete',$2,'pass',$3)",
    )
    .bind(id)
    .bind(b.performed_by.as_deref())
    .bind(&snapshot)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(Json(DeliverOut {
        system_id: id,
        delivery_datetime: now,
        units_delivered: units.len(),
    }))
}

// ---------------- parts sweep (scope §6.5) ----------------

#[derive(Deserialize)]
pub struct SweepReq {
    pub scanned_serials: Vec<String>,
    #[serde(default)]
    pub performed_by: Option<String>,
}

#[derive(Serialize)]
pub struct SweepOut {
    pub validation_id: Uuid,
    pub overall: String,
    pub reconciliation: Value,
    pub validation_state: ValidationState,
}

/// Scan-and-reconcile the system's members against the scanned serial set. A clean sweep
/// re-validates the system (and authorizes a transfer); discrepancies record fail.
pub async fn sweep_system(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(b): Json<SweepReq>,
) -> ApiResult<Json<SweepOut>> {
    let mut tx = s.db.begin().await?;
    let _ = lock_system(&mut tx, id).await?;
    let members: Vec<(Uuid, Option<String>)> =
        sqlx::query_as("SELECT id, serial_number FROM inventory_unit WHERE system_id = $1")
            .bind(id)
            .fetch_all(&mut *tx)
            .await?;

    let scanned: HashSet<&str> = b.scanned_serials.iter().map(|s| s.as_str()).collect();
    let recorded: HashSet<&str> = members.iter().filter_map(|(_, sn)| sn.as_deref()).collect();

    let mut missing = 0usize;
    let per_unit: Vec<Value> = members
        .iter()
        .map(|(uid, sn)| {
            let result = match sn.as_deref() {
                Some(s) if scanned.contains(s) => "matched",
                _ => {
                    missing += 1;
                    "missing"
                }
            };
            json!({ "unit_id": uid, "serial": sn, "result": result })
        })
        .collect();
    let extras: Vec<&str> = scanned
        .iter()
        .copied()
        .filter(|s| !recorded.contains(s))
        .collect();

    let clean = missing == 0 && extras.is_empty();
    let overall = if clean { "clean" } else { "discrepancies" };
    let reconciliation =
        json!({ "per_unit": per_unit, "unexpected_extra": extras, "overall": overall });
    let result = if clean {
        ValidationResult::Pass
    } else {
        ValidationResult::Fail
    };
    let snapshot = parts_snapshot(&mut tx, id).await?;

    let validation_id: Uuid = sqlx::query_scalar(
        "INSERT INTO system_validation \
         (system_id, validation_type, trigger, performed_by, result, parts_snapshot, reconciliation) \
         VALUES ($1,'sweep','transfer_request',$2,$3,$4,$5) RETURNING id",
    )
    .bind(id)
    .bind(b.performed_by.as_deref())
    .bind(result)
    .bind(&snapshot)
    .bind(&reconciliation)
    .fetch_one(&mut *tx)
    .await?;

    let new_state = if clean {
        sqlx::query(
            "UPDATE system SET validation_state = 'validated', last_validated_at = now(), \
             last_validated_by = $2 WHERE id = $1",
        )
        .bind(id)
        .bind(b.performed_by.as_deref())
        .execute(&mut *tx)
        .await?;
        ValidationState::Validated
    } else {
        invalidate(&mut tx, id).await?;
        ValidationState::Invalidated
    };
    tx.commit().await?;

    Ok(Json(SweepOut {
        validation_id,
        overall: overall.to_string(),
        reconciliation,
        validation_state: new_state,
    }))
}

// ---------------- warranty transfer (scope §6.5) ----------------

#[derive(Deserialize)]
pub struct TransferReq {
    pub to_owner_ref: String,
    #[serde(default)]
    pub performed_by: Option<String>,
    /// The authorizing clean sweep; if omitted, the system must currently be `validated`.
    #[serde(default)]
    pub sweep_id: Option<Uuid>,
    #[serde(default = "default_outcome")]
    pub cec_warranty_outcome: CecWarrantyOutcome,
    #[serde(default)]
    pub cec_transfer_fee: Option<rust_decimal::Decimal>,
}

fn default_outcome() -> CecWarrantyOutcome {
    CecWarrantyOutcome::Carried
}

#[derive(Serialize)]
pub struct TransferOut {
    pub transfer_id: Uuid,
    pub result: TransferResult,
    pub from_owner_ref: Option<String>,
    pub to_owner_ref: String,
    pub mfr_warranty_outcome: Value,
}

/// Transfer a delivered system to a new owner (scope §6.5). Precondition: a clean sweep
/// (or a currently-validated system). Manufacturer warranty carries per-part only where the
/// maker allows it; non-transferable parts are flagged void-on-transfer.
pub async fn transfer_system(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(b): Json<TransferReq>,
) -> ApiResult<Json<TransferOut>> {
    let mut tx = s.db.begin().await?;
    let system = lock_system(&mut tx, id).await?;
    if !matches!(system.current_owner, OwnerKind::Customer) {
        return Err(ApiError::BadRequest(
            "only a delivered (customer-owned) system can be transferred".into(),
        ));
    }

    // Authorize via the named sweep, or a currently-validated system.
    let authorized = match b.sweep_id {
        Some(sid) => {
            let ok: Option<(String, String)> = sqlx::query_as(
                "SELECT result::text, validation_type::text FROM system_validation \
                 WHERE id = $1 AND system_id = $2",
            )
            .bind(sid)
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
            matches!(ok, Some((r, t)) if r == "pass" && (t == "sweep" || t == "pre_transfer"))
        }
        None => matches!(system.validation_state, ValidationState::Validated),
    };
    if !authorized {
        return Err(ApiError::BadRequest(
            "transfer blocked: a clean parts sweep is required (scope §6.5)".into(),
        ));
    }

    // Per-part manufacturer transferability.
    let parts: Vec<(Uuid, Option<bool>)> = sqlx::query_as(
        "SELECT u.id, m.warranty_transferable FROM inventory_unit u \
         LEFT JOIN product p ON p.id = u.product_id \
         LEFT JOIN manufacturer m ON m.id = p.manufacturer_id WHERE u.system_id = $1",
    )
    .bind(id)
    .fetch_all(&mut *tx)
    .await?;
    let mfr_outcome = Value::Array(
        parts
            .iter()
            .map(|(uid, transferable)| {
                let outcome = if transferable.unwrap_or(true) {
                    "carried"
                } else {
                    "void_non_transferable"
                };
                json!({ "unit_id": uid, "outcome": outcome })
            })
            .collect(),
    );

    let from_owner_ref = system.customer_ref.clone();
    sqlx::query("UPDATE system SET customer_ref = $2, status = 'in_service' WHERE id = $1")
        .bind(id)
        .bind(&b.to_owner_ref)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE inventory_unit SET customer_ref = $2 WHERE system_id = $1")
        .bind(id)
        .bind(&b.to_owner_ref)
        .execute(&mut *tx)
        .await?;

    let transfer_id: Uuid = sqlx::query_scalar(
        "INSERT INTO system_transfer \
         (system_id, from_owner_ref, to_owner_ref, performed_by, sweep_id, mfr_warranty_outcome, \
          cec_warranty_outcome, cec_transfer_fee, result) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'completed') RETURNING id",
    )
    .bind(id)
    .bind(from_owner_ref.as_deref())
    .bind(&b.to_owner_ref)
    .bind(b.performed_by.as_deref())
    .bind(b.sweep_id)
    .bind(&mfr_outcome)
    .bind(b.cec_warranty_outcome)
    .bind(b.cec_transfer_fee)
    .fetch_one(&mut *tx)
    .await?;

    for (uid, _) in &parts {
        log_unit_event(
            &mut *tx,
            *uid,
            UnitEventType::Transfer,
            from_owner_ref.as_deref(),
            Some(&b.to_owner_ref),
            b.performed_by.as_deref(),
            Some(id),
            None,
        )
        .await?;
        log_unit_event(
            &mut *tx,
            *uid,
            UnitEventType::OwnerChange,
            from_owner_ref.as_deref(),
            Some(&b.to_owner_ref),
            b.performed_by.as_deref(),
            Some(id),
            None,
        )
        .await?;
    }
    tx.commit().await?;

    Ok(Json(TransferOut {
        transfer_id,
        result: TransferResult::Completed,
        from_owner_ref,
        to_owner_ref: b.to_owner_ref,
        mfr_warranty_outcome: mfr_outcome,
    }))
}
