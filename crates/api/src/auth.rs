//! Thin operator auth (scope §18): argon2 password hashes + signed session cookies keyed off
//! `SESSION_SECRET`. Intended to sit behind the Headscale mesh, not the public internet. The
//! API data/mutation routes are wrapped with `require_auth`; `/health`, `/readyz`, `/auth/*`,
//! and the read-only UI stay public.

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use axum::routing::{get, post};
use axum::Json;
use axum_extra::extract::cookie::{Cookie, SameSite, SignedCookieJar};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::AppState;

pub const SESSION_COOKIE: &str = "cec_session";

#[derive(Deserialize)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

fn hash_password(pw: &str) -> ApiResult<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(pw.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("hash error: {e}")))
}

fn verify_password(pw: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .map(|ph| {
            Argon2::default()
                .verify_password(pw.as_bytes(), &ph)
                .is_ok()
        })
        .unwrap_or(false)
}

/// A valid argon2 PHC hash computed once, used to equalize timing on the unknown-user login
/// path (so a missing username costs the same argon2 work as a wrong password).
fn dummy_hash() -> &'static str {
    static H: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    H.get_or_init(|| hash_password("timing-equalizer-not-a-credential").unwrap_or_default())
}

fn session_cookie(user_id: Uuid) -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE, user_id.to_string()))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        // .secure(true) — enable behind TLS; left off so it works over http on the mesh.
        .build()
}

/// Create the first operator. Allowed only while no users exist; afterward use
/// `POST /auth/users` (authenticated).
pub async fn bootstrap(
    State(s): State<AppState>,
    Json(c): Json<Credentials>,
) -> ApiResult<Json<Value>> {
    if c.username.trim().is_empty() || c.password.len() < 12 {
        return Err(ApiError::BadRequest(
            "username required and password must be at least 12 chars".into(),
        ));
    }
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM app_user")
        .fetch_one(&s.db)
        .await?;
    if n > 0 {
        return Err(ApiError::BadRequest(
            "already bootstrapped; create users via POST /auth/users while authenticated".into(),
        ));
    }
    let hash = hash_password(&c.password)?;
    sqlx::query("INSERT INTO app_user (username, password_hash) VALUES ($1,$2)")
        .bind(c.username.trim())
        .bind(hash)
        .execute(&s.db)
        .await?;
    Ok(Json(json!({ "ok": true, "username": c.username.trim() })))
}

/// Create an operator (must already be authenticated — sits behind `require_auth`).
pub async fn create_user(
    State(s): State<AppState>,
    Json(c): Json<Credentials>,
) -> ApiResult<Json<Value>> {
    if c.username.trim().is_empty() || c.password.len() < 12 {
        return Err(ApiError::BadRequest(
            "username required and password must be at least 12 chars".into(),
        ));
    }
    let hash = hash_password(&c.password)?;
    sqlx::query("INSERT INTO app_user (username, password_hash) VALUES ($1,$2)")
        .bind(c.username.trim())
        .bind(hash)
        .execute(&s.db)
        .await?;
    Ok(Json(json!({ "ok": true, "username": c.username.trim() })))
}

pub async fn login(
    State(s): State<AppState>,
    jar: SignedCookieJar,
    Json(c): Json<Credentials>,
) -> ApiResult<(SignedCookieJar, Json<Value>)> {
    let row: Option<(Uuid, String)> =
        sqlx::query_as("SELECT id, password_hash FROM app_user WHERE username = $1")
            .bind(&c.username)
            .fetch_optional(&s.db)
            .await?;
    let (id, hash) = match row {
        Some(r) => r,
        None => {
            // Verify against a fixed dummy hash so the unknown-user path costs the same as a
            // wrong-password path (no timing-based username enumeration).
            verify_password(&c.password, dummy_hash());
            return Err(ApiError::Unauthorized("invalid credentials".into()));
        }
    };
    if !verify_password(&c.password, &hash) {
        return Err(ApiError::Unauthorized("invalid credentials".into()));
    }
    Ok((
        jar.add(session_cookie(id)),
        Json(json!({ "ok": true, "username": c.username })),
    ))
}

pub async fn logout(jar: SignedCookieJar) -> (SignedCookieJar, Json<Value>) {
    // Removal cookie must carry the same path as the session cookie to actually clear it.
    let removal = Cookie::build((SESSION_COOKIE, "")).path("/").build();
    (jar.remove(removal), Json(json!({ "ok": true })))
}

/// Current session, or 401. Reads the signed cookie directly (no middleware needed).
pub async fn me(State(s): State<AppState>, jar: SignedCookieJar) -> ApiResult<Json<Value>> {
    let uid = jar
        .get(SESSION_COOKIE)
        .and_then(|c| Uuid::parse_str(c.value()).ok())
        .ok_or_else(|| ApiError::Unauthorized("not logged in".into()))?;
    let username: Option<String> =
        sqlx::query_scalar("SELECT username FROM app_user WHERE id = $1")
            .bind(uid)
            .fetch_optional(&s.db)
            .await?;
    let username = username.ok_or_else(|| ApiError::Unauthorized("not logged in".into()))?;
    Ok(Json(json!({ "username": username, "user_id": uid })))
}

/// Middleware: require a valid signed session for the wrapped routes. The jar is rebuilt
/// from the request headers with the state's key (rather than relying on the extractor
/// inside `from_fn_with_state`).
pub async fn require_auth(
    State(s): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let jar = SignedCookieJar::from_headers(req.headers(), s.cookie_key.clone());
    let uid = jar
        .get(SESSION_COOKIE)
        .and_then(|c| Uuid::parse_str(c.value()).ok())
        .ok_or_else(|| ApiError::Unauthorized("authentication required".into()))?;
    let exists: Option<Uuid> = sqlx::query_scalar("SELECT id FROM app_user WHERE id = $1")
        .bind(uid)
        .fetch_optional(&s.db)
        .await?;
    if exists.is_none() {
        return Err(ApiError::Unauthorized("authentication required".into()));
    }
    Ok(next.run(req).await)
}

/// Public auth routes (login/bootstrap/logout/me).
pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/auth/bootstrap", post(bootstrap))
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/auth/me", get(me))
}
