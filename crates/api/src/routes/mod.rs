//! Phase 0 resource routes: catalog (vendors/manufacturers/products), purchases with
//! line items and receipt upload, serialized units with event logging, and bulk stock.

use axum::{
    routing::{get, patch, post},
    Router,
};

use crate::AppState;

pub mod catalog;
pub mod direct;
pub mod intake;
pub mod purchases;
pub mod reports;
pub mod rma;
pub mod scan;
pub mod shipments;
pub mod stock;
pub mod systems;
pub mod ui;
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
        .route(
            "/line-items/{id}/resolve",
            post(purchases::resolve_line_item),
        )
        .route("/line-items/{id}/expand", post(purchases::expand_bundle))
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
        .route("/units/{id}/verify", post(scan::verify_unit))
        .route("/units/{id}/asset-tag", post(scan::unit_label))
        .route("/systems/{id}/asset-tag", post(scan::system_label))
        .route("/stock/{id}/asset-tag", post(scan::stock_label))
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
        // systems + delivery
        .route(
            "/systems",
            post(systems::create_system).get(systems::list_systems),
        )
        .route("/systems/{id}", get(systems::get_system))
        .route("/systems/{id}/members", post(systems::add_member))
        .route(
            "/systems/{id}/members/{unit_id}",
            axum::routing::delete(systems::remove_member),
        )
        .route("/systems/{id}/validate", post(systems::validate_system))
        .route("/systems/{id}/deliver", post(systems::deliver_system))
        .route("/systems/{id}/sweep", post(systems::sweep_system))
        .route("/systems/{id}/transfer", post(systems::transfer_system))
        // cec.direct seam
        .route("/availability", get(direct::availability))
        .route("/units/{id}/reserve", post(direct::reserve_unit))
        .route("/units/{id}/consume", post(direct::consume_unit))
        // RMA lifecycle
        .route("/units/{id}/rma", post(rma::open_rma))
        .route("/rma", get(rma::list_rma))
        .route("/rma/{id}", get(rma::get_rma).patch(rma::update_rma))
        .route("/rma/{id}/proof-package", post(rma::proof_package))
        .route("/rma/{id}/replacement", post(rma::intake_replacement))
        // no-receipt intakes
        .route("/trade-ins", post(intake::create_trade_in))
        .route("/opening-balance", post(intake::create_opening_balance))
        // bulk stock
        .route("/stock", post(stock::create_stock).get(stock::list_stock))
        .route("/stock/{id}/adjust", post(stock::adjust_stock))
        // cross-cutting worklists + export
        .route("/reorder", get(reports::reorder_list))
        .route(
            "/receiving/reconciliation",
            get(reports::receiving_reconciliation),
        )
        .route("/export", get(reports::export_json))
        .route("/export/units.csv", get(reports::export_units_csv))
        // receipt extraction (proxies the Python extractor service, scope §11)
        .route("/extract-preview", post(crate::extractor::extract_preview))
        .route(
            "/purchases/from-extraction",
            post(crate::extractor::create_from_extraction),
        )
}

/// Public, read-only server-rendered UI (scope §18 path 1). Kept outside the auth-protected
/// data routes; a production deploy puts the whole app behind the Headscale mesh + login.
pub fn ui_router() -> Router<AppState> {
    Router::new()
        .route("/", get(ui::dashboard))
        .route("/ui/units", get(ui::units_page))
        .route("/ui/systems", get(ui::systems_page))
        .route("/ui/purchases", get(ui::purchases_page))
        .route("/ui/scan", get(ui::scan_index))
        .route("/ui/scan/{unit_id}", get(ui::scan_page))
}
