//! CEC Inventory API. Phase 0: the health spine plus manual entry (purchases, units,
//! bulk stock), receipt-file upload to the object store, and a `unit_event` row written
//! on every unit mutation (scope Sections 4 and 16).
//!
//! Data access uses SQLx runtime queries (no compile-time `query!` macros yet), so the
//! workspace builds offline with no `DATABASE_URL` and CI needs no `.sqlx/` cache. The
//! move to compile-time-checked queries + a committed `.sqlx/` is a Phase 1 follow-up
//! (see docs/DECISIONS.md D-001).

use std::{path::PathBuf, sync::Arc};

use axum::{routing::get, Json, Router};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

pub mod costing;
pub mod error;
pub mod events;
pub mod routes;

/// Shared application state handed to every handler.
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    /// Filesystem root for the object store (receipts, later unit photos). Default
    /// `STORAGE_BACKEND=fs`; MinIO is a drop-in later (scope INV-OQ-2).
    pub storage_root: Arc<PathBuf>,
}

/// Build state from the environment: connect the pool, run migrations, ensure the
/// object-store root exists. `DATABASE_URL` lives only in the gitignored `.env`.
pub async fn build_state() -> anyhow::Result<AppState> {
    let database_url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set (see .env, never commit it)");
    let storage_root: PathBuf = std::env::var("STORAGE_FS_ROOT")
        .unwrap_or_else(|_| "data/objects".to_string())
        .into();
    std::fs::create_dir_all(&storage_root)?;

    let db = PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await?;
    // migrations/ lives at the repo root, two levels above this crate.
    sqlx::migrate!("../../migrations").run(&db).await?;

    Ok(AppState {
        db,
        storage_root: Arc::new(storage_root),
    })
}

/// Assemble the full router: the health spine plus the Phase 0 resource routes.
pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/readyz", get(readyz))
        .merge(routes::router())
        .with_state(state)
}

/// Connect, build the router, and serve. The binary entry point calls this.
pub async fn run() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let bind = std::env::var("API_BIND").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    let state = build_state().await?;
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!("listening on {bind}");
    axum::serve(listener, app(state)).await?;
    Ok(())
}

async fn readyz(
    axum::extract::State(s): axum::extract::State<AppState>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&s.db)
        .await
        .map(|_| Json(serde_json::json!({ "db": "up" })))
        .map_err(|e| (axum::http::StatusCode::SERVICE_UNAVAILABLE, e.to_string()))
}
