use std::fmt;
use std::panic::AssertUnwindSafe;
use std::str::FromStr;

use axum::Router;
use futures_util::FutureExt;
use http::{HeaderName, HeaderValue, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

const PATH_ENCODE_SET: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'.')
    .remove(b'-')
    .remove(b'_')
    .remove(b'~');

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
            if base_path.is_empty()
                || base_path == "/"
                || !base_path.starts_with('/')
                || base_path.ends_with('/')
            {
                return Err(FfiError {
                    code: "CONFIG_ERROR".to_string(),
                    message: "basePath must be an absolute, non-root path without a trailing slash"
                        .to_string(),
                });
            }
            config.server.base_path = Some(base_path.to_string());
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
        let encoded_path = percent_encoding::utf8_percent_encode(&path, PATH_ENCODE_SET)
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
    pub fn runtime_handle_for_conformance(
        &self,
        request: WireRequest,
    ) -> Result<FfiResponse, FfiError> {
        self.runtime.block_on(self.handle(request))
    }

    #[cfg(test)]
    fn with_router_for_test(router: Router, max_body_bytes: u64) -> Result<Self, FfiError> {
        let runtime = tokio::runtime::Runtime::new().map_err(|e| FfiError {
            code: "RUNTIME_ERROR".to_string(),
            message: e.to_string(),
        })?;
        Ok(Self {
            runtime,
            router,
            limits: NormalisationLimits { max_body_bytes },
        })
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

#[cfg(test)]
mod tests {
    use std::convert::Infallible;
    use std::future::{ready, Ready};
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll};

    use axum::body::{Body, Bytes};
    use futures_util::FutureExt;
    use http::{Request, Response};
    use http_body::{Body as HttpBody, Frame};
    use tracing_subscriber::layer::{Context as LayerContext, Layer};
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::registry::LookupSpan;

    use super::*;

    const PANIC_DETAIL: &str = "router panic raw-subject-fixture secret-token-fixture";
    const SECRET: &str = "secret-token-fixture";
    const SUBJECT: &str = "raw-subject-fixture";

    #[derive(Clone, Copy)]
    enum PanicPoint {
        ServiceFuture,
        ResponseBody,
    }

    #[derive(Clone, Copy)]
    struct InjectedService {
        panic_point: PanicPoint,
    }

    impl tower::Service<Request<Body>> for InjectedService {
        type Response = Response<Body>;
        type Error = Infallible;
        type Future = Ready<Result<Self::Response, Self::Error>>;

        fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, _request: Request<Body>) -> Self::Future {
            match self.panic_point {
                PanicPoint::ServiceFuture => panic!("{PANIC_DETAIL}"),
                PanicPoint::ResponseBody => ready(Ok(Response::new(Body::new(PanickingBody)))),
            }
        }
    }

    struct PanickingBody;

    impl HttpBody for PanickingBody {
        type Data = Bytes;
        type Error = Infallible;

        fn poll_frame(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
            panic!("{PANIC_DETAIL}")
        }
    }

    #[derive(Default)]
    struct EventVisitor(String);

    impl tracing::field::Visit for EventVisitor {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
            use std::fmt::Write;
            let _ = write!(self.0, "{}={value:?};", field.name());
        }
    }

    #[derive(Clone)]
    struct EventCapture(Arc<Mutex<String>>);

    impl<S> Layer<S> for EventCapture
    where
        S: tracing::Subscriber + for<'lookup> LookupSpan<'lookup>,
    {
        fn on_event(&self, event: &tracing::Event<'_>, _ctx: LayerContext<'_, S>) {
            let mut visitor = EventVisitor::default();
            event.record(&mut visitor);
            self.0
                .lock()
                .expect("capture mutex must not be poisoned")
                .push_str(&visitor.0);
        }
    }

    fn runtime(panic_point: PanicPoint) -> OidcExchange {
        OidcExchange::with_router_for_test(
            Router::new().fallback_service(InjectedService { panic_point }),
            1024,
        )
        .expect("test runtime must construct")
    }

    fn request(request_id: &str) -> WireRequest {
        WireRequest {
            method: "POST".into(),
            raw_path: b"/injected".to_vec(),
            query: None,
            headers: vec![
                ("x-request-id".into(), request_id.into()),
                ("authorization".into(), format!("Bearer {SECRET}")),
                ("x-subject".into(), SUBJECT.into()),
            ],
            body: SUBJECT.as_bytes().to_vec(),
            hints: TransportHints { path_is_raw: true },
        }
    }

    fn assert_safe_500(response: &FfiResponse, expected_request_id: Option<&str>, logs: &str) {
        assert_eq!(response.status, 500);
        assert_eq!(response.body, Vec::<u8>::new());
        let expected_headers = expected_request_id.map_or_else(Vec::new, |request_id| {
            vec![("x-request-id".to_string(), request_id.to_string())]
        });
        assert_eq!(response.headers, expected_headers);
        let response_debug = format!("{:?}{:?}", response.headers, response.body);
        for sensitive in [PANIC_DETAIL, SECRET, SUBJECT] {
            assert!(!response_debug.contains(sensitive));
            assert!(!logs.contains(sensitive));
        }
        assert!(logs.contains("panic contained at FFI request boundary"));
    }

    fn exercise_async(panic_point: PanicPoint, request_id: &str) {
        let runtime = runtime(panic_point);
        let captured = Arc::new(Mutex::new(String::new()));
        let subscriber = tracing_subscriber::registry().with(EventCapture(captured.clone()));
        let _guard = tracing::subscriber::set_default(subscriber);

        let outcome = runtime
            .runtime
            .block_on(AssertUnwindSafe(runtime.handle(request(request_id))).catch_unwind());
        let response = outcome
            .expect("handle future must not unwind")
            .expect("panic has HTTP semantics");
        let expected_id = HeaderValue::from_str(request_id).ok().map(|_| request_id);
        let logs = captured.lock().expect("capture mutex must not be poisoned");
        assert_safe_500(&response, expected_id, &logs);
    }

    #[test]
    fn service_future_poll_panic_is_contained_and_runtime_is_reusable() {
        exercise_async(PanicPoint::ServiceFuture, "req-safe-123");
        let runtime = runtime(PanicPoint::ServiceFuture);
        for _ in 0..2 {
            let outcome = runtime
                .runtime
                .block_on(AssertUnwindSafe(runtime.handle(request("req-reuse"))).catch_unwind());
            assert_eq!(
                outcome.expect("runtime must remain usable").unwrap().status,
                500
            );
        }
    }

    #[test]
    fn response_body_poll_panic_is_contained_and_invalid_request_id_is_not_reflected() {
        exercise_async(PanicPoint::ResponseBody, "invalid\nrequest-id");
    }

    #[test]
    fn deprecated_sync_compatibility_path_does_not_unwind() {
        let runtime = runtime(PanicPoint::ResponseBody);
        #[allow(deprecated)]
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            runtime.handle_request(
                "POST",
                "/injected",
                vec![("x-request-id".into(), "sync-safe-123".into())],
                Vec::new(),
            )
        }));
        let response = outcome
            .expect("block_on compatibility trampoline must not unwind")
            .expect("panic has HTTP semantics");
        assert_safe_500(
            &response,
            Some("sync-safe-123"),
            "panic contained at FFI request boundary",
        );
    }
}
