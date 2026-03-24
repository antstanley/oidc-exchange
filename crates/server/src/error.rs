use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

use oidc_exchange_core::error::Error;

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
    error_description: String,
}

/// Wrapper around domain errors and route-level errors that implements
/// `IntoResponse` for axum handlers.
pub enum ApiError {
    /// A domain error from the core service.
    Domain(Error),
    /// The `grant_type` parameter was not recognized.
    UnsupportedGrantType,
}

impl From<Error> for ApiError {
    fn from(err: Error) -> Self {
        ApiError::Domain(err)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            ApiError::UnsupportedGrantType => {
                let body = ErrorResponse {
                    error: "unsupported_grant_type".to_string(),
                    error_description: "The grant_type parameter is not supported".to_string(),
                };
                (StatusCode::BAD_REQUEST, Json(body)).into_response()
            }
            ApiError::Domain(err) => {
                let (status, error_code, description) = map_domain_error(&err);
                let body = ErrorResponse {
                    error: error_code,
                    error_description: description,
                };
                (status, Json(body)).into_response()
            }
        }
    }
}

fn map_domain_error(err: &Error) -> (StatusCode, String, String) {
    match err {
        Error::InvalidGrant { reason } => (
            StatusCode::BAD_REQUEST,
            "invalid_grant".to_string(),
            reason.clone(),
        ),
        Error::InvalidToken { reason } => (
            StatusCode::UNAUTHORIZED,
            "invalid_token".to_string(),
            reason.clone(),
        ),
        Error::InvalidRequest { reason } => (
            StatusCode::BAD_REQUEST,
            "invalid_request".to_string(),
            reason.clone(),
        ),
        Error::UnknownProvider { provider } => (
            StatusCode::BAD_REQUEST,
            "invalid_request".to_string(),
            format!("unknown provider: {}", provider),
        ),
        Error::AccessDenied { reason } => (
            StatusCode::FORBIDDEN,
            "access_denied".to_string(),
            reason.clone(),
        ),
        Error::UserSuspended { user_id: _ } => (
            StatusCode::FORBIDDEN,
            "access_denied".to_string(),
            "user account is suspended".to_string(),
        ),
        Error::Unauthorized { reason } => (
            StatusCode::UNAUTHORIZED,
            "unauthorized".to_string(),
            reason.clone(),
        ),
        Error::ProviderError { .. } => (
            StatusCode::BAD_GATEWAY,
            "server_error".to_string(),
            "upstream provider error".to_string(),
        ),
        Error::ProviderTimeout { .. } => (
            StatusCode::GATEWAY_TIMEOUT,
            "server_error".to_string(),
            "upstream provider timeout".to_string(),
        ),
        Error::StoreError { .. }
        | Error::KeyError { .. }
        | Error::AuditError { .. }
        | Error::SyncError { .. }
        | Error::ConfigError { .. } => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error".to_string(),
            "internal server error".to_string(),
        ),
    }
}
