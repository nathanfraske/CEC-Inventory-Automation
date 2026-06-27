//! Shared domain types. Map to the Postgres enums in migrations/0001_init.sql.
//! Pattern for a native PG enum: derive sqlx::Type with the matching type_name.
//! Add the rest of the enums and the row structs as the schema is built out.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "owner_kind", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum OwnerKind {
    Shop,
    Customer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "cec_warranty_class", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum CecWarrantyClass {
    Full,
    Refurb,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "validation_state", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum ValidationState {
    Validated,
    Invalidated,
    PendingRevalidation,
}
