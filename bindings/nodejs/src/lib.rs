use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use napi::bindgen_prelude::{AsyncTask, Buffer};
use napi::{Env, Task};
use napi_derive::napi;
use oidc_exchange_ffi::{FfiResponse, TransportHints, WireRequest};

static SYNC_WARNING_EMITTED: AtomicBool = AtomicBool::new(false);

#[napi(object)]
pub struct HeaderEntry {
    pub name: String,
    pub value: String,
}

#[napi(object)]
pub struct HttpRequest {
    pub method: String,
    pub raw_path: Buffer,
    pub query: Option<Buffer>,
    pub headers: Vec<HeaderEntry>,
    pub body: Option<Buffer>,
    pub path_is_raw: bool,
}

#[napi(object)]
pub struct HttpResponse {
    pub status: u32,
    pub headers: Vec<HeaderEntry>,
    pub body: Buffer,
}

#[napi(object)]
pub struct Limits {
    pub max_body_bytes: i64,
}

#[napi(object)]
pub struct OidcExchangeOptions {
    pub config_string: Option<String>,
    pub config: Option<String>,
    pub base_path: Option<String>,
}

#[napi]
pub struct OidcExchange {
    inner: Arc<oidc_exchange_ffi::OidcExchange>,
}

pub struct HandleRequestTask {
    inner: Arc<oidc_exchange_ffi::OidcExchange>,
    request: Option<WireRequest>,
}

impl Task for HandleRequestTask {
    type Output = FfiResponse;
    type JsValue = HttpResponse;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        let request = self
            .request
            .take()
            .ok_or_else(|| napi::Error::from_reason("request task was already consumed"))?;
        self.inner
            .handle_blocking(request)
            .map_err(|error| napi::Error::from_reason(error.to_string()))
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(response_to_node(output))
    }
}

#[napi]
impl OidcExchange {
    #[napi(constructor)]
    pub fn new(options: OidcExchangeOptions) -> napi::Result<Self> {
        let inner = if let Some(ref config_string) = options.config_string {
            oidc_exchange_ffi::OidcExchange::new_with_base_path(
                config_string,
                options.base_path.as_deref(),
            )
        } else if let Some(ref config_path) = options.config {
            let config_string = std::fs::read_to_string(config_path)
                .map_err(|error| napi::Error::from_reason(error.to_string()))?;
            oidc_exchange_ffi::OidcExchange::new_with_base_path(
                &config_string,
                options.base_path.as_deref(),
            )
        } else {
            return Err(napi::Error::from_reason(
                "Either `config` (file path) or `config_string` (inline TOML) must be provided",
            ));
        };
        let inner = inner.map_err(|error| napi::Error::from_reason(error.to_string()))?;
        Ok(Self {
            inner: Arc::new(inner),
        })
    }

    #[napi]
    pub fn handle_request(&self, request: HttpRequest) -> AsyncTask<HandleRequestTask> {
        AsyncTask::new(HandleRequestTask {
            inner: Arc::clone(&self.inner),
            request: Some(request_to_wire(request)),
        })
    }

    #[napi]
    pub fn handle_request_sync(&self, request: HttpRequest) -> napi::Result<HttpResponse> {
        if !SYNC_WARNING_EMITTED.swap(true, Ordering::Relaxed) {
            eprintln!(
                "DeprecationWarning: handleRequestSync is deprecated; await handleRequest instead"
            );
        }
        self.inner
            .handle_blocking(request_to_wire(request))
            .map(response_to_node)
            .map_err(|error| napi::Error::from_reason(error.to_string()))
    }

    #[napi]
    pub fn limits(&self) -> Limits {
        Limits {
            max_body_bytes: self.inner.limits().max_body_bytes as i64,
        }
    }

    #[napi]
    pub fn shutdown(&self) {}
}

fn request_to_wire(request: HttpRequest) -> WireRequest {
    WireRequest {
        method: request.method,
        raw_path: request.raw_path.to_vec(),
        query: request.query.map(|query| query.to_vec()),
        headers: request
            .headers
            .into_iter()
            .map(|header| (header.name, header.value))
            .collect(),
        body: request.body.map_or_else(Vec::new, |body| body.to_vec()),
        hints: TransportHints {
            path_is_raw: request.path_is_raw,
        },
    }
}

fn response_to_node(response: FfiResponse) -> HttpResponse {
    HttpResponse {
        status: response.status as u32,
        headers: response
            .headers
            .into_iter()
            .map(|(name, value)| HeaderEntry { name, value })
            .collect(),
        body: Buffer::from(response.body),
    }
}
