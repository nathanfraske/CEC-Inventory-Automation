//! Two-layer warranty computation and RMA readiness (scope Section 5). Pure functions so
//! the rules are unit-tested without a database; the handlers in `routes::warranty` load
//! the inputs and persist the results.

use chrono::{Days, Months, NaiveDate};

use cec_inventory_domain::{CecWarrantyClass, MfrWarrantyBasis};

/// Manufacturer warranty expiry (scope Section 5.2/5.4). `start` is per
/// `Manufacturer.warranty_start_basis`; the basis selects how term is applied.
pub fn mfr_expiry(
    start: Option<NaiveDate>,
    term_months: Option<i32>,
    basis: MfrWarrantyBasis,
    replacement_days: Option<i32>,
    predecessor_expiry: Option<NaiveDate>,
    registered_extra_months: Option<i32>,
) -> Option<NaiveDate> {
    match basis {
        MfrWarrantyBasis::OriginalRemainder => predecessor_expiry,
        MfrWarrantyBasis::ReplacementTerm => {
            let days = replacement_days.unwrap_or(90).max(0) as u64;
            start.and_then(|s| s.checked_add_days(Days::new(days)))
        }
        MfrWarrantyBasis::Standard | MfrWarrantyBasis::Registered => {
            let base = term_months.unwrap_or(0).max(0);
            let extra = if matches!(basis, MfrWarrantyBasis::Registered) {
                registered_extra_months.unwrap_or(0).max(0)
            } else {
                0
            };
            let months = (base + extra) as u32;
            start.and_then(|s| s.checked_add_months(Months::new(months)))
        }
    }
}

/// CEC-provided warranty expiry (scope Section 5.2/5.4): `cec_warranty_start` + the term
/// for the class (from `CecWarrantyPolicy`). `none` class has no expiry.
pub fn cec_expiry(
    start: Option<NaiveDate>,
    class: CecWarrantyClass,
    term_months: Option<i32>,
) -> Option<NaiveDate> {
    if matches!(class, CecWarrantyClass::None) {
        return None;
    }
    let months = term_months.unwrap_or(0).max(0) as u32;
    start.and_then(|s| s.checked_add_months(Months::new(months)))
}

/// The external RMA path (scope Section 5.5). Ownership does not block; it sets the
/// execution path elsewhere (Section 7).
pub fn rma_eligibility(
    has_serial: bool,
    identity_resolved: bool,
    proof_available: bool,
    mfr_expires: Option<NaiveDate>,
    today: NaiveDate,
) -> (bool, Option<&'static str>) {
    if !has_serial {
        return (false, Some("no_serial"));
    }
    if !identity_resolved {
        return (false, Some("identity_unresolved"));
    }
    if !proof_available {
        return (false, Some("no_proof_of_purchase"));
    }
    match mfr_expires {
        Some(e) if e >= today => (true, None),
        _ => (false, Some("warranty_expired")),
    }
}

/// Whether CEC will service the unit under its own warranty (scope Section 5.5): covered
/// class, not expired, and the unit's System is currently validated.
pub fn cec_warranty_active(
    class: CecWarrantyClass,
    cec_expires: Option<NaiveDate>,
    system_validated: bool,
    today: NaiveDate,
) -> bool {
    !matches!(class, CecWarrantyClass::None)
        && cec_expires.map(|e| e >= today).unwrap_or(false)
        && system_validated
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn standard_mfr_term_adds_months() {
        let e = mfr_expiry(
            Some(d("2026-03-02")),
            Some(36),
            MfrWarrantyBasis::Standard,
            None,
            None,
            None,
        );
        assert_eq!(e, Some(d("2029-03-02")));
    }

    #[test]
    fn replacement_term_adds_days() {
        let e = mfr_expiry(
            Some(d("2026-06-01")),
            Some(36),
            MfrWarrantyBasis::ReplacementTerm,
            Some(90),
            None,
            None,
        );
        assert_eq!(e, Some(d("2026-08-30")));
    }

    #[test]
    fn original_remainder_carries_predecessor() {
        let e = mfr_expiry(
            Some(d("2026-06-01")),
            Some(36),
            MfrWarrantyBasis::OriginalRemainder,
            None,
            Some(d("2027-01-15")),
            None,
        );
        assert_eq!(e, Some(d("2027-01-15")));
    }

    #[test]
    fn cec_term_and_none_class() {
        assert_eq!(
            cec_expiry(Some(d("2026-06-01")), CecWarrantyClass::Full, Some(12)),
            Some(d("2027-06-01"))
        );
        assert_eq!(
            cec_expiry(Some(d("2026-06-01")), CecWarrantyClass::None, Some(12)),
            None
        );
    }

    #[test]
    fn rma_block_reasons_in_priority_order() {
        let today = d("2026-06-27");
        assert_eq!(
            rma_eligibility(false, true, true, Some(d("2030-01-01")), today).1,
            Some("no_serial")
        );
        assert_eq!(
            rma_eligibility(true, false, true, Some(d("2030-01-01")), today).1,
            Some("identity_unresolved")
        );
        assert_eq!(
            rma_eligibility(true, true, false, Some(d("2030-01-01")), today).1,
            Some("no_proof_of_purchase")
        );
        assert_eq!(
            rma_eligibility(true, true, true, Some(d("2020-01-01")), today).1,
            Some("warranty_expired")
        );
        assert_eq!(
            rma_eligibility(true, true, true, Some(d("2030-01-01")), today),
            (true, None)
        );
    }

    #[test]
    fn cec_active_requires_validated_system() {
        let today = d("2026-06-27");
        assert!(cec_warranty_active(
            CecWarrantyClass::Full,
            Some(d("2027-01-01")),
            true,
            today
        ));
        assert!(!cec_warranty_active(
            CecWarrantyClass::Full,
            Some(d("2027-01-01")),
            false,
            today
        ));
        assert!(!cec_warranty_active(
            CecWarrantyClass::None,
            Some(d("2027-01-01")),
            true,
            today
        ));
    }
}
