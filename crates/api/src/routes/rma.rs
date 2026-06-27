//! RMA lifecycle (scope §7): a case opens on a failed unit and runs through one of three
//! execution modes. Includes the proof-of-purchase package (7.4) and replacement intake
//! with remainder-of-term warranty and system re-validation (7.6/7.7).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::FromRow;
use uuid::Uuid;

use cec_inventory_domain::{
    AcquisitionMethod, CecWarrantyClass, ConditionKind, MfrWarrantyBasis, OwnerKind, RmaCustody,
    RmaExecutionMode, RmaParty, RmaProofSource, RmaStatus, UnitEventType,
};

use crate::error::{ApiError, ApiResult};
use crate::events::log_unit_event;
use crate::AppState;

const RMA_COLS: &str = "id, unit_id, owner_at_failure, party, execution_mode, proof_source, \
    custody, rma_number, fault_description, status, assist_artifacts, advance_replacement, \
    auth_hold_ref, return_due_date, opened_at, closed_at, shipped_at, return_tracking, \
    replacement_unit_id, resolution, notes";

#[derive(Serialize, FromRow)]
pub struct RmaCase {
    pub id: Uuid,
    pub unit_id: Uuid,
    pub owner_at_failure: Option<OwnerKind>,
    pub party: Option<RmaParty>,
    pub execution_mode: Option<RmaExecutionMode>,
    pub proof_source: Option<RmaProofSource>,
    pub custody: Option<RmaCustody>,
    pub rma_number: Option<String>,
    pub fault_description: Option<String>,
    pub status: RmaStatus,
    pub assist_artifacts: Option<Value>,
    pub advance_replacement: bool,
    pub auth_hold_ref: Option<String>,
    pub return_due_date: Option<NaiveDate>,
    pub opened_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
    pub shipped_at: Option<DateTime<Utc>>,
    pub return_tracking: Option<String>,
    pub replacement_unit_id: Option<Uuid>,
    pub resolution: Option<String>,
    pub notes: Option<String>,
}

#[derive(FromRow)]
struct UnitForRma {
    owner: OwnerKind,
    line_item_id: Option<Uuid>,
    status: String,
    system_id: Option<Uuid>,
    product_id: Option<Uuid>,
    customer_ref: Option<String>,
}

#[derive(Deserialize)]
pub struct OpenRma {
    #[serde(default)]
    pub party: Option<RmaParty>,
    #[serde(default)]
    pub execution_mode: Option<RmaExecutionMode>,
    #[serde(default)]
    pub fault_description: Option<String>,
    #[serde(default)]
    pub advance_replacement: bool,
    #[serde(default)]
    pub return_due_date: Option<NaiveDate>,
    #[serde(default)]
    pub rma_number: Option<String>,
    #[serde(default)]
    pub actor: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

async fn load_unit_for_rma(s: &AppState, unit_id: Uuid) -> ApiResult<UnitForRma> {
    sqlx::query_as::<_, UnitForRma>(
        "SELECT owner, line_item_id, status::text AS status, system_id, product_id, customer_ref \
         FROM inventory_unit WHERE id = $1",
    )
    .bind(unit_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| ApiError::NotFound("unit not found".into()))
}

async fn fetch_case(s: &AppState, id: Uuid) -> ApiResult<RmaCase> {
    let sql = format!("SELECT {RMA_COLS} FROM rma_case WHERE id = $1");
    sqlx::query_as::<_, RmaCase>(&sql)
        .bind(id)
        .fetch_optional(&s.db)
        .await?
        .ok_or_else(|| ApiError::NotFound("rma case not found".into()))
}

/// Open an RMA on a failed unit (scope §7.1/7.2). Defaults derive the execution mode,
/// proof source, and custody from ownership; all are overridable.
pub async fn open_rma(
    State(s): State<AppState>,
    Path(unit_id): Path<Uuid>,
    Json(b): Json<OpenRma>,
) -> ApiResult<(StatusCode, Json<RmaCase>)> {
    let unit = load_unit_for_rma(&s, unit_id).await?;

    let proof_source = if matches!(unit.owner, OwnerKind::Shop) && unit.line_item_id.is_some() {
        RmaProofSource::CecReceipt
    } else {
        RmaProofSource::CustomerReceipt
    };
    let execution_mode = b.execution_mode.unwrap_or(match unit.owner {
        OwnerKind::Customer => RmaExecutionMode::CustomerManagedAssist,
        OwnerKind::Shop => RmaExecutionMode::CecManaged,
    });
    let custody = if unit.status == "with_customer" {
        RmaCustody::WithCustomer
    } else {
        RmaCustody::AtCec
    };

    let mut tx = s.db.begin().await?;
    let sql = format!(
        "INSERT INTO rma_case \
         (unit_id, owner_at_failure, party, execution_mode, proof_source, custody, rma_number, \
          fault_description, advance_replacement, return_due_date, notes) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) RETURNING {RMA_COLS}"
    );
    let case = sqlx::query_as::<_, RmaCase>(&sql)
        .bind(unit_id)
        .bind(unit.owner)
        .bind(b.party)
        .bind(execution_mode)
        .bind(proof_source)
        .bind(custody)
        .bind(b.rma_number.as_deref())
        .bind(b.fault_description.as_deref())
        .bind(b.advance_replacement)
        .bind(b.return_due_date)
        .bind(b.notes.as_deref())
        .fetch_one(&mut *tx)
        .await?;

    sqlx::query("UPDATE inventory_unit SET status = 'rma_open' WHERE id = $1")
        .bind(unit_id)
        .execute(&mut *tx)
        .await?;
    log_unit_event(
        &mut *tx,
        unit_id,
        UnitEventType::RmaOpen,
        Some(&unit.status),
        Some("rma_open"),
        b.actor.as_deref(),
        unit.system_id,
        Some(json!({ "rma_case_id": case.id, "execution_mode": serde_json::to_value(execution_mode).ok() })),
    )
    .await?;
    tx.commit().await?;

    Ok((StatusCode::CREATED, Json(case)))
}

pub async fn list_rma(State(s): State<AppState>) -> ApiResult<Json<Vec<RmaCase>>> {
    let sql = format!("SELECT {RMA_COLS} FROM rma_case ORDER BY opened_at DESC");
    Ok(Json(
        sqlx::query_as::<_, RmaCase>(&sql).fetch_all(&s.db).await?,
    ))
}

pub async fn get_rma(State(s): State<AppState>, Path(id): Path<Uuid>) -> ApiResult<Json<RmaCase>> {
    Ok(Json(fetch_case(&s, id).await?))
}

#[derive(Deserialize)]
pub struct UpdateRma {
    #[serde(default)]
    pub status: Option<RmaStatus>,
    #[serde(default)]
    pub custody: Option<RmaCustody>,
    #[serde(default)]
    pub rma_number: Option<String>,
    #[serde(default)]
    pub return_tracking: Option<String>,
    #[serde(default)]
    pub resolution: Option<String>,
    #[serde(default)]
    pub auth_hold_ref: Option<String>,
    #[serde(default)]
    pub actor: Option<String>,
}

/// Update a case's status/custody/tracking (scope §7.3). COALESCE keeps unset fields.
pub async fn update_rma(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(b): Json<UpdateRma>,
) -> ApiResult<Json<RmaCase>> {
    let case = fetch_case(&s, id).await?;
    let closed = matches!(b.status, Some(RmaStatus::Closed));

    // The case UPDATE and its event must be one atomic unit (else the status can change with
    // no `rma_update` event — a provenance gap, scope §16). Lock + read the prior status
    // inside the tx so `from_value` is accurate under concurrent updates.
    let mut tx = s.db.begin().await?;
    let prior_status: String =
        sqlx::query_scalar("SELECT status::text FROM rma_case WHERE id = $1 FOR UPDATE")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    let sql = format!(
        "UPDATE rma_case SET status = COALESCE($2, status), custody = COALESCE($3, custody), \
         rma_number = COALESCE($4, rma_number), return_tracking = COALESCE($5, return_tracking), \
         resolution = COALESCE($6, resolution), auth_hold_ref = COALESCE($7, auth_hold_ref), \
         closed_at = CASE WHEN $8 THEN now() ELSE closed_at END WHERE id = $1 RETURNING {RMA_COLS}"
    );
    let updated = sqlx::query_as::<_, RmaCase>(&sql)
        .bind(id)
        .bind(b.status)
        .bind(b.custody)
        .bind(b.rma_number.as_deref())
        .bind(b.return_tracking.as_deref())
        .bind(b.resolution.as_deref())
        .bind(b.auth_hold_ref.as_deref())
        .bind(closed)
        .fetch_one(&mut *tx)
        .await?;
    let to_val = if b.status.is_some() {
        serde_json::to_value(updated.status)
            .ok()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
    } else {
        None
    };
    log_unit_event(
        &mut *tx,
        case.unit_id,
        UnitEventType::RmaUpdate,
        Some(&prior_status),
        to_val.as_deref(),
        b.actor.as_deref(),
        None,
        Some(json!({ "rma_case_id": id })),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(updated))
}

/// Build the proof-of-purchase package (scope §7.4): receipt, serial, purchase facts, and
/// the manufacturer warranty terms bundled for the customer's filing. Stored on the case.
pub async fn proof_package(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let case = fetch_case(&s, id).await?;
    let row = sqlx::query_as::<_, ProofRow>(
        "SELECT u.serial_number, u.asset_tag, u.mfr_warranty_expires, p.model, p.mpn, \
                m.name AS mfr_name, pur.receipt_files, pur.purchase_datetime, v.name AS vendor_name \
         FROM inventory_unit u \
         LEFT JOIN product p ON p.id = u.product_id \
         LEFT JOIN manufacturer m ON m.id = p.manufacturer_id \
         LEFT JOIN purchase_line_item li ON li.id = u.line_item_id \
         LEFT JOIN purchase pur ON pur.id = li.purchase_id \
         LEFT JOIN vendor v ON v.id = pur.vendor_id \
         WHERE u.id = $1",
    )
    .bind(case.unit_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| ApiError::NotFound("unit not found".into()))?;

    let package = json!({
        "rma_case_id": id,
        "unit_id": case.unit_id,
        "serial_number": row.serial_number,
        "asset_tag": row.asset_tag,
        "product": { "model": row.model, "mpn": row.mpn, "manufacturer": row.mfr_name },
        "mfr_warranty_expires": row.mfr_warranty_expires,
        "purchase": {
            "datetime": row.purchase_datetime,
            "vendor": row.vendor_name,
            "receipt_files": row.receipt_files,
        },
        "generated_at": Utc::now(),
    });

    // Persist a reference on the case and write the artifact to the object store.
    let rel = format!("proof-packages/{id}.json");
    let abs = s.storage_root.join(&rel);
    if let Some(parent) = abs.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
    }
    tokio::fs::write(&abs, serde_json::to_vec_pretty(&package).unwrap())
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    sqlx::query(
        "UPDATE rma_case SET assist_artifacts = jsonb_set(COALESCE(assist_artifacts,'{}'::jsonb), \
         '{proof_package_ref}', $2::jsonb, true) WHERE id = $1",
    )
    .bind(id)
    .bind(json!(rel))
    .execute(&s.db)
    .await?;

    Ok(Json(package))
}

#[derive(FromRow)]
struct ProofRow {
    serial_number: Option<String>,
    asset_tag: Option<String>,
    mfr_warranty_expires: Option<NaiveDate>,
    model: Option<String>,
    mpn: Option<String>,
    mfr_name: Option<String>,
    receipt_files: Option<Value>,
    purchase_datetime: Option<DateTime<Utc>>,
    vendor_name: Option<String>,
}

#[derive(Deserialize)]
pub struct ReplacementReq {
    #[serde(default)]
    pub serial_number: Option<String>,
    #[serde(default = "default_refurb_false")]
    pub refurbished: bool,
    #[serde(default)]
    pub unit_cost: Option<rust_decimal::Decimal>,
    #[serde(default)]
    pub actor: Option<String>,
}

fn default_refurb_false() -> bool {
    false
}

/// Intake a replacement unit (scope §7.6/7.7): new unit with `rma_replacement`, links to its
/// predecessor, inherits the system (which re-validates), remainder-of-term mfr warranty.
pub async fn intake_replacement(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(b): Json<ReplacementReq>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let case = fetch_case(&s, id).await?;
    let failed = load_unit_for_rma(&s, case.unit_id).await?;
    let product_id = failed
        .product_id
        .ok_or_else(|| ApiError::BadRequest("failed unit has no product".into()))?;

    let (condition, cec_class) = if b.refurbished {
        (ConditionKind::Refurb, CecWarrantyClass::Refurb)
    } else {
        (ConditionKind::New, CecWarrantyClass::Full)
    };
    // A replacement that joins a customer system stays with the customer.
    let status = if failed.status == "with_customer" {
        "with_customer"
    } else {
        "in_stock"
    };

    let mut tx = s.db.begin().await?;
    let new_unit: Uuid = sqlx::query_scalar(
        "INSERT INTO inventory_unit \
         (product_id, system_id, owner, customer_ref, serial_number, condition, \
          acquisition_method, status, unit_cost, mfr_warranty_basis, cec_warranty_class, replaces_unit_id) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8::unit_status,$9,$10,$11,$12) RETURNING id",
    )
    .bind(product_id)
    .bind(failed.system_id)
    .bind(failed.owner)
    .bind(failed.customer_ref.as_deref())
    .bind(b.serial_number.as_deref())
    .bind(condition)
    .bind(AcquisitionMethod::RmaReplacement)
    .bind(status)
    .bind(b.unit_cost)
    .bind(MfrWarrantyBasis::ReplacementTerm)
    .bind(cec_class)
    .bind(case.unit_id)
    .fetch_one(&mut *tx)
    .await?;

    // Predecessor retires; case points to the replacement.
    sqlx::query("UPDATE inventory_unit SET status = 'returned' WHERE id = $1")
        .bind(case.unit_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE rma_case SET replacement_unit_id = $2, status = 'replacement_received' WHERE id = $1",
    )
    .bind(id)
    .bind(new_unit)
    .execute(&mut *tx)
    .await?;

    // A replacement is a membership change → the system re-validates (scope §7.7).
    if let Some(sys) = failed.system_id {
        sqlx::query("UPDATE system SET validation_state = 'invalidated' WHERE id = $1")
            .bind(sys)
            .execute(&mut *tx)
            .await?;
    }

    log_unit_event(
        &mut *tx,
        case.unit_id,
        UnitEventType::ReplaceOut,
        Some(&failed.status),
        Some("returned"),
        b.actor.as_deref(),
        failed.system_id,
        Some(json!({ "rma_case_id": id, "replacement_unit_id": new_unit })),
    )
    .await?;
    log_unit_event(
        &mut *tx,
        new_unit,
        UnitEventType::ReplaceIn,
        None,
        Some(status),
        b.actor.as_deref(),
        failed.system_id,
        Some(json!({ "rma_case_id": id, "replaces_unit_id": case.unit_id })),
    )
    .await?;
    tx.commit().await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "replacement_unit_id": new_unit,
            "replaces_unit_id": case.unit_id,
            "condition": serde_json::to_value(condition).ok(),
            "system_revalidation_required": failed.system_id.is_some(),
        })),
    ))
}
