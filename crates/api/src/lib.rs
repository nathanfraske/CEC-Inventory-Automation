//! CEC Inventory API. Phase 0: the health spine plus manual entry (purchases, units,
//! bulk stock), receipt-file upload to the object store, and a `unit_event` row written
//! on every unit mutation (scope Sections 4 and 16).
//!
//! Data access uses SQLx runtime queries (no compile-time `query!` macros yet), so the
//! workspace builds offline with no `DATABASE_URL` and CI needs no `.sqlx/` cache. The
//! move to compile-time-checked queries + a committed `.sqlx/` is a Phase 1 follow-up
//! (see docs/DECISIONS.md D-001).

use std::{path::PathBuf, sync::Arc};

use axum::extract::{DefaultBodyLimit, FromRef};
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
    /// In-memory failed-login counter for the brute-force throttle (scope §18 hardening).
    pub login_throttle: auth::LoginThrottle,
    /// In-memory async receipt-image extraction jobs (scope §11.2 UX). Ephemeral: process-local,
    /// pruned after 30 min; an api restart drops them (the operator just re-uploads).
    pub vlm_jobs: extractor::VlmJobs,
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
    // migrations/ lives at the repo root, two levels above this crate. NOTE: `sqlx::migrate!`
    // embeds the migration files at COMPILE time and does not reliably re-embed when a new
    // migration is added on stable Rust — touch this file (or `cargo clean -p cec-inventory-api`)
    // so the macro re-reads the directory. Current set: 0001 init, 0002 app_user,
    // 0003 integrity_hardening (serial/asset-tag uniqueness + append-only triggers),
    // 0004 app_user_role (RBAC), 0005 api_token (service-account bearer tokens),
    // 0006 policy_unique (vendor_return_policy + cec_warranty_policy per-category uniqueness).
    sqlx::migrate!("../../migrations").run(&db).await?;

    // Cookie signing key from SESSION_SECRET. Fail closed (like DATABASE_URL): no baked-in
    // default key, and no zero-padding of short secrets (which would shrink the keyspace and,
    // with the old hardcoded default, allowed anyone to forge sessions). Require ≥64 bytes —
    // `scripts/gen_secrets.sh` writes a 64-hex-char value.
    let secret = std::env::var("SESSION_SECRET")
        .expect("SESSION_SECRET must be set (run scripts/gen_secrets.sh; never commit it)");
    if secret.len() < 64 {
        panic!("SESSION_SECRET must be at least 64 bytes (run scripts/gen_secrets.sh)");
    }
    let cookie_key = Key::from(secret.as_bytes());

    Ok(AppState {
        db,
        storage_root: Arc::new(storage_root),
        cookie_key,
        login_throttle: std::sync::Arc::new(
            std::sync::Mutex::new(std::collections::HashMap::new()),
        ),
        vlm_jobs: std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
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
    let mut protected = routes::router();
    if require_auth {
        protected = protected.route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_auth,
        ));
    }

    // Privilege-escalation surface (creating operators + minting API tokens) — gate behind
    // `require_admin` (which does its own principal check), separate from operator-level routes.
    let mut admin = Router::new()
        .route("/auth/users", post(auth::create_user))
        .route(
            "/auth/tokens",
            post(auth::create_token).get(auth::list_tokens),
        )
        .route("/auth/tokens/{id}/revoke", post(auth::revoke_token));
    if require_auth {
        admin = admin.route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_admin,
        ));
    }

    let public = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/readyz", get(readyz))
        .merge(routes::ui_router())
        .merge(auth::router());

    // Global 1 MiB body cap (DoS guard) for JSON/form routes. The receipt/image upload routes
    // raise this per-route in routes::router() (photos exceed 1 MiB); the inner route limit wins.
    public
        .merge(protected)
        .merge(admin)
        .layer(DefaultBodyLimit::max(1024 * 1024))
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
