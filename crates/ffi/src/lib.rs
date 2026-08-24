use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use axum::Router;
use http_body_util::BodyExt;
use tokio::task::JoinHandle;
use tower::ServiceExt;

use oidc_exchange::reaper::{self, HostRuntime};
use oidc_exchange::shutdown::ShutdownSignal;

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
///
/// A persistent embedder hosts the session reaper (`04-http-api.md` →
/// Bootstrap step 7): while the host process lives, expired sessions and
/// retirement records are swept every `session_repository.cleanup_interval`
/// on this instance's own runtime. An instance constructed inside AWS Lambda
/// — the Node/Python Lambda bindings wrap this same type — classifies as
/// [`HostRuntime::Lambda`] via [`HostRuntime::detect`] and spawns nothing;
/// those deployments drive `POST /internal/sessions/cleanup` from an external
/// scheduler instead.
pub struct OidcExchange {
    runtime: tokio::runtime::Runtime,
    router: Router,
    /// The spawned session reaper's handle, retained so [`Drop`] can abort it
    /// before the runtime shuts down: dropping a `JoinHandle` alone detaches
    /// its task rather than stopping it, so the explicit abort is what keeps
    /// no reaper surviving its host.
    reaper: Option<JoinHandle<()>>,
}

impl Drop for OidcExchange {
    fn drop(&mut self) {
        // Abort first, explicitly: once `runtime` drops below, its tasks die
        // with it anyway, but owning the reaper's stop here keeps that
        // guarantee independent of field-drop order.
        if let Some(handle) = self.reaper.take() {
            handle.abort();
        }
    }
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

        let service = Arc::new(
            runtime
                .block_on(oidc_exchange::bootstrap::build_service(&config))
                .map_err(|e| FfiError {
                    code: "SERVICE_ERROR".to_string(),
                    message: e.to_string(),
                })?,
        );

        // Bootstrap step 7: a persistent embedder parks the reaper loop on
        // *this* embedder-owned runtime via `Runtime::spawn` (which is why the
        // loop future is split from `tokio::spawn`), so sweeps run whenever
        // the host process is alive. There is no OS-signal story inside an
        // embedder, so the loop gets a never-firing signal and stops when
        // `Drop` aborts it; a Lambda-hosted instance spawns none.
        let reaper_handle = if HostRuntime::detect().hosts_reaper() {
            Some(runtime.spawn(reaper::reaper_loop(
                Arc::clone(&service),
                reaper::cleanup_interval_duration(&config),
                ShutdownSignal::never(),
            )))
        } else {
            None
        };

        // FFI has one request surface and no second socket to bind, so the
        // single-plane rule applies (`04-http-api.md` → Bootstrap, step 6):
        // `exchange` and `admin` serve their own plane, `all` serves the
        // public plane and logs a startup warning naming the unmounted
        // internal routes. Plane separation on this runtime is expressed by
        // constructing a second instance with `role = "admin"`.
        let routers = oidc_exchange::bootstrap::build_routers_shared(&config, service)
            .map_err(|e| FfiError {
                code: "SERVICE_ERROR".to_string(),
                message: e.to_string(),
            })?;
        let router = routers.single_plane().ok_or_else(|| FfiError {
            code: "SERVICE_ERROR".to_string(),
            message: "configured role produces no servable router plane".to_string(),
        })?;

        Ok(Self {
            runtime,
            router,
            reaper: reaper_handle,
        })
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
            router.oneshot(request).await.map_err(|e| FfiError {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// An embedder instance builds over a minimal admin-role SQLite config,
    /// serves `/health`, hosts a session reaper when the host process is
    /// persistent, and drops without hanging — which exercises the `Drop`
    /// path that aborts the reaper before the runtime shuts down.
    #[test]
    fn persistent_embedder_builds_serves_and_drops_with_a_reaper_hosted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("embedder.sqlite");
        let config_toml = format!(
            "[server]\nissuer = \"https://auth.example.com\"\nrole = \"admin\"\n\n\
             [repository]\nadapter = \"sqlite\"\n\n\
             [repository.sqlite]\npath = \"{}\"\n",
            db_path.display()
        );

        let exchange = OidcExchange::new(&config_toml).expect("embedded exchange builds");

        // The reaper is hosted only under a persistent host; inside Lambda it
        // must be absent rather than ticking on a frozen process.
        if HostRuntime::detect() == HostRuntime::Persistent {
            assert!(
                exchange.reaper.is_some(),
                "a persistent embedder must host the session reaper"
            );
        } else {
            assert!(
                exchange.reaper.is_none(),
                "a Lambda-hosted embedder must not spawn a reaper"
            );
        }

        let response = exchange
            .handle_request("GET", "/health", Vec::new(), Vec::new())
            .expect("health request routes");
        assert_eq!(response.status, 200, "the embedded router serves /health");

        let body: serde_json::Value =
            serde_json::from_slice(&response.body).expect("health body is JSON");
        assert_eq!(body["status"], "ok", "health reports service ok");

        // Dropping aborts the reaper and shuts the runtime down; a hang or a
        // panic here means the lifecycle is not owned end to end.
        drop(exchange);
    }
}
