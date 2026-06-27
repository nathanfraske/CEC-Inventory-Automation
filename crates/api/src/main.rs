// Thin binary entry point. All wiring lives in the library (src/lib.rs) so the
// integration tests can build and drive the same app. axum::serve exists from 0.7 on.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    cec_inventory_api::run().await
}
