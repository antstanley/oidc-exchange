use napi::bindgen_prelude::*;
use napi_derive::napi;

// ---------------------------------------------------------------------------
// Data types exposed to JavaScript
// ---------------------------------------------------------------------------

#[napi(object)]
pub struct HeaderEntry {
    pub name: String,
    pub value: String,
}

#[napi(object)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub headers: Vec<HeaderEntry>,
    pub body: Option<Buffer>,
}

#[napi(object)]
pub struct HttpResponse {
    pub status: u32,
    pub headers: Vec<HeaderEntry>,
    pub body: Buffer,
}

#[napi(object)]
pub struct OidcExchangeOptions {
    /// Inline TOML configuration string.
    pub config_string: Option<String>,
    /// Path to a TOML configuration file.
    pub config: Option<String>,
}

// ---------------------------------------------------------------------------
// Main class
// ---------------------------------------------------------------------------

#[napi]
pub struct OidcExchange {
    inner: oidc_exchange_ffi::OidcExchange,
}

#[napi]
impl OidcExchange {
    #[napi(constructor)]
    pub fn new(options: OidcExchangeOptions) -> napi::Result<Self> {
        let inner = if let Some(ref config_string) = options.config_string {
            oidc_exchange_ffi::OidcExchange::new(config_string)
        } else if let Some(ref config_path) = options.config {
            oidc_exchange_ffi::OidcExchange::from_file(config_path)
        } else {
            return Err(napi::Error::from_reason(
                "Either `config` (file path) or `config_string` (inline TOML) must be provided",
            ));
        };

        let inner = inner.map_err(|e| napi::Error::from_reason(e.to_string()))?;

        Ok(Self { inner })
    }

    #[napi]
    pub fn handle_request(&self, request: HttpRequest) -> napi::Result<HttpResponse> {
        let headers: Vec<(String, String)> = request
            .headers
            .into_iter()
            .map(|h| (h.name, h.value))
            .collect();

        let body = request.body.map(|b| b.to_vec()).unwrap_or_default();

        let response = self
            .inner
            .handle_request(&request.method, &request.path, headers, body)
            .map_err(|e| napi::Error::from_reason(e.to_string()))?;

        let resp_headers: Vec<HeaderEntry> = response
            .headers
            .into_iter()
            .map(|(name, value)| HeaderEntry { name, value })
            .collect();

        Ok(HttpResponse {
            status: response.status as u32,
            headers: resp_headers,
            body: Buffer::from(response.body),
        })
    }

    #[napi]
    pub fn shutdown(&self) {
        // No-op – reserved for future cleanup logic.
    }
}
