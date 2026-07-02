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
        Error::Conflict { detail } => {
            (StatusCode::CONFLICT, "conflict".to_string(), detail.clone())
        }
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

#[cfg(test)]
mod tests {
    use http_body_util::BodyExt;

    use super::*;

    async fn response_to_json(response: Response) -> (StatusCode, serde_json::Value) {
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json = serde_json::from_slice(&bytes).unwrap();
        (status, json)
    }

    #[tokio::test]
    async fn conflict_error_renders_409_with_conflict_code() {
        let err = ApiError::Domain(Error::Conflict {
            detail: "user already registered for (google, sub-123)".to_string(),
        });

        let (status, body) = response_to_json(err.into_response()).await;

        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["error"], "conflict");
        assert_eq!(
            body["error_description"],
            "user already registered for (google, sub-123)"
        );
    }

    #[test]
    fn oauth_error_envelope_enum_includes_conflict_and_stays_closed() {
        let schema_str = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../.specs/canonical-types.schema.json"
        ))
        .expect("failed to read .specs/canonical-types.schema.json");
        let schema: serde_json::Value =
            serde_json::from_str(&schema_str).expect("failed to parse canonical-types schema");

        let enum_values = schema["$defs"]["OAuthErrorEnvelope"]["properties"]["error"]["enum"]
            .as_array()
            .expect("OAuthErrorEnvelope.properties.error.enum must be an array")
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect::<Vec<_>>();

        // Positive: the new wire code is present.
        assert!(
            enum_values.iter().any(|v| v == "conflict"),
            "expected \"conflict\" in the OAuthErrorEnvelope error enum, got {enum_values:?}"
        );

        // Negative-space: the enum is closed — an arbitrary code outside the
        // eight known members is not a valid enum member.
        let bogus = "not_a_real_error_code";
        assert!(
            !enum_values.iter().any(|v| v == bogus),
            "the OAuthErrorEnvelope error enum must stay closed"
        );
        assert_eq!(
            enum_values.len(),
            8,
            "expected exactly eight OAuthErrorEnvelope error codes, got {enum_values:?}"
        );
    }
}
