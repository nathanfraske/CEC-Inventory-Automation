//! Thin operator auth (scope §18): argon2 password hashes + signed session cookies keyed off
//! `SESSION_SECRET`. Intended to sit behind the Headscale mesh, not the public internet. The
//! API data/mutation routes are wrapped with `require_auth`; `/health`, `/readyz`, `/auth/*`,
//! and the read-only UI stay public.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use argon2::password_hash::rand_core::{OsRng, RngCore};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::extract::{Path, Request, State};
use axum::http::{header, HeaderMap};
use axum::middleware::Next;
use axum::response::Response;
use axum::routing::{get, post};
use axum::Json;
use axum_extra::extract::cookie::{Cookie, SameSite, SignedCookieJar};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::FromRow;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::AppState;

pub const SESSION_COOKIE: &str = "cec_session";
/// Sessions expire this long after issue (absolute). The signed cookie carries the issue time.
pub const SESSION_TTL_SECS: u64 = 12 * 3600;
/// Login throttle: lock a username after this many consecutive failures, for this long.
const MAX_LOGIN_FAILS: u32 = 10;
const LOGIN_LOCKOUT: Duration = Duration::from_secs(900);

/// Per-username failed-login state for the in-memory login throttle (resets on restart — a
/// pragmatic brute-force speed-bump for the thin mesh deployment, not a distributed limiter).
#[derive(Default)]
pub struct LoginAttempt {
    fails: u32,
    locked_until: Option<Instant>,
}
pub type LoginThrottle = Arc<Mutex<HashMap<String, LoginAttempt>>>;

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Parse the signed session cookie (`<uuid>|<issued_unix>`) and return the user id only if the
/// session has not exceeded `SESSION_TTL_SECS`. Old cookies without a timestamp read as expired.
fn session_user(jar: &SignedCookieJar) -> Option<Uuid> {
    let raw = jar.get(SESSION_COOKIE)?;
    let (uid, issued) = raw.value().split_once('|')?;
    let issued: u64 = issued.parse().ok()?;
    if now_unix().saturating_sub(issued) > SESSION_TTL_SECS {
        return None;
    }
    Uuid::parse_str(uid).ok()
}

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
    // Value carries the issue time so the session has an absolute TTL (see `session_user`).
    Cookie::build((SESSION_COOKIE, format!("{user_id}|{}", now_unix())))
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
    // The first account is an admin (it can create the rest).
    sqlx::query("INSERT INTO app_user (username, password_hash, role) VALUES ($1,$2,'admin')")
        .bind(c.username.trim())
        .bind(hash)
        .execute(&s.db)
        .await?;
    Ok(Json(
        json!({ "ok": true, "username": c.username.trim(), "role": "admin" }),
    ))
}

/// Create an operator (admin-only — sits behind `require_admin`). New accounts are `operator`
/// (the column default); promote to admin out-of-band if needed.
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

/// True if the username is currently locked out (and not yet expired).
fn is_locked(throttle: &LoginThrottle, username: &str) -> bool {
    let map = throttle.lock().unwrap();
    map.get(username)
        .and_then(|a| a.locked_until)
        .map(|until| until > Instant::now())
        .unwrap_or(false)
}

fn record_login_failure(throttle: &LoginThrottle, username: &str) {
    let mut map = throttle.lock().unwrap();
    let a = map.entry(username.to_string()).or_default();
    a.fails += 1;
    if a.fails >= MAX_LOGIN_FAILS {
        a.locked_until = Some(Instant::now() + LOGIN_LOCKOUT);
    }
}

pub async fn login(
    State(s): State<AppState>,
    jar: SignedCookieJar,
    Json(c): Json<Credentials>,
) -> ApiResult<(SignedCookieJar, Json<Value>)> {
    // Brute-force speed-bump: refuse while locked out (constant cost, no DB/argon2 work).
    if is_locked(&s.login_throttle, &c.username) {
        return Err(ApiError::TooManyRequests(
            "too many failed logins; try again later".into(),
        ));
    }
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
            record_login_failure(&s.login_throttle, &c.username);
            return Err(ApiError::Unauthorized("invalid credentials".into()));
        }
    };
    if !verify_password(&c.password, &hash) {
        record_login_failure(&s.login_throttle, &c.username);
        return Err(ApiError::Unauthorized("invalid credentials".into()));
    }
    // Success clears the failure counter.
    s.login_throttle.lock().unwrap().remove(&c.username);
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

/// Current session, or 401. Reads the signed cookie directly (TTL-checked, no middleware).
pub async fn me(State(s): State<AppState>, jar: SignedCookieJar) -> ApiResult<Json<Value>> {
    let uid = session_user(&jar).ok_or_else(|| ApiError::Unauthorized("not logged in".into()))?;
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT username, role FROM app_user WHERE id = $1")
            .bind(uid)
            .fetch_optional(&s.db)
            .await?;
    let (username, role) = row.ok_or_else(|| ApiError::Unauthorized("not logged in".into()))?;
    Ok(Json(
        json!({ "username": username, "user_id": uid, "role": role }),
    ))
}

// ---------------------------------------------------------------------------
// principal resolution: a request is authenticated by EITHER a session cookie
// (operators in a browser) OR an `Authorization: Bearer <token>` API token
// (external/service-account apps). Both carry a role.
// ---------------------------------------------------------------------------

fn sha256_hex(s: &str) -> String {
    Sha256::digest(s.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Mint a new API token: returns (plaintext shown once, sha256 hex stored). The `cec_pat_`
/// prefix makes leaked tokens recognizable (e.g. to secret scanners).
fn new_token() -> (String, String) {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let body = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    let plaintext = format!("cec_pat_{body}");
    let hash = sha256_hex(&plaintext);
    (plaintext, hash)
}

/// Resolve the caller's role from the session cookie or a bearer API token; `None` if neither
/// authenticates. Tokens are looked up by their sha256 hash; a valid token bumps `last_used_at`.
async fn resolve_role(s: &AppState, headers: &HeaderMap) -> Option<String> {
    // 1) Session cookie (TTL-checked).
    let jar = SignedCookieJar::from_headers(headers, s.cookie_key.clone());
    if let Some(uid) = session_user(&jar) {
        if let Ok(Some(role)) =
            sqlx::query_scalar::<_, String>("SELECT role FROM app_user WHERE id = $1")
                .bind(uid)
                .fetch_optional(&s.db)
                .await
        {
            return Some(role);
        }
    }
    // 2) Bearer API token.
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|t| !t.is_empty())?;
    let hash = sha256_hex(token);
    let row: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT id, role FROM api_token WHERE token_hash = $1 AND revoked_at IS NULL",
    )
    .bind(&hash)
    .fetch_optional(&s.db)
    .await
    .ok()
    .flatten();
    if let Some((id, role)) = row {
        let _ = sqlx::query("UPDATE api_token SET last_used_at = now() WHERE id = $1")
            .bind(id)
            .execute(&s.db)
            .await; // best-effort
        return Some(role);
    }
    None
}

/// Middleware: require a valid principal (cookie session or API token) for the wrapped routes.
pub async fn require_auth(
    State(s): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    if resolve_role(&s, req.headers()).await.is_some() {
        Ok(next.run(req).await)
    } else {
        Err(ApiError::Unauthorized("authentication required".into()))
    }
}

/// Middleware: require an `admin` principal (the privilege-escalation surface — creating
/// operators / minting tokens). Non-admins get 403.
pub async fn require_admin(
    State(s): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    match resolve_role(&s, req.headers()).await.as_deref() {
        Some("admin") => Ok(next.run(req).await),
        Some(_) => Err(ApiError::Forbidden("admin role required".into())),
        None => Err(ApiError::Unauthorized("authentication required".into())),
    }
}

// ---------------------------------------------------------------------------
// API token management (admin-only; mounted behind `require_admin`).
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct CreateToken {
    pub label: String,
    /// `operator` (default) or `admin`.
    #[serde(default)]
    pub role: Option<String>,
}

/// Mint a service-account token. The plaintext is returned ONCE; only its hash is stored.
pub async fn create_token(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(b): Json<CreateToken>,
) -> ApiResult<Json<Value>> {
    if b.label.trim().is_empty() {
        return Err(ApiError::BadRequest("label is required".into()));
    }
    let role = match b.role.as_deref() {
        Some("admin") => "admin",
        _ => "operator",
    };
    // Record which operator minted it (from the session cookie, if that's how they're calling).
    let jar = SignedCookieJar::from_headers(&headers, s.cookie_key.clone());
    let created_by = match session_user(&jar) {
        Some(uid) => sqlx::query_scalar::<_, String>("SELECT username FROM app_user WHERE id = $1")
            .bind(uid)
            .fetch_optional(&s.db)
            .await
            .ok()
            .flatten(),
        None => None,
    };
    let (plaintext, hash) = new_token();
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO api_token (label, token_hash, role, created_by) VALUES ($1,$2,$3,$4) RETURNING id",
    )
    .bind(b.label.trim())
    .bind(&hash)
    .bind(role)
    .bind(created_by)
    .fetch_one(&s.db)
    .await?;
    Ok(Json(json!({
        "id": id,
        "label": b.label.trim(),
        "role": role,
        "token": plaintext,
        "note": "store this token now — it is shown only once and cannot be recovered",
    })))
}

#[derive(Serialize, FromRow)]
pub struct TokenInfo {
    pub id: Uuid,
    pub label: String,
    pub role: String,
    pub created_by: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
    pub revoked_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// List tokens (metadata only — never the secret).
pub async fn list_tokens(State(s): State<AppState>) -> ApiResult<Json<Vec<TokenInfo>>> {
    let rows = sqlx::query_as::<_, TokenInfo>(
        "SELECT id, label, role, created_by, created_at, last_used_at, revoked_at \
         FROM api_token ORDER BY created_at DESC",
    )
    .fetch_all(&s.db)
    .await?;
    Ok(Json(rows))
}

/// Revoke a token (idempotent-ish: 404 if unknown or already revoked).
pub async fn revoke_token(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let n =
        sqlx::query("UPDATE api_token SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL")
            .bind(id)
            .execute(&s.db)
            .await?
            .rows_affected();
    if n == 0 {
        return Err(ApiError::NotFound(
            "token not found or already revoked".into(),
        ));
    }
    Ok(Json(json!({ "ok": true, "id": id, "revoked": true })))
}

/// Public auth routes (login/bootstrap/logout/me).
pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/auth/bootstrap", post(bootstrap))
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/auth/me", get(me))
}
