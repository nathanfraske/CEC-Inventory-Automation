// Minimal runnable spine: load env, connect Postgres, run migrations, serve health.
// Adapt to the installed axum version if a major has moved (axum::serve exists in 0.7+).
use axum::{extract::State, routing::get, Json, Router};
use sqlx::postgres::PgPoolOptions;
use std::env;

#[derive(Clone)]
struct AppState {
    db: sqlx::PgPool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // DATABASE_URL contains the password and lives only in the gitignored .env.
    let database_url =
        env::var("DATABASE_URL").expect("DATABASE_URL must be set (see .env, never commit it)");
    let bind = env::var("API_BIND").unwrap_or_else(|_| "0.0.0.0:8080".to_string());

    let db = PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await?;

    // migrations/ lives at the repo root, one level above the crate.
    sqlx::migrate!("../../migrations").run(&db).await?;

    let state = AppState { db };
    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/readyz", get(readyz))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!("listening on {bind}");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn readyz(
    State(s): State<AppState>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&s.db)
        .await
        .map(|_| Json(serde_json::json!({ "db": "up" })))
        .map_err(|e| (axum::http::StatusCode::SERVICE_UNAVAILABLE, e.to_string()))
}
