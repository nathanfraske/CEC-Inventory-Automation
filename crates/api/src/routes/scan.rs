//! Serial verification pass + asset-tag printing (scope §13). The verification pass binds
//! or confirms a unit's serial (downgrading a receipt-supplied serial to a confirming scan)
//! and warns on `Product.serial_format_regex` mismatches; asset-tag endpoints assign an
//! internal scannable ID and return a printable label payload (ZPL + human text).

use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::FromRow;
use uuid::Uuid;

use cec_inventory_domain::UnitEventType;

use crate::error::{ApiError, ApiResult};
use crate::events::log_unit_event;
use crate::AppState;

// ---------------- serial-format validation ----------------

/// Validate a serial against an optional regex (scope §13.3). Returns None when there is no
/// regex (nothing to check) or the regex itself is invalid (a warning, not a hard fail).
fn format_valid(regex: Option<&str>, serial: &str) -> (Option<bool>, Option<String>) {
    match regex {
        None => (None, None),
        Some(rx) => match regex::Regex::new(rx) {
            Ok(re) => (Some(re.is_match(serial)), None),
            Err(_) => (
                None,
                Some(format!("product serial_format_regex is invalid: {rx}")),
            ),
        },
    }
}

#[derive(FromRow)]
struct UnitSerialRow {
    serial_number: Option<String>,
    verified: bool,
    serial_format_regex: Option<String>,
}

#[derive(Deserialize)]
pub struct VerifyReq {
    pub scanned_serial: String,
    #[serde(default)]
    pub actor: Option<String>,
}

#[derive(Serialize)]
pub struct VerifyOut {
    pub unit_id: Uuid,
    pub verified: bool,
    pub matched: bool,
    pub bound_from_scan: bool,
    pub format_valid: Option<bool>,
    pub warnings: Vec<String>,
}

/// The verification pass (scope §13.4): bind the serial if the unit had none, confirm it if
/// it matches, or flag a mismatch. Format mismatches warn but never block.
pub async fn verify_unit(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(b): Json<VerifyReq>,
) -> ApiResult<Json<VerifyOut>> {
    let row = sqlx::query_as::<_, UnitSerialRow>(
        "SELECT u.serial_number, u.verified, p.serial_format_regex \
         FROM inventory_unit u LEFT JOIN product p ON p.id = u.product_id WHERE u.id = $1",
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| ApiError::NotFound("unit not found".into()))?;

    let (fmt_valid, fmt_warn) = format_valid(row.serial_format_regex.as_deref(), &b.scanned_serial);
    let mut warnings = Vec::new();
    if let Some(w) = fmt_warn {
        warnings.push(w);
    }
    if fmt_valid == Some(false) {
        warnings.push("scanned serial does not match the product serial format".into());
    }

    let (verified, matched, bound_from_scan) = match row.serial_number.as_deref() {
        None => (true, false, true),
        Some(existing) if existing == b.scanned_serial => (true, true, false),
        Some(_) => (row.verified, false, false),
    };

    let mut tx = s.db.begin().await?;
    if bound_from_scan {
        sqlx::query(
            "UPDATE inventory_unit SET serial_number = $2, serial_source = 'scan', verified = true WHERE id = $1",
        )
        .bind(id)
        .bind(&b.scanned_serial)
        .execute(&mut *tx)
        .await?;
    } else if matched {
        sqlx::query("UPDATE inventory_unit SET verified = true WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    if matched || bound_from_scan {
        log_unit_event(
            &mut *tx,
            id,
            UnitEventType::Verify,
            None,
            Some(&b.scanned_serial),
            b.actor.as_deref(),
            None,
            Some(json!({ "bound_from_scan": bound_from_scan, "format_valid": fmt_valid })),
        )
        .await?;
    }
    tx.commit().await?;

    Ok(Json(VerifyOut {
        unit_id: id,
        verified,
        matched,
        bound_from_scan,
        format_valid: fmt_valid,
        warnings,
    }))
}

// ---------------- asset-tag printing ----------------

#[derive(Serialize)]
pub struct LabelOut {
    pub asset_tag: String,
    pub kind: &'static str,
    pub zpl: String,
    pub label_text: String,
}

/// Code128 ZPL for a thermal label (scope §13.5); a PDF sheet generator is the alternative.
fn zpl(tag: &str) -> String {
    format!("^XA^FO40,40^BCN,120,Y,N,N^FD{tag}^FS^FO40,180^A0N,28,28^FD{tag}^FS^XZ")
}

fn new_tag(prefix: &str) -> String {
    let short = Uuid::new_v4().simple().to_string()[..8].to_uppercase();
    format!("{prefix}-{short}")
}

async fn assign_tag(
    s: &AppState,
    table: &str,
    id: Uuid,
    prefix: &'static str,
    kind: &'static str,
) -> ApiResult<Json<LabelOut>> {
    // Assign a tag if absent; otherwise reuse the existing one (idempotent reprint).
    let existing: Option<Option<String>> =
        sqlx::query_scalar(&format!("SELECT asset_tag FROM {table} WHERE id = $1"))
            .bind(id)
            .fetch_optional(&s.db)
            .await?;
    let existing = existing.ok_or_else(|| ApiError::NotFound(format!("{kind} not found")))?;

    let tag = match existing {
        Some(t) => t,
        None => {
            let t = new_tag(prefix);
            sqlx::query(&format!("UPDATE {table} SET asset_tag = $2 WHERE id = $1"))
                .bind(id)
                .bind(&t)
                .execute(&s.db)
                .await?;
            t
        }
    };
    Ok(Json(LabelOut {
        asset_tag: tag.clone(),
        kind,
        zpl: zpl(&tag),
        label_text: tag,
    }))
}

pub async fn unit_label(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<LabelOut>> {
    let out = assign_tag(&s, "inventory_unit", id, "CEC-U", "unit").await?;
    // Record the tag assignment on the unit timeline.
    log_unit_event(
        &s.db,
        id,
        UnitEventType::Note,
        None,
        Some(&out.0.asset_tag),
        None,
        None,
        Some(json!({ "action": "asset_tag_assigned" })),
    )
    .await?;
    Ok(out)
}

pub async fn system_label(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<LabelOut>> {
    assign_tag(&s, "system", id, "CEC-S", "system").await
}

pub async fn stock_label(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<LabelOut>> {
    assign_tag(&s, "stock_item", id, "CEC-B", "stock item").await
}

#[cfg(test)]
mod tests {
    use super::format_valid;

    #[test]
    fn serial_format_validation() {
        assert_eq!(format_valid(None, "anything").0, None);
        assert_eq!(
            format_valid(Some(r"^GPU-\d{4}[A-Z]$"), "GPU-2291X").0,
            Some(true)
        );
        assert_eq!(
            format_valid(Some(r"^GPU-\d{4}[A-Z]$"), "GPU-22X").0,
            Some(false)
        );
        // invalid regex → unknown + warning, never a hard failure
        let (v, w) = format_valid(Some("("), "x");
        assert_eq!(v, None);
        assert!(w.is_some());
    }
}
