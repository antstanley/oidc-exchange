use std::fmt;
use std::str::FromStr;

use axum::Router;
use http_body_util::BodyExt;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Error type returned by FFI operations.
#[derive(Debug)]
pub struct FfiError {
    pub code: String,
    pub message: String,
}

impl fmt::Display for FfiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for FfiError {}

// ---------------------------------------------------------------------------
// Response type
// ---------------------------------------------------------------------------

/// Simplified HTTP response returned to FFI callers.
pub struct FfiResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

// ---------------------------------------------------------------------------
// Main wrapper
// ---------------------------------------------------------------------------

/// Wraps the OIDC-Exchange Axum application for use from foreign language
/// bindings (Node.js via napi-rs, Python via PyO3, etc.).
pub struct OidcExchange {
    runtime: tokio::runtime::Runtime,
    router: Router,
}

impl OidcExchange {
    /// Create a new instance by parsing a TOML configuration string.
    pub fn new(config_toml: &str) -> Result<Self, FfiError> {
        let config = oidc_exchange::bootstrap::parse_config(config_toml).map_err(|e| FfiError {
            code: "CONFIG_ERROR".to_string(),
            message: e.to_string(),
        })?;

        let runtime = tokio::runtime::Runtime::new().map_err(|e| FfiError {
            code: "RUNTIME_ERROR".to_string(),
            message: e.to_string(),
        })?;

        let service =
            runtime
                .block_on(oidc_exchange::bootstrap::build_service(&config))
                .map_err(|e| FfiError {
                    code: "SERVICE_ERROR".to_string(),
                    message: e.to_string(),
                })?;

        let router = oidc_exchange::bootstrap::build_router(&config, service);

        Ok(Self { runtime, router })
    }

    /// Create a new instance by reading configuration from a file path.
    pub fn from_file(path: &str) -> Result<Self, FfiError> {
        let config_toml = std::fs::read_to_string(path).map_err(|e| FfiError {
            code: "IO_ERROR".to_string(),
            message: e.to_string(),
        })?;
        Self::new(&config_toml)
    }

    /// Send an HTTP request through the Axum router and return the response.
    pub fn handle_request(
        &self,
        method: &str,
        path: &str,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> Result<FfiResponse, FfiError> {
        let method = http::Method::from_str(method).map_err(|e| FfiError {
            code: "INVALID_METHOD".to_string(),
            message: e.to_string(),
        })?;

        let mut builder = http::Request::builder().method(method).uri(path);

        for (key, value) in &headers {
            builder = builder.header(key.as_str(), value.as_str());
        }

        let request = builder
            .body(axum::body::Body::from(body))
            .map_err(|e| FfiError {
                code: "REQUEST_BUILD_ERROR".to_string(),
                message: e.to_string(),
            })?;

        let router = self.router.clone();

        let response = self.runtime.block_on(async {
            router
                .oneshot(request)
                .await
                .map_err(|e| FfiError {
                    code: "ROUTER_ERROR".to_string(),
                    message: e.to_string(),
                })
        })?;

        let status = response.status().as_u16();

        let resp_headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str().to_string(),
                    v.to_str().unwrap_or_default().to_string(),
                )
            })
            .collect();

        let body_bytes = self.runtime.block_on(async {
            response
                .into_body()
                .collect()
                .await
                .map(|collected| collected.to_bytes().to_vec())
                .map_err(|e| FfiError {
                    code: "BODY_ERROR".to_string(),
                    message: e.to_string(),
                })
        })?;

        Ok(FfiResponse {
            status,
            headers: resp_headers,
            body: body_bytes,
        })
    }
}
