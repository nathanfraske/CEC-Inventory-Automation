//! Unified API error type mapped to HTTP responses. Handlers return `ApiResult<T>`.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(Debug)]
pub enum ApiError {
    NotFound(String),
    BadRequest(String),
    Db(sqlx::Error),
    Internal(anyhow::Error),
}

pub type ApiResult<T> = Result<T, ApiError>;

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        match e {
            sqlx::Error::RowNotFound => ApiError::NotFound("resource not found".into()),
            other => ApiError::Db(other),
        }
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError::Internal(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::NotFound(m) => (StatusCode::NOT_FOUND, m),
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            ApiError::Db(e) => {
                // Surface constraint violations as actionable 4xx; everything else is 500.
                if let Some(dbe) = e.as_database_error() {
                    if dbe.is_unique_violation() {
                        (StatusCode::CONFLICT, dbe.message().to_string())
                    } else if dbe.is_foreign_key_violation() {
                        (StatusCode::BAD_REQUEST, dbe.message().to_string())
                    } else {
                        tracing::error!("db error: {e}");
                        (StatusCode::INTERNAL_SERVER_ERROR, "database error".into())
                    }
                } else {
                    tracing::error!("db error: {e}");
                    (StatusCode::INTERNAL_SERVER_ERROR, "database error".into())
                }
            }
            ApiError::Internal(e) => {
                tracing::error!("internal error: {e:?}");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error".into())
            }
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}
