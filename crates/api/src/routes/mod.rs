//! Phase 0 resource routes: catalog (vendors/manufacturers/products), purchases with
//! line items and receipt upload, serialized units with event logging, and bulk stock.

use axum::{
    routing::{get, patch, post},
    Router,
};

use crate::AppState;

pub mod catalog;
pub mod purchases;
pub mod stock;
pub mod units;

pub fn router() -> Router<AppState> {
    Router::new()
        // catalog
        .route(
            "/vendors",
            post(catalog::create_vendor).get(catalog::list_vendors),
        )
        .route("/vendors/{id}", get(catalog::get_vendor))
        .route(
            "/manufacturers",
            post(catalog::create_manufacturer).get(catalog::list_manufacturers),
        )
        .route(
            "/products",
            post(catalog::create_product).get(catalog::list_products),
        )
        .route("/products/{id}", get(catalog::get_product))
        // purchases
        .route(
            "/purchases",
            post(purchases::create_purchase).get(purchases::list_purchases),
        )
        .route("/purchases/{id}", get(purchases::get_purchase))
        .route("/purchases/{id}/line-items", post(purchases::add_line_item))
        .route("/purchases/{id}/receipt", post(purchases::upload_receipt))
        // serialized units
        .route("/units", post(units::create_unit).get(units::list_units))
        .route("/units/{id}", get(units::get_unit))
        .route("/units/{id}/status", patch(units::change_status))
        .route("/units/{id}/events", get(units::list_events))
        // bulk stock
        .route("/stock", post(stock::create_stock).get(stock::list_stock))
        .route("/stock/{id}/adjust", post(stock::adjust_stock))
}
