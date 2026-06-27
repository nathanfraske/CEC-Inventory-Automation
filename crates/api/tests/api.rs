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
