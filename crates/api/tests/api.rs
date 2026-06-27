//! End-to-end Phase 0 flow against a live Postgres. Skipped automatically when
//! `DATABASE_URL` is unset (e.g. CI, which builds DB-free). Run locally with a DB up:
//!   set -a; . ./.env; set +a; cargo test -p cec-inventory-api

use serde_json::{json, Value};

/// Unique-per-run serial. `inventory_unit.serial_number` is globally unique (migration 0003),
/// so fixed serials would collide on a re-run against a persistent dev DB; this keeps tests
/// idempotent. (CI runs against a fresh DB regardless.)
fn sn(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4().simple())
}

async fn spawn() -> Option<String> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("skipping integration test: DATABASE_URL not set");
        return None;
    }
    // Isolated object-store root per run, outside the repo.
    let dir = std::env::temp_dir().join(format!("cec-test-objects-{}", uuid::Uuid::new_v4()));
    std::env::set_var("STORAGE_FS_ROOT", &dir);

    let state = cec_inventory_api::build_state().await.expect("build_state");
    // Auth off for the resource-route tests; the auth flow has its own spawn below.
    let app = cec_inventory_api::build_app(state, false);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Some(format!("http://{addr}"))
}

/// Spawn the app with auth enabled (production wiring) for the auth flow test.
async fn spawn_authed() -> Option<String> {
    if std::env::var("DATABASE_URL").is_err() {
        return None;
    }
    let dir = std::env::temp_dir().join(format!("cec-test-objects-{}", uuid::Uuid::new_v4()));
    std::env::set_var("STORAGE_FS_ROOT", &dir);
    let state = cec_inventory_api::build_state().await.expect("build_state");
    let app = cec_inventory_api::app(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Some(format!("http://{addr}"))
}

#[tokio::test]
async fn phase0_crud_and_event_log_flow() {
    let Some(base) = spawn().await else { return };
    let c = reqwest::Client::new();

    // health spine
    let health = c.get(format!("{base}/health")).send().await.unwrap();
    assert_eq!(health.text().await.unwrap(), "ok");

    // catalog: vendor + product
    let vendor: Value = c
        .post(format!("{base}/vendors"))
        .json(&json!({ "name": "Micro Center" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let vendor_id = vendor["id"].as_str().unwrap().to_string();

    let product: Value = c
        .post(format!("{base}/products"))
        .json(&json!({ "model": "RTX 4090", "category": "gpu", "serialized": true }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let product_id = product["id"].as_str().unwrap().to_string();

    // purchase with a line item (money as JSON strings → numeric(12,2))
    let purchase: Value = c
        .post(format!("{base}/purchases"))
        .json(&json!({
            "vendor_id": vendor_id,
            "source_type": "manual",
            "currency": "USD",
            "total": "1999.00",
            "line_items": [{
                "product_id": product_id,
                "description_as_printed": "RTX 4090",
                "quantity": 1,
                "unit_price": "1999.00",
                "line_total": "1999.00"
            }]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let purchase_id = purchase["id"].as_str().unwrap().to_string();
    let line_item_id = purchase["line_items"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(purchase["line_items"].as_array().unwrap().len(), 1);
    assert_eq!(purchase["total"], "1999.00");

    // receipt upload (multipart) → appended to receipt_files
    let form = reqwest::multipart::Form::new().part(
        "file",
        reqwest::multipart::Part::bytes(b"fake-receipt-bytes".to_vec())
            .file_name("receipt.jpg")
            .mime_str("image/jpeg")
            .unwrap(),
    );
    let up: Value = c
        .post(format!("{base}/purchases/{purchase_id}/receipt"))
        .multipart(form)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(up["receipt_files"].as_array().unwrap().len(), 1);

    let got: Value = c
        .get(format!("{base}/purchases/{purchase_id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(got["receipt_files"].as_array().unwrap().len(), 1);
    assert_eq!(got["receipt_files"][0]["filename"], "receipt.jpg");

    // create a serialized unit → logs an intake event
    let unit: Value = c
        .post(format!("{base}/units"))
        .json(&json!({
            "product_id": product_id,
            "line_item_id": line_item_id,
            "serial_number": sn("GPU-2291X"),
            "serial_source": "scan",
            "condition": "new",
            "intake_by": "tester"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let unit_id = unit["id"].as_str().unwrap().to_string();
    assert_eq!(unit["status"], "in_stock");
    assert_eq!(unit["owner"], "shop");

    let events: Value = c
        .get(format!("{base}/units/{unit_id}/events"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(events.as_array().unwrap().len(), 1);
    assert_eq!(events[0]["event_type"], "intake");
    assert_eq!(events[0]["to_value"], "in_stock");

    // mutate status → logs a status_change event with from/to
    let patch = c
        .patch(format!("{base}/units/{unit_id}/status"))
        .json(&json!({ "status": "reserved", "actor": "tester", "note": "held for build" }))
        .send()
        .await
        .unwrap();
    assert!(patch.status().is_success());

    let events2: Value = c
        .get(format!("{base}/units/{unit_id}/events"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let arr = events2.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[1]["event_type"], "status_change");
    assert_eq!(arr[1]["from_value"], "in_stock");
    assert_eq!(arr[1]["to_value"], "reserved");

    // bulk stock + guarded adjustment
    let stock: Value = c
        .post(format!("{base}/stock"))
        .json(&json!({ "product_id": product_id, "location_bin": "A1", "quantity_on_hand": 100 }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let stock_id = stock["id"].as_str().unwrap().to_string();

    let adj: Value = c
        .post(format!("{base}/stock/{stock_id}/adjust"))
        .json(&json!({ "delta": -10 }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(adj["quantity_on_hand"], 90);

    // adjustment that would go negative is rejected
    let bad = c
        .post(format!("{base}/stock/{stock_id}/adjust"))
        .json(&json!({ "delta": -1000 }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 400);

    // unknown id → 404
    let nf = c
        .get(format!("{base}/units/{}", uuid::Uuid::new_v4()))
        .send()
        .await
        .unwrap();
    assert_eq!(nf.status(), 404);
}

#[tokio::test]
async fn phase1_landed_cost_and_shipment_tracking() {
    let Some(base) = spawn().await else { return };
    let c = reqwest::Client::new();

    let product: Value = c
        .post(format!("{base}/products"))
        .json(&json!({ "model": "PSU 850W", "category": "psu" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let product_id = product["id"].as_str().unwrap().to_string();

    // Purchase: shipping 50 + tax 100 across two lines (1000 qty4, 1500 qty1).
    let purchase: Value = c
        .post(format!("{base}/purchases"))
        .json(&json!({
            "source_type": "manual",
            "shipping": "50.00",
            "tax": "100.00",
            "line_items": [
                { "product_id": product_id, "quantity": 4, "line_total": "1000.00" },
                { "product_id": product_id, "quantity": 1, "line_total": "1500.00" }
            ]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let purchase_id = purchase["id"].as_str().unwrap().to_string();
    let line1 = purchase["line_items"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // A unit bound to line 1 should receive the per-unit landed cost on allocation.
    let unit: Value = c
        .post(format!("{base}/units"))
        .json(&json!({ "product_id": product_id, "line_item_id": line1, "serial_number": sn("PSU-1") }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let unit_id = unit["id"].as_str().unwrap().to_string();

    // Allocate landed cost.
    let alloc: Value = c
        .post(format!("{base}/purchases/{purchase_id}/allocate-costs"))
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(alloc["extra_total"], "150.00");
    // Lines come back ordered by id (random uuid), so match by line_total.
    let find = |total: &str| -> Value {
        alloc["lines"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["line_total"] == total)
            .cloned()
            .unwrap()
    };
    // line 1000 (qty 4): +60 = 1060, per-unit 265; line 1500 (qty 1): +90 = 1590.
    let l1000 = find("1000.00");
    let l1500 = find("1500.00");
    assert_eq!(l1000["allocated_landed_cost"], "1060.00");
    assert_eq!(l1000["per_unit_cost"], "265.00");
    assert_eq!(l1000["units_updated"], 1);
    assert_eq!(l1500["allocated_landed_cost"], "1590.00");

    // The bound unit now carries the landed per-unit cost, and a `note` event was logged.
    let unit2: Value = c
        .get(format!("{base}/units/{unit_id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(unit2["unit_cost"], "265.00");
    let uevents: Value = c
        .get(format!("{base}/units/{unit_id}/events"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(uevents
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["event_type"] == "note"));

    // Shipment capture + the poll engine (stepwise mock, exercised directly).
    let shipment: Value = c
        .post(format!("{base}/purchases/{purchase_id}/shipments"))
        .json(&json!({ "carrier": "ups", "tracking_number": "1Z999AA10123456784" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let shipment_id: uuid::Uuid = shipment["id"].as_str().unwrap().parse().unwrap();
    assert_eq!(shipment["poll_state"], "active");

    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect(&std::env::var("DATABASE_URL").unwrap())
        .await
        .unwrap();
    let provider = cec_inventory_tracking::MockProvider::stepwise();

    let statuses = ["pre_transit", "in_transit", "out_for_delivery", "delivered"];
    for (i, expected) in statuses.iter().enumerate() {
        let outcome = cec_inventory_tracking::poll_shipment(&pool, &provider, shipment_id)
            .await
            .unwrap();
        let got_status = serde_json::to_value(outcome.status).unwrap();
        assert_eq!(got_status, json!(expected), "poll {i}");
    }
    // A fifth poll is a no-op once delivered/stopped.
    let after = cec_inventory_tracking::poll_shipment(&pool, &provider, shipment_id)
        .await
        .unwrap();
    assert_eq!(after.new_events, 0);

    let got: Value = c
        .get(format!("{base}/shipments/{shipment_id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(got["status"], "delivered");
    assert_eq!(got["poll_state"], "stopped");
    assert_eq!(got["events"].as_array().unwrap().len(), 4);
    assert!(got["delivered_at"].is_string());
}

#[tokio::test]
async fn phase3_warranty_recompute_and_readiness() {
    let Some(base) = spawn().await else { return };
    let c = reqwest::Client::new();

    let mfr: Value = c
        .post(format!("{base}/manufacturers"))
        .json(&json!({ "name": "EVGA", "default_warranty_months": 36 }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let mfr_id = mfr["id"].as_str().unwrap().to_string();

    let product: Value = c
        .post(format!("{base}/products"))
        .json(&json!({ "model": "GPU", "category": "gpu", "manufacturer_id": mfr_id }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let product_id = product["id"].as_str().unwrap().to_string();

    // CEC full-class policy: 12 months.
    c.post(format!("{base}/warranty-policies"))
        .json(&json!({ "warranty_class": "full", "term_months": 12, "transferable": true }))
        .send()
        .await
        .unwrap();

    // Purchase with a known datetime → mfr clock start.
    let purchase: Value = c
        .post(format!("{base}/purchases"))
        .json(&json!({
            "source_type": "manual",
            "purchase_datetime": "2026-01-01T00:00:00Z",
            "line_items": [{ "product_id": product_id, "quantity": 1, "line_total": "999.00" }]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let line_id = purchase["line_items"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let unit: Value = c
        .post(format!("{base}/units"))
        .json(&json!({ "product_id": product_id, "line_item_id": line_id, "serial_number": sn("GPU-W1") }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let unit_id = unit["id"].as_str().unwrap().to_string();

    // Recompute: mfr term 36 months from 2026-01-01 → 2029-01-01; rma eligible (serial +
    // resolved + CEC receipt + in-warranty). CEC class is still `none` until delivery.
    let w: Value = c
        .post(format!("{base}/units/{unit_id}/recompute-warranty"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(w["mfr_warranty_expires"], "2029-01-01");
    assert_eq!(w["rma_eligible"], true);
    assert_eq!(w["cec_warranty_class"], "none");
    assert_eq!(w["cec_warranty_active"], false);

    // A unit with no serial blocks on `no_serial`.
    let bare: Value = c
        .post(format!("{base}/units"))
        .json(&json!({ "product_id": product_id, "line_item_id": line_id }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let bare_id = bare["id"].as_str().unwrap().to_string();
    let bw: Value = c
        .post(format!("{base}/units/{bare_id}/recompute-warranty"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(bw["rma_eligible"], false);
    assert_eq!(bw["rma_block_reason"], "no_serial");
}

#[tokio::test]
async fn phase2_trade_in_and_opening_balance() {
    let Some(base) = spawn().await else { return };
    let c = reqwest::Client::new();
    let product: Value = c
        .post(format!("{base}/products"))
        .json(&json!({ "model": "Used SSD", "category": "storage" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let product_id = product["id"].as_str().unwrap().to_string();

    // Trade-in with no proof → unit not RMA-able, reason recorded, owner shop.
    let ti: Value = c
        .post(format!("{base}/trade-ins"))
        .json(&json!({
            "customer_ref": "cust-1",
            "proof_of_purchase_status": "customer_lacks",
            "units": [{ "product_id": product_id, "serial_number": sn("SSD-T1"), "condition": "used" }]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ti_unit = ti["unit_ids"][0].as_str().unwrap().to_string();
    let u: Value = c
        .get(format!("{base}/units/{ti_unit}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(u["owner"], "shop");
    assert_eq!(u["acquisition_method"], "trade_in");
    assert_eq!(u["rma_eligible"], false);
    assert_eq!(u["rma_block_reason"], "no_proof_of_purchase");

    // Opening-balance, unknown origin → synthetic purchase, units not RMA-able.
    let ob: Value = c
        .post(format!("{base}/opening-balance"))
        .json(&json!({
            "origin_known": false,
            "units": [{ "product_id": product_id, "serial_number": sn("SSD-OB1") }]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(ob["purchase_id"].is_string());
    let ob_unit = ob["unit_ids"][0].as_str().unwrap().to_string();
    let ou: Value = c
        .get(format!("{base}/units/{ob_unit}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(ou["acquisition_method"], "opening_balance");
    assert_eq!(ou["rma_block_reason"], "no_proof_of_purchase");

    // The opening-balance purchase is a synthetic source_type=opening_balance.
    let pur: Value = c
        .get(format!(
            "{base}/purchases/{}",
            ob["purchase_id"].as_str().unwrap()
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(pur["source_type"], "opening_balance");
}

#[tokio::test]
async fn phase3_systems_delivery_starts_cec_clock() {
    let Some(base) = spawn().await else { return };
    let c = reqwest::Client::new();
    c.post(format!("{base}/warranty-policies"))
        .json(&json!({ "warranty_class": "full", "term_months": 12 }))
        .send()
        .await
        .unwrap();
    let product: Value = c
        .post(format!("{base}/products"))
        .json(&json!({ "model": "Board X", "category": "motherboard" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let product_id = product["id"].as_str().unwrap().to_string();
    let unit: Value = c
        .post(format!("{base}/units"))
        .json(&json!({ "product_id": product_id, "serial_number": sn("BRD-1") }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let unit_id = unit["id"].as_str().unwrap().to_string();

    let system: Value = c
        .post(format!("{base}/systems"))
        .json(&json!({ "label": "BUILD-001", "cec_warranty_class": "full" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let system_id = system["id"].as_str().unwrap().to_string();

    c.post(format!("{base}/systems/{system_id}/members"))
        .json(&json!({ "unit_id": unit_id }))
        .send()
        .await
        .unwrap();

    // Delivery before validation is rejected.
    let early = c
        .post(format!("{base}/systems/{system_id}/deliver"))
        .json(&json!({ "customer_ref": "cust-9" }))
        .send()
        .await
        .unwrap();
    assert_eq!(early.status(), 400);

    // Validate (EOL pass) → validated.
    let v: Value = c
        .post(format!("{base}/systems/{system_id}/validate"))
        .json(&json!({ "validation_type": "eol", "result": "pass", "performed_by": "bench" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["validation_state"], "validated");

    // Deliver → unit flips to customer and the CEC clock starts.
    let d: Value = c
        .post(format!("{base}/systems/{system_id}/deliver"))
        .json(&json!({ "customer_ref": "cust-9", "performed_by": "front-desk" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(d["units_delivered"], 1);

    let u: Value = c
        .get(format!("{base}/units/{unit_id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(u["owner"], "customer");
    assert_eq!(u["status"], "with_customer");
    assert_eq!(u["cec_warranty_class"], "full");
    assert!(u["cec_warranty_expires"].is_string());

    let events: Value = c
        .get(format!("{base}/units/{unit_id}/events"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let types: Vec<&str> = events
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["event_type"].as_str().unwrap())
        .collect();
    assert!(types.contains(&"deliver"));
    assert!(types.contains(&"owner_change"));
}

#[tokio::test]
async fn phase4_parts_sweep_and_transfer() {
    let Some(base) = spawn().await else { return };
    let c = reqwest::Client::new();
    c.post(format!("{base}/warranty-policies"))
        .json(&json!({ "warranty_class": "full", "term_months": 12 }))
        .send()
        .await
        .unwrap();
    let product: Value = c
        .post(format!("{base}/products"))
        .json(&json!({ "model": "CPU Y", "category": "cpu" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let product_id = product["id"].as_str().unwrap().to_string();
    let serial = sn("SW-1");
    let unit: Value = c
        .post(format!("{base}/units"))
        .json(&json!({ "product_id": product_id, "serial_number": serial.clone() }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let unit_id = unit["id"].as_str().unwrap().to_string();
    let system: Value = c
        .post(format!("{base}/systems"))
        .json(&json!({ "label": "SWP-1", "cec_warranty_class": "full" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let sid = system["id"].as_str().unwrap().to_string();
    c.post(format!("{base}/systems/{sid}/members"))
        .json(&json!({ "unit_id": unit_id }))
        .send()
        .await
        .unwrap();
    c.post(format!("{base}/systems/{sid}/validate"))
        .json(&json!({ "validation_type": "eol", "result": "pass" }))
        .send()
        .await
        .unwrap();
    c.post(format!("{base}/systems/{sid}/deliver"))
        .json(&json!({ "customer_ref": "cust-A" }))
        .send()
        .await
        .unwrap();

    // Discrepancy sweep (wrong serial) → invalidated; transfer then blocked.
    let bad: Value = c
        .post(format!("{base}/systems/{sid}/sweep"))
        .json(&json!({ "scanned_serials": ["NOPE"] }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(bad["overall"], "discrepancies");
    assert_eq!(bad["validation_state"], "invalidated");
    let blocked = c
        .post(format!("{base}/systems/{sid}/transfer"))
        .json(&json!({ "to_owner_ref": "cust-B" }))
        .send()
        .await
        .unwrap();
    assert_eq!(blocked.status(), 400);

    // Clean sweep → validated; transfer authorized by the sweep.
    let good: Value = c
        .post(format!("{base}/systems/{sid}/sweep"))
        .json(&json!({ "scanned_serials": [serial.clone()], "performed_by": "bench" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(good["overall"], "clean");
    let sweep_id = good["validation_id"].as_str().unwrap().to_string();

    let tr: Value = c
        .post(format!("{base}/systems/{sid}/transfer"))
        .json(&json!({ "to_owner_ref": "cust-B", "sweep_id": sweep_id, "performed_by": "desk" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(tr["result"], "completed");
    assert_eq!(tr["from_owner_ref"], "cust-A");
    assert_eq!(tr["mfr_warranty_outcome"][0]["outcome"], "carried");

    let u: Value = c
        .get(format!("{base}/units/{unit_id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(u["customer_ref"], "cust-B");
    let events: Value = c
        .get(format!("{base}/units/{unit_id}/events"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(events
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["event_type"] == "transfer"));
}

#[tokio::test]
async fn phase3_rma_lifecycle_and_replacement() {
    let Some(base) = spawn().await else { return };
    let c = reqwest::Client::new();
    let mfr: Value = c
        .post(format!("{base}/manufacturers"))
        .json(&json!({ "name": "Corsair", "default_warranty_months": 60 }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let product: Value = c
        .post(format!("{base}/products"))
        .json(&json!({ "model": "PSU RMx", "category": "psu", "manufacturer_id": mfr["id"] }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let product_id = product["id"].as_str().unwrap().to_string();
    let purchase: Value = c
        .post(format!("{base}/purchases"))
        .json(&json!({
            "source_type": "manual",
            "purchase_datetime": "2026-02-01T00:00:00Z",
            "line_items": [{ "product_id": product_id, "quantity": 1, "line_total": "150.00" }]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let line_id = purchase["line_items"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let serial = sn("PSU-RMA-1");
    let unit: Value = c
        .post(format!("{base}/units"))
        .json(&json!({ "product_id": product_id, "line_item_id": line_id, "serial_number": serial.clone() }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let unit_id = unit["id"].as_str().unwrap().to_string();

    // Open RMA: shop-owned + CEC receipt → cec_managed, proof cec_receipt.
    let case: Value = c
        .post(format!("{base}/units/{unit_id}/rma"))
        .json(&json!({ "fault_description": "no power", "party": "manufacturer" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let case_id = case["id"].as_str().unwrap().to_string();
    assert_eq!(case["status"], "open");
    assert_eq!(case["proof_source"], "cec_receipt");
    assert_eq!(case["execution_mode"], "cec_managed");
    let ucheck: Value = c
        .get(format!("{base}/units/{unit_id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(ucheck["status"], "rma_open");

    // Proof-of-purchase package.
    let pkg: Value = c
        .post(format!("{base}/rma/{case_id}/proof-package"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(pkg["serial_number"], serial);
    assert_eq!(pkg["product"]["manufacturer"], "Corsair");

    // Refurbished replacement intake.
    let rep: Value = c
        .post(format!("{base}/rma/{case_id}/replacement"))
        .json(&json!({ "serial_number": sn("PSU-RMA-2"), "refurbished": true }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let new_unit = rep["replacement_unit_id"].as_str().unwrap().to_string();
    let nu: Value = c
        .get(format!("{base}/units/{new_unit}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(nu["acquisition_method"], "rma_replacement");
    assert_eq!(nu["condition"], "refurb");
    assert_eq!(nu["cec_warranty_class"], "refurb");

    // Predecessor retired; case advanced.
    let old: Value = c
        .get(format!("{base}/units/{unit_id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(old["status"], "returned");
    let cc: Value = c
        .get(format!("{base}/rma/{case_id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(cc["status"], "replacement_received");
    assert_eq!(cc["replacement_unit_id"], new_unit);
}

#[tokio::test]
async fn phase5_direct_reserve_consume() {
    let Some(base) = spawn().await else { return };
    let c = reqwest::Client::new();
    let product: Value = c
        .post(format!("{base}/products"))
        .json(&json!({ "model": "RAM Kit", "category": "memory" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let product_id = product["id"].as_str().unwrap().to_string();
    let unit: Value = c
        .post(format!("{base}/units"))
        .json(&json!({ "product_id": product_id, "serial_number": sn("RAM-1") }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let unit_id = unit["id"].as_str().unwrap().to_string();
    let system: Value = c
        .post(format!("{base}/systems"))
        .json(&json!({ "label": "BUILD-DIRECT", "build_id": uuid::Uuid::new_v4() }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let system_id = system["id"].as_str().unwrap().to_string();

    // Reserve in_stock → reserved.
    let r = c
        .post(format!("{base}/units/{unit_id}/reserve"))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success());

    // Consume reserved → installed, attached to the system.
    let cons: Value = c
        .post(format!("{base}/units/{unit_id}/consume"))
        .json(&json!({ "system_id": system_id }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(cons["status"], "installed");
    let u: Value = c
        .get(format!("{base}/units/{unit_id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(u["status"], "installed");
    assert_eq!(u["system_id"], system_id);

    // Reserving a non-in-stock unit is rejected.
    let bad = c
        .post(format!("{base}/units/{unit_id}/reserve"))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 400);

    // Availability read returns serialized + bulk arrays.
    let avail: Value = c
        .get(format!("{base}/availability"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(avail["serialized"].is_array());
    assert!(avail["bulk"].is_array());
}

#[tokio::test]
async fn phase2_verify_and_asset_tags() {
    let Some(base) = spawn().await else { return };
    let c = reqwest::Client::new();
    let product: Value = c
        .post(format!("{base}/products"))
        .json(&json!({ "model": "GPU V", "category": "gpu", "serial_format_regex": r"^GPU-\d{4}[A-Z]$" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let product_id = product["id"].as_str().unwrap().to_string();

    let unit: Value = c
        .post(format!("{base}/units"))
        .json(&json!({ "product_id": product_id }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let unit_id = unit["id"].as_str().unwrap().to_string();
    let v: Value = c
        .post(format!("{base}/units/{unit_id}/verify"))
        .json(&json!({ "scanned_serial": "GPU-1234A" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["bound_from_scan"], true);
    assert_eq!(v["verified"], true);
    assert_eq!(v["format_valid"], true);

    let v2: Value = c
        .post(format!("{base}/units/{unit_id}/verify"))
        .json(&json!({ "scanned_serial": "GPU-1234A" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v2["matched"], true);
    let v3: Value = c
        .post(format!("{base}/units/{unit_id}/verify"))
        .json(&json!({ "scanned_serial": "WRONG" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v3["matched"], false);
    assert_eq!(v3["format_valid"], false);
    assert!(!v3["warnings"].as_array().unwrap().is_empty());

    let t1: Value = c
        .post(format!("{base}/units/{unit_id}/asset-tag"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let tag = t1["asset_tag"].as_str().unwrap().to_string();
    assert!(tag.starts_with("CEC-U-"));
    assert!(t1["zpl"].as_str().unwrap().contains(&tag));
    let t2: Value = c
        .post(format!("{base}/units/{unit_id}/asset-tag"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(t2["asset_tag"], tag);
}

#[tokio::test]
async fn phase1_identity_resolution_and_bundle_expansion() {
    let Some(base) = spawn().await else { return };
    let c = reqwest::Client::new();
    let mk_product = |model: &str| {
        let c = &c;
        let base = &base;
        let model = model.to_string();
        async move {
            let p: Value = c
                .post(format!("{base}/products"))
                .json(&json!({ "model": model }))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            p["id"].as_str().unwrap().to_string()
        }
    };
    let cpu = mk_product("CPU combo").await;
    let board = mk_product("Board combo").await;
    let placeholder = mk_product("placeholder").await;

    // A combo line at a combined price, initially unresolved.
    let purchase: Value = c
        .post(format!("{base}/purchases"))
        .json(&json!({
            "source_type": "manual",
            "line_items": [{ "description_as_printed": "CPU + Board combo", "quantity": 1, "line_total": "500.00" }]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let combo_line = purchase["line_items"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Expand MSRP-weighted (300/200 of 500).
    let exp: Value = c
        .post(format!("{base}/line-items/{combo_line}/expand"))
        .json(&json!({
            "components": [
                { "product_id": cpu, "msrp": "300.00" },
                { "product_id": board, "msrp": "200.00" }
            ]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(exp["allocation"], "msrp");
    let children = exp["children"].as_array().unwrap();
    assert_eq!(children.len(), 2);
    let total: f64 = children
        .iter()
        .map(|ch| ch["line_total"].as_str().unwrap().parse::<f64>().unwrap())
        .sum();
    assert_eq!(total, 500.0);
    let cpu_child = children.iter().find(|ch| ch["product_id"] == cpu).unwrap();
    assert_eq!(cpu_child["line_total"], "300.00");

    // Parent is now flagged as a bundle.
    let got: Value = c
        .get(format!(
            "{base}/purchases/{}",
            purchase["id"].as_str().unwrap()
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let parent = got["line_items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["id"] == combo_line.as_str())
        .unwrap();
    assert_eq!(parent["is_bundle"], true);

    // Identity resolution on a fresh line.
    let li: Value = c
        .post(format!(
            "{base}/purchases/{}/line-items",
            purchase["id"].as_str().unwrap()
        ))
        .json(&json!({ "description_as_printed": "mystery", "quantity": 1 }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let resolved: Value = c
        .post(format!(
            "{base}/line-items/{}/resolve",
            li["id"].as_str().unwrap()
        ))
        .json(&json!({ "product_id": placeholder }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resolved["product_id"], placeholder);
    assert_eq!(resolved["resolution_status"], "confirmed");
}

#[tokio::test]
async fn crosscutting_reorder_reconciliation_export() {
    let Some(base) = spawn().await else { return };
    let c = reqwest::Client::new();
    let product: Value = c
        .post(format!("{base}/products"))
        .json(&json!({ "model": "Cable" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let product_id = product["id"].as_str().unwrap().to_string();

    // Stock below reorder point shows on the reorder worklist.
    let low: Value = c
        .post(format!("{base}/stock"))
        .json(&json!({ "product_id": product_id, "quantity_on_hand": 2, "reorder_point": 5 }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let low_id = low["id"].as_str().unwrap().to_string();
    let reorder: Value = c
        .get(format!("{base}/reorder"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(reorder
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["stock_id"] == low_id.as_str()));

    // A delivered shipment with no intaked units → receiving reconciliation flags it.
    let purchase: Value = c
        .post(format!("{base}/purchases"))
        .json(&json!({ "source_type": "manual" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let purchase_id = purchase["id"].as_str().unwrap().to_string();
    let shipment: Value = c
        .post(format!("{base}/purchases/{purchase_id}/shipments"))
        .json(&json!({ "carrier": "fedex", "tracking_number": "F123" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let shipment_id: uuid::Uuid = shipment["id"].as_str().unwrap().parse().unwrap();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect(&std::env::var("DATABASE_URL").unwrap())
        .await
        .unwrap();
    cec_inventory_tracking::poll_shipment(
        &pool,
        &cec_inventory_tracking::MockProvider::full(),
        shipment_id,
    )
    .await
    .unwrap();
    let recon: Value = c
        .get(format!("{base}/receiving/reconciliation"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(recon["delivered_not_received"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["shipment_id"] == shipment_id.to_string()));

    // Full JSON export + units CSV.
    let exp: Value = c
        .get(format!("{base}/export"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(exp["units"].is_array());
    assert!(exp["purchases"].is_array());
    let csv = c
        .get(format!("{base}/export/units.csv"))
        .send()
        .await
        .unwrap();
    assert!(csv
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .contains("text/csv"));
    let body = csv.text().await.unwrap();
    assert!(body.starts_with("id,serial_number,product_id,status"));
}

#[tokio::test]
async fn phase1_extract_preview_502_when_unreachable() {
    let Some(base) = spawn().await else { return };
    // Point the extractor seam at an unreachable address; the proxy should surface a 502.
    std::env::set_var("EXTRACTOR_URL", "http://127.0.0.1:9");
    let c = reqwest::Client::new();
    let resp = c
        .post(format!("{base}/extract-preview"))
        .json(&json!({ "text": "Micro Center\n1 x RTX 4090 $1599.00 $1599.00" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 502);
}

#[tokio::test]
async fn ui_pages_render() {
    let Some(base) = spawn().await else { return };
    let c = reqwest::Client::new();

    let dash = c.get(format!("{base}/")).send().await.unwrap();
    assert!(dash
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .contains("text/html"));
    let html = dash.text().await.unwrap();
    assert!(html.contains("CEC Inventory"));
    assert!(html.contains("Dashboard"));

    for (path, marker) in [
        ("/ui/units", "<h1>Units</h1>"),
        ("/ui/systems", "<h1>Systems</h1>"),
        ("/ui/purchases", "<h1>Purchases</h1>"),
    ] {
        let body = c
            .get(format!("{base}{path}"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(body.contains(marker), "{path} missing {marker}");
    }

    // The new public form pages render (login is robust to app_user count: both the
    // first-run and the returning-login branch carry a password field).
    for (path, marker) in [
        ("/ui/login", "name=\"password\""),
        ("/ui/new", "Serialized unit"),
        ("/ui/purchases/new", "Line items"),
    ] {
        let body = c
            .get(format!("{base}{path}"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(body.contains(marker), "{path} missing {marker}");
    }

    // Detail pages render against real rows — this exercises the detail joins/enum casts
    // against the live schema, not just static HTML. (spawn() has auth off, so the JSON
    // mutations need no cookie.)
    let prod: Value = c
        .post(format!("{base}/products"))
        .json(&json!({ "model": "UI-Detail-Probe" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let pid = prod["id"].as_str().unwrap();
    let unit: Value = c
        .post(format!("{base}/units"))
        .json(&json!({ "product_id": pid, "serial_number": sn("UIDET-1") }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let uid2 = unit["id"].as_str().unwrap();
    let ud = c
        .get(format!("{base}/ui/units/{uid2}"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(ud.contains("Change status"), "unit detail missing actions");
    assert!(
        ud.contains("Event timeline"),
        "unit detail missing timeline"
    );
    assert!(ud.contains("UIDET-1"), "unit detail missing the serial");

    let sys: Value = c
        .post(format!("{base}/systems"))
        .json(&json!({ "label": "UI-Detail-System" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let sid = sys["id"].as_str().unwrap();
    let sd = c
        .get(format!("{base}/ui/systems/{sid}"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(sd.contains("Add member"), "system detail missing actions");
    assert!(sd.contains("Parts sweep"), "system detail missing sweep");
    assert!(
        sd.contains("Deliver to customer"),
        "system detail missing deliver"
    );

    // The scan island embeds the unit id and the BarcodeDetector path.
    let uid = uuid::Uuid::new_v4();
    let scan = c
        .get(format!("{base}/ui/scan/{uid}"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(scan.contains("BarcodeDetector"));
    assert!(scan.contains(&uid.to_string()));
}

// Runs only when EXTRACTOR_TEST_URL points at a live extractor service (set locally with
// uvicorn). Skipped in CI, which has no extractor.
#[tokio::test]
async fn phase1_create_from_extraction() {
    let Ok(ext) = std::env::var("EXTRACTOR_TEST_URL") else {
        eprintln!("skipping: EXTRACTOR_TEST_URL not set");
        return;
    };
    std::env::set_var("EXTRACTOR_URL", &ext);
    let Some(base) = spawn().await else { return };
    let c = reqwest::Client::new();

    let text = "Micro Center\nOrder # MC-9\n2026-03-02 14:31\n\
1 x GeForce RTX 4090  SKU:GPU1  SN:GPU-2291X  $1599.00  $1599.00\n\
2 x HDMI Cable  SKU:CAB9  $9.99  $19.98\n\
Subtotal $1618.98\nTax $133.57\nTotal $1752.55\n";

    let r: Value = c
        .post(format!("{base}/purchases/from-extraction"))
        .json(&json!({ "text": text, "created_by": "op1" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(r["engine"], "template");
    assert_eq!(r["line_item_count"], 2);
    assert_eq!(r["needs_resolution"], true);

    let pur: Value = c
        .get(format!(
            "{base}/purchases/{}",
            r["purchase_id"].as_str().unwrap()
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(pur["order_number"], "MC-9");
    assert_eq!(pur["total"], "1752.55");
    let lines = pur["line_items"].as_array().unwrap();
    assert_eq!(lines.len(), 2);
    // Lines come back unresolved for the operator to map + scan into units.
    assert!(lines.iter().all(|l| l["resolution_status"] == "unresolved"));
}

// `POST /purchases/from-payload` persists a caller-supplied §11.4 extraction payload (the
// interim seam an external/operator/agent vision pass uses) — no extractor service needed, so
// this runs in CI. Mirrors what the Claude-vision backend would emit for a photographed receipt.
#[tokio::test]
async fn from_payload_persists_supplied_extraction() {
    let Some(base) = spawn().await else { return };
    let c = reqwest::Client::new();

    let payload = json!({
        "extraction": {
            "vendor": "Photographed Vendor",
            "purchase_datetime": "2026-05-04T09:15:00",
            "order_number": "IMG-42",
            "currency": "USD",
            "engine": "vlm_claude",
            "line_items": [
                {"description": "RTX 4070", "vendor_sku": "GPU7", "quantity": 1,
                 "unit_price": 599.00, "line_total": 599.00, "serials": ["SNIMG-1"], "is_bundle": false},
                {"description": "Case fan", "quantity": 3, "unit_price": 12.0, "line_total": 36.0}
            ],
            "subtotal": 635.00, "tax": 52.00, "total": 687.00,
            "field_confidence": {"vendor": 0.6, "total": 0.6, "datetime": 0.6}
        },
        "source_type": "physical_photo",
        "created_by": "vision-op"
    });

    let r: Value = c
        .post(format!("{base}/purchases/from-payload"))
        .json(&payload)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(r["engine"], "vlm_claude");
    assert_eq!(r["line_item_count"], 2);
    assert_eq!(r["needs_resolution"], true);

    let pur: Value = c
        .get(format!(
            "{base}/purchases/{}",
            r["purchase_id"].as_str().unwrap()
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(pur["order_number"], "IMG-42");
    assert_eq!(pur["total"], "687.00");
    assert_eq!(pur["source_type"], "physical_photo");
    let lines = pur["line_items"].as_array().unwrap();
    assert_eq!(lines.len(), 2);
    assert!(lines.iter().all(|l| l["resolution_status"] == "unresolved"));

    // A non-object payload is a 400, not a 500.
    let bad = c
        .post(format!("{base}/purchases/from-payload"))
        .json(&json!({ "extraction": "not-an-object" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 400);
}

// Serial numbers are globally unique (migration 0003): a second unit with the same serial is
// rejected with 409 (the central unique-violation mapping), not silently duplicated.
#[tokio::test]
async fn serial_number_globally_unique() {
    let Some(base) = spawn().await else { return };
    let c = reqwest::Client::new();
    let product: Value = c
        .post(format!("{base}/products"))
        .json(&json!({ "model": "Unique-Serial Probe" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let pid = product["id"].as_str().unwrap();
    let serial = sn("DUP");

    let first = c
        .post(format!("{base}/units"))
        .json(&json!({ "product_id": pid, "serial_number": serial.clone() }))
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), 201);

    let dup = c
        .post(format!("{base}/units"))
        .json(&json!({ "product_id": pid, "serial_number": serial.clone() }))
        .send()
        .await
        .unwrap();
    assert_eq!(dup.status(), 409, "duplicate serial must conflict");

    // A NULL serial is exempt from the partial unique index — two are fine.
    for _ in 0..2 {
        let r = c
            .post(format!("{base}/units"))
            .json(&json!({ "product_id": pid }))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 201, "null-serial units are allowed");
    }
}

#[tokio::test]
async fn auth_bootstrap_login_protect_logout() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        return;
    };
    // spawn_authed() runs migrations (creating app_user); then clear accounts for a
    // deterministic bootstrap.
    let Some(base) = spawn_authed().await else {
        return;
    };
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect(&url)
        .await
        .unwrap();
    sqlx::query("DELETE FROM app_user")
        .execute(&pool)
        .await
        .unwrap();

    // Unauthenticated access to a protected route is rejected.
    let anon = reqwest::Client::new();
    let r = anon.get(format!("{base}/units")).send().await.unwrap();
    assert_eq!(r.status(), 401);

    // Bootstrap the first operator.
    let boot = anon
        .post(format!("{base}/auth/bootstrap"))
        .json(&json!({ "username": "op1", "password": "supersecret-12" }))
        .send()
        .await
        .unwrap();
    assert!(boot.status().is_success(), "bootstrap: {}", boot.status());

    // A second bootstrap is refused.
    let boot2 = anon
        .post(format!("{base}/auth/bootstrap"))
        .json(&json!({ "username": "op2", "password": "supersecret-12" }))
        .send()
        .await
        .unwrap();
    assert_eq!(boot2.status(), 400);

    // Wrong password → 401.
    let bad = anon
        .post(format!("{base}/auth/login"))
        .json(&json!({ "username": "op1", "password": "nope" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 401);

    // Login with a cookie-storing client, then the session unlocks protected routes.
    let c = reqwest::Client::builder()
        .cookie_store(true)
        .build()
        .unwrap();
    let login = c
        .post(format!("{base}/auth/login"))
        .json(&json!({ "username": "op1", "password": "supersecret-12" }))
        .send()
        .await
        .unwrap();
    assert!(login.status().is_success());

    let me: Value = c
        .get(format!("{base}/auth/me"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(me["username"], "op1");

    let units = c.get(format!("{base}/units")).send().await.unwrap();
    assert!(
        units.status().is_success(),
        "authed units status {}",
        units.status()
    );

    // Logout clears the session.
    c.post(format!("{base}/auth/logout")).send().await.unwrap();
    let after = c.get(format!("{base}/units")).send().await.unwrap();
    assert_eq!(after.status(), 401);
}
