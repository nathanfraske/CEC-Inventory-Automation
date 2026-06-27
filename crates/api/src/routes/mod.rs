//! Phase 0 resource routes: catalog (vendors/manufacturers/products), purchases with
//! line items and receipt upload, serialized units with event logging, and bulk stock.

use axum::{
    routing::{get, patch, post},
    Router,
};

use crate::AppState;

pub mod catalog;
pub mod purchases;
pub mod shipments;
pub mod stock;
pub mod units;
pub mod warranty;

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
        .route(
            "/purchases/{id}/allocate-costs",
            post(crate::costing::allocate_costs),
        )
        // shipments + tracking
        .route(
            "/purchases/{id}/shipments",
            post(shipments::create_shipment),
        )
        .route("/shipments", get(shipments::list_shipments))
        .route("/shipments/{id}", get(shipments::get_shipment))
        .route("/shipments/{id}/poll", post(shipments::poll_now))
        // serialized units
        .route("/units", post(units::create_unit).get(units::list_units))
        .route("/units/{id}", get(units::get_unit))
        .route("/units/{id}/status", patch(units::change_status))
        .route("/units/{id}/events", get(units::list_events))
        .route("/units/{id}/warranty", get(warranty::warranty_view))
        .route(
            "/units/{id}/recompute-warranty",
            post(warranty::recompute_warranty),
        )
        // CEC warranty policy
        .route(
            "/warranty-policies",
            post(warranty::create_policy).get(warranty::list_policies),
        )
        // bulk stock
        .route("/stock", post(stock::create_stock).get(stock::list_stock))
        .route("/stock/{id}/adjust", post(stock::adjust_stock))
}
