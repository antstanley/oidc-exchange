use std::fmt;
use std::panic::AssertUnwindSafe;
use std::str::FromStr;

use axum::Router;
use futures_util::FutureExt;
use http::{HeaderName, HeaderValue, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

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

pub struct FfiResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TransportHints {
    pub path_is_raw: bool,
}

#[derive(Debug, Clone)]
pub struct WireRequest {
    pub method: String,
    pub raw_path: Vec<u8>,
    pub query: Option<Vec<u8>>,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub hints: TransportHints,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NormalisationLimits {
    pub max_body_bytes: u64,
}

pub struct OidcExchange {
    runtime: tokio::runtime::Runtime,
    router: Router,
    limits: NormalisationLimits,
}

impl OidcExchange {
    pub fn new(config_toml: &str) -> Result<Self, FfiError> {
        Self::new_with_base_path(config_toml, None)
    }

    pub fn new_with_base_path(
        config_toml: &str,
        base_path: Option<&str>,
    ) -> Result<Self, FfiError> {
        let mut config =
            oidc_exchange::bootstrap::parse_config(config_toml).map_err(|e| FfiError {
                code: "CONFIG_ERROR".to_string(),
                message: e.to_string(),
            })?;
        if let Some(base_path) = base_path {
            config.server.base_path = Some(base_path.to_string());
            config.normalise();
            config.validate().map_err(|e| FfiError {
                code: "CONFIG_ERROR".to_string(),
                message: e.to_string(),
            })?;
        }
        let limits = NormalisationLimits {
            max_body_bytes: config.server.max_request_body_bytes as u64,
        };
        let runtime = tokio::runtime::Runtime::new().map_err(|e| FfiError {
            code: "RUNTIME_ERROR".to_string(),
            message: e.to_string(),
        })?;
        let service = runtime
            .block_on(oidc_exchange::bootstrap::build_service(&config))
            .map_err(|e| FfiError {
                code: "SERVICE_ERROR".to_string(),
                message: e.to_string(),
            })?;
        let router = oidc_exchange::bootstrap::build_router(&config, service);
        Ok(Self {
            runtime,
            router,
            limits,
        })
    }

    pub fn from_file(path: &str) -> Result<Self, FfiError> {
        let config_toml = std::fs::read_to_string(path).map_err(|e| FfiError {
            code: "IO_ERROR".to_string(),
            message: e.to_string(),
        })?;
        Self::new(&config_toml)
    }

    pub fn limits(&self) -> NormalisationLimits {
        self.limits
    }

    /// Total asynchronous wire boundary. Host-originated shaping failures become HTTP
    /// responses; `FfiError` is reserved for failures with no HTTP meaning.
    pub async fn handle(&self, request: WireRequest) -> Result<FfiResponse, FfiError> {
        let request_id = request_id(&request.headers);
        let future = async {
            let request = match self.normalise_request(request) {
                Ok(request) => request,
                Err(status) => return Ok(status_response(status)),
            };
            let response = self
                .router
                .clone()
                .oneshot(request)
                .await
                .map_err(|e| FfiError {
                    code: "ROUTER_ERROR".to_string(),
                    message: e.to_string(),
                })?;
            response_to_ffi(response).await
        };
        match AssertUnwindSafe(future).catch_unwind().await {
            Ok(result) => result,
            Err(_) => {
                tracing::error!(
                    request_id = request_id.as_deref().unwrap_or("unavailable"),
                    "panic contained at FFI request boundary"
                );
                Ok(panic_response(request_id.as_deref()))
            }
        }
    }

    fn normalise_request(
        &self,
        request: WireRequest,
    ) -> Result<http::Request<axum::body::Body>, StatusCode> {
        if request.body.len() as u64 > self.limits.max_body_bytes {
            return Err(StatusCode::PAYLOAD_TOO_LARGE);
        }
        let method =
            http::Method::from_str(&request.method).map_err(|_| StatusCode::BAD_REQUEST)?;
        let raw_path = if request.raw_path.is_empty() {
            b"/".as_slice()
        } else {
            request.raw_path.as_slice()
        };
        if !raw_path.starts_with(b"/") || raw_path.starts_with(b"//") {
            return Err(StatusCode::BAD_REQUEST);
        }
        let raw_path = std::str::from_utf8(raw_path).map_err(|_| StatusCode::BAD_REQUEST)?;
        if request.hints.path_is_raw && (raw_path.contains('?') || raw_path.contains('#')) {
            return Err(StatusCode::BAD_REQUEST);
        }
        let path = if request.hints.path_is_raw {
            percent_encoding::percent_decode_str(raw_path)
                .decode_utf8()
                .map_err(|_| StatusCode::BAD_REQUEST)?
                .into_owned()
        } else {
            raw_path.to_string()
        };
        let query = match request.query.as_deref() {
            Some(b"") | None => None,
            Some(bytes) => Some(std::str::from_utf8(bytes).map_err(|_| StatusCode::BAD_REQUEST)?),
        };
        // Re-encode decoded host path data before constructing `http::Uri`; this keeps
        // decoded `?` and `#` inside the path rather than promoting them to URI delimiters.
        let encoded_path =
            percent_encoding::utf8_percent_encode(&path, percent_encoding::NON_ALPHANUMERIC)
                .to_string()
                .replace("%2F", "/");
        let path_and_query = match query {
            Some(query) => format!("{encoded_path}?{query}"),
            None => encoded_path,
        };
        let uri = http::Uri::from_str(&path_and_query).map_err(|_| StatusCode::BAD_REQUEST)?;
        if uri.scheme().is_some() || uri.authority().is_some() || !uri.path().starts_with('/') {
            return Err(StatusCode::BAD_REQUEST);
        }
        let actual_body_len = request.body.len() as u64;
        let mut built = http::Request::builder()
            .method(method)
            .uri(uri)
            .body(axum::body::Body::from(request.body))
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        #[cfg(feature = "conformance")]
        built
            .extensions_mut()
            .insert(oidc_exchange::middleware::base_path::ConformancePath(path));
        let mut dropped_headers = 0_u64;
        for (name, value) in request.headers {
            match (
                HeaderName::from_bytes(name.as_bytes()),
                HeaderValue::from_str(&value),
            ) {
                (Ok(name), Ok(value)) => {
                    if name == http::header::CONTENT_LENGTH {
                        let declared = value
                            .to_str()
                            .ok()
                            .and_then(|value| value.parse::<u64>().ok())
                            .ok_or(StatusCode::BAD_REQUEST)?;
                        if declared > self.limits.max_body_bytes {
                            return Err(StatusCode::PAYLOAD_TOO_LARGE);
                        }
                        if declared != actual_body_len {
                            return Err(StatusCode::BAD_REQUEST);
                        }
                    }
                    built.headers_mut().append(name, value);
                }
                _ => dropped_headers += 1,
            }
        }
        if dropped_headers > 0 {
            tracing::warn!(
                dropped_headers,
                "invalid request headers dropped at FFI boundary"
            );
        }
        Ok(built)
    }

    #[cfg(feature = "conformance")]
    #[doc(hidden)]
    pub fn runtime_handle_for_test(
        &self,
        request: WireRequest,
    ) -> Result<FfiResponse, FfiError> {
        self.runtime.block_on(self.handle(request))
    }

    #[cfg(feature = "conformance")]
    #[doc(hidden)]
    pub fn runtime_handle_for_conformance(
        &self,
        request: WireRequest,
    ) -> Result<FfiResponse, FfiError> {
        self.runtime_handle_for_test(request)
    }

    /// Deprecated compatibility route. New bindings should pass a `WireRequest` to `handle`.
    #[deprecated(note = "use async handle(WireRequest)")]
    pub fn handle_request(
        &self,
        method: &str,
        path: &str,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> Result<FfiResponse, FfiError> {
        let (raw_path, query) = path
            .split_once('?')
            .map_or((path.as_bytes().to_vec(), None), |(path, query)| {
                (path.as_bytes().to_vec(), Some(query.as_bytes().to_vec()))
            });
        self.runtime.block_on(self.handle(WireRequest {
            method: method.to_string(),
            raw_path,
            query,
            headers,
            body,
            hints: TransportHints { path_is_raw: false },
        }))
    }
}

fn status_response(status: StatusCode) -> FfiResponse {
    FfiResponse {
        status: status.as_u16(),
        headers: Vec::new(),
        body: Vec::new(),
    }
}

fn request_id(headers: &[(String, String)]) -> Option<String> {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("x-request-id"))
        .map(|(_, value)| value.clone())
}

fn panic_response(request_id: Option<&str>) -> FfiResponse {
    let mut response = status_response(StatusCode::INTERNAL_SERVER_ERROR);
    if let Some(request_id) = request_id {
        if HeaderValue::from_str(request_id).is_ok() {
            response
                .headers
                .push(("x-request-id".to_string(), request_id.to_string()));
        }
    }
    response
}

async fn response_to_ffi(response: axum::response::Response) -> Result<FfiResponse, FfiError> {
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_string(), value.to_string()))
        })
        .collect();
    let body = response
        .into_body()
        .collect()
        .await
        .map_err(|e| FfiError {
            code: "BODY_ERROR".to_string(),
            message: e.to_string(),
        })?
        .to_bytes()
        .to_vec();
    Ok(FfiResponse {
        status,
        headers,
        body,
    })
}
