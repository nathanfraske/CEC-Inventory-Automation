//! Shared domain types. Map to the Postgres enums in migrations/0001_init.sql.
//! Pattern for a native PG enum: derive sqlx::Type with the matching type_name, and
//! `rename_all = "snake_case"` so the Rust PascalCase variants encode to the PG values.
//! Add the rest of the enums and the row structs as the schema is built out.

use serde::{Deserialize, Serialize};

macro_rules! pg_enum {
    ($(#[$m:meta])* $name:ident => $type_name:literal { $($variant:ident),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
        #[sqlx(type_name = $type_name, rename_all = "snake_case")]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $($variant),+
        }
    };
}

pg_enum!(OwnerKind => "owner_kind" { Shop, Customer });

pg_enum!(CecWarrantyClass => "cec_warranty_class" { Full, Refurb, None });

pg_enum!(ValidationState => "validation_state" { Validated, Invalidated, PendingRevalidation });

pg_enum!(SourceType => "source_type" {
    PhysicalPhoto, Pdf, Email, Manual, TradeIn, OpeningBalance
});

pg_enum!(ResolutionStatus => "resolution_status" { Unresolved, Suggested, Confirmed });

pg_enum!(SerialSource => "serial_source" { Receipt, Scan, Ocr, Manual });

pg_enum!(ConditionKind => "condition_kind" { New, OpenBox, Used, Refurb, Unknown });

pg_enum!(AcquisitionMethod => "acquisition_method" {
    Purchase, TradeIn, RmaReplacement, Gift, Salvage, OpeningBalance
});

pg_enum!(UnitStatus => "unit_status" {
    InStock, Reserved, InBuild, Installed, WithCustomer, Shipped,
    RmaOpen, PendingReturn, Defective, Returned, Scrapped
});

pg_enum!(UnitEventType => "unit_event_type" {
    Intake, StatusChange, SerialEdit, Verify, Reserve, Install, Deliver, Ship,
    LocationChange, OwnerChange, WarrantyRegistered, Revalidated, Transfer,
    RmaOpen, RmaUpdate, ReplaceOut, ReplaceIn, Scrap, Note
});

pg_enum!(CarrierKind => "carrier_kind" { Usps, Ups, Fedex, Dhl, Other });

pg_enum!(ShipmentStatus => "shipment_status" {
    PreTransit, LabelCreated, InTransit, OutForDelivery, Delivered, Exception, Returned, Unknown
});

pg_enum!(PollState => "poll_state" { Active, Stopped });
