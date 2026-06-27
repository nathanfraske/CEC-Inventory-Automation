//! End-to-end Phase 0 flow against a live Postgres. Skipped automatically when
//! `DATABASE_URL` is unset (e.g. CI, which builds DB-free). Run locally with a DB up:
//!   set -a; . ./.env; set +a; cargo test -p cec-inventory-api

use serde_json::{json, Value};

async fn spawn() -> Option<String> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("skipping integration test: DATABASE_URL not set");
        return None;
    }
    // Isolated object-store root per run, outside the repo.
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
            "serial_number": "GPU-2291X",
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
        .json(&json!({ "product_id": product_id, "line_item_id": line1, "serial_number": "PSU-1" }))
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
        .json(&json!({ "product_id": product_id, "line_item_id": line_id, "serial_number": "GPU-W1" }))
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
            "units": [{ "product_id": product_id, "serial_number": "SSD-T1", "condition": "used" }]
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
            "units": [{ "product_id": product_id, "serial_number": "SSD-OB1" }]
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
        .json(&json!({ "product_id": product_id, "serial_number": "BRD-1" }))
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
    let unit: Value = c
        .post(format!("{base}/units"))
        .json(&json!({ "product_id": product_id, "serial_number": "SW-1" }))
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
        .json(&json!({ "scanned_serials": ["SW-1"], "performed_by": "bench" }))
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
    let unit: Value = c
        .post(format!("{base}/units"))
        .json(&json!({ "product_id": product_id, "line_item_id": line_id, "serial_number": "PSU-RMA-1" }))
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
    assert_eq!(pkg["serial_number"], "PSU-RMA-1");
    assert_eq!(pkg["product"]["manufacturer"], "Corsair");

    // Refurbished replacement intake.
    let rep: Value = c
        .post(format!("{base}/rma/{case_id}/replacement"))
        .json(&json!({ "serial_number": "PSU-RMA-2", "refurbished": true }))
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
        .json(&json!({ "product_id": product_id, "serial_number": "RAM-1" }))
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
