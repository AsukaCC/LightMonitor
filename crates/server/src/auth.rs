use crate::models::ErrorResponse;
use crate::state::AppState;
use axum::Json;
use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};

pub struct AuthUser {
    pub username: String,
    pub token: String,
}

#[derive(Debug)]
pub enum ApiError {
    Unauthorized(String),
    BadRequest(String),
    Conflict(String),
    NotFound(String),
    BadGateway(String),
    Internal(String),
}

impl ApiError {
    pub fn unauthorized(msg: impl Into<String>) -> Self {
        Self::Unauthorized(msg.into())
    }

    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self::BadRequest(msg.into())
    }

    pub fn conflict(msg: impl Into<String>) -> Self {
        Self::Conflict(msg.into())
    }

    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::NotFound(msg.into())
    }

    pub fn bad_gateway(msg: impl Into<String>) -> Self {
        Self::BadGateway(msg.into())
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg),
            Self::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            Self::Conflict(msg) => (StatusCode::CONFLICT, msg),
            Self::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            Self::BadGateway(msg) => (StatusCode::BAD_GATEWAY, msg),
            Self::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
        };
        (status, Json(ErrorResponse { error: message })).into_response()
    }
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = extract_bearer(&parts.headers)
            .ok_or_else(|| ApiError::unauthorized("missing bearer token"))?;
        let username = state
            .db
            .session_username(&token)
            .map_err(|e| ApiError::internal(e.to_string()))?
            .ok_or_else(|| ApiError::unauthorized("invalid or expired session"))?;
        Ok(AuthUser { username, token })
    }
}

pub fn extract_bearer(headers: &axum::http::HeaderMap) -> Option<String> {
    let value = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let token = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))?;
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

pub fn random_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
