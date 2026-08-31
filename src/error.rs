use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;
use utoipa::ToSchema;

/// Error response body returned for all 4xx/5xx responses.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Database error: {0}")]
    Database(#[from] surrealdb::Error),

    #[error("Not found")]
    NotFound,

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Invalid relationship type: {0}")]
    InvalidRelType(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    /// 409 Conflict — body is a free-form JSON payload (e.g. merge conflict details).
    #[error("Conflict")]
    Conflict(serde_json::Value),

    /// tack-server rejected the request outright (its own 4xx) -- surfaced
    /// with tack's own status/message rather than flattened to a generic
    /// 500, since callers will actually hit real cases like "team has no
    /// organization assigned" (400) in practice. See `tack_client.rs`.
    #[error("tack-server rejected the request: {1}")]
    TackUpstream(u16, String),

    #[error("tack-server unreachable: {0}")]
    TackUnreachable(String),

    /// Genuine server-side data inconsistency (e.g. a malformed UUID stored
    /// where one is expected) -- never triggerable by a client's own input.
    #[error("Internal error: {0}")]
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            AppError::Conflict(body) => {
                (StatusCode::CONFLICT, Json(body)).into_response()
            }
            other => {
                let (status, message) = match &other {
                    AppError::Database(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
                    AppError::NotFound => (StatusCode::NOT_FOUND, "Not found".to_string()),
                    AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg.clone()),
                    AppError::Forbidden(msg) => (StatusCode::FORBIDDEN, msg.clone()),
                    AppError::InvalidRelType(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
                    AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
                    // A tack-server 401 means tack rejected the JWT we forwarded
                    // it -- an upstream dependency/auth-config failure, NOT the
                    // caller's own clann session expiring. Relaying it verbatim
                    // makes the webapp treat it as session death (any 401 there
                    // dispatches `auth:unauthorized` -> logout -> "session
                    // expired"), so a single failed notes fetch logs the user
                    // out of the whole app. Surface it as a gateway error like
                    // TackUnreachable instead. Other tack 4xx (403 note-ACL,
                    // 404, 400, 409) are genuinely client-meaningful -> relayed.
                    AppError::TackUpstream(401, _) => (
                        StatusCode::BAD_GATEWAY,
                        "notes service rejected the request".to_string(),
                    ),
                    AppError::TackUpstream(code, msg) => (
                        StatusCode::from_u16(*code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                        msg.clone(),
                    ),
                    AppError::TackUnreachable(msg) => (StatusCode::BAD_GATEWAY, msg.clone()),
                    AppError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg.clone()),
                    AppError::Conflict(_) => unreachable!(),
                };
                (status, Json(json!({ "error": message }))).into_response()
            }
        }
    }
}
