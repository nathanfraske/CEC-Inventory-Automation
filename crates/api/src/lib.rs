//! CEC Inventory API. Phase 0: the health spine plus manual entry (purchases, units,
//! bulk stock), receipt-file upload to the object store, and a `unit_event` row written
//! on every unit mutation (scope Sections 4 and 16).
//!
//! Data access uses SQLx runtime queries (no compile-time `query!` macros yet), so the
//! workspace builds offline with no `DATABASE_URL` and CI needs no `.sqlx/` cache. The
//! move to compile-time-checked queries + a committed `.sqlx/` is a Phase 1 follow-up
//! (see docs/DECISIONS.md D-001).

use std::{path::PathBuf, sync::Arc};

use axum::extract::FromRef;
use axum::{
    routing::{get, post},
    Json, Router,
};
use axum_extra::extract::cookie::Key;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

pub mod auth;
pub mod costing;
pub mod error;
pub mod events;
pub mod extractor;
pub mod routes;
pub mod warranty;

/// Shared application state handed to every handler.
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    /// Filesystem root for the object store (receipts, later unit photos). Default
    /// `STORAGE_BACKEND=fs`; MinIO is a drop-in later (scope INV-OQ-2).
    pub storage_root: Arc<PathBuf>,
    /// Signing key for session cookies, derived from `SESSION_SECRET`.
    pub cookie_key: Key,
}

// Lets the `SignedCookieJar` extractor pull the signing key out of the app state.
impl FromRef<AppState> for Key {
    fn from_ref(state: &AppState) -> Self {
        state.cookie_key.clone()
    }
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

    // Derive the cookie signing key from SESSION_SECRET (≥64 bytes; pad short dev secrets).
    let mut secret = std::env::var("SESSION_SECRET")
        .unwrap_or_else(|_| "dev-insecure-session-secret-change-me".to_string())
        .into_bytes();
    if secret.len() < 64 {
        secret.resize(64, 0);
    }
    let cookie_key = Key::from(&secret);

    Ok(AppState {
        db,
        storage_root: Arc::new(storage_root),
        cookie_key,
    })
}

/// Assemble the full app with auth enabled (production). See `build_app`.
pub fn app(state: AppState) -> Router {
    build_app(state, true)
}

/// Assemble the router: a public layer (health, auth, read-only UI) plus the data and
/// mutation routes, optionally wrapped in the auth middleware (scope §18). `require_auth`
/// is `false` only for the non-auth integration tests.
pub fn build_app(state: AppState, require_auth: bool) -> Router {
    let mut protected = routes::router().route("/auth/users", post(auth::create_user));
    if require_auth {
        protected = protected.route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_auth,
        ));
    }

    let public = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/readyz", get(readyz))
        .merge(routes::ui_router())
        .merge(auth::router());

    public.merge(protected).with_state(state)
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
