//! Per-invocation synchronous flush seam for Lambda mode.
//!
//! `04-http-api.md` → Bootstrap, step 6: "In Lambda mode, telemetry and blocking audit writes
//! flush synchronously before each invocation's response is returned, since the execution
//! environment may freeze immediately after the response." [`FlushOnResponse`] is the tower
//! `Service` middleware that implements that contract, and [`run_lambda`] is the wrapper
//! `main.rs`'s Lambda branch calls instead of `lambda_http::run` directly, so the flush hook
//! always sits between the router's response and the runtime API handing that response back to
//! Lambda.
//!
//! Buffered audit adapters flush through this same hook when one exists: today no adapter
//! buffers (`AuditLog` has no `flush` method; `SqsAuditLog::emit` awaits `send_message` per
//! event), so only the telemetry flush (`crate::telemetry::flush_telemetry`) is wired in by
//! this task, and it is itself a documented no-op under the current stdout-JSON pipeline. The
//! seam is what the OTLP/X-Ray exporters change (`changes/2026-06-24-complete_telemetry_exporters.md`)
//! and any future buffering audit adapter both plug into.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use tower::Service;

/// A tower `Service` middleware that calls a flush hook synchronously after the wrapped
/// service's response future resolves, before the response is handed back to the caller.
///
/// The flush runs unconditionally — on both a success (e.g. `200`) and an error/non-200
/// (e.g. `404`/`5xx`) response — because the Lambda execution environment may freeze
/// immediately after *any* response is returned, not only successful ones.
pub struct FlushOnResponse<S> {
    inner: S,
    flush: Arc<dyn Fn() + Send + Sync>,
}

impl<S> FlushOnResponse<S> {
    /// Wrap `inner` so `flush` fires synchronously after each call's response future
    /// resolves, before the response is returned to the caller.
    pub fn new(inner: S, flush: Arc<dyn Fn() + Send + Sync>) -> Self {
        Self { inner, flush }
    }
}

impl<S, Req> Service<Req> for FlushOnResponse<S>
where
    S: Service<Req>,
    S::Future: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Req) -> Self::Future {
        let fut = self.inner.call(req);
        let flush = Arc::clone(&self.flush);
        Box::pin(async move {
            let result = fut.await;
            // Flush unconditionally, before the response (success or error) is returned —
            // see the type doc comment above and the module-level spec citation.
            flush();
            result
        })
    }
}

/// Serve `app` (the shared router `bootstrap::build_router` produces, identical to the one the
/// hyper path serves) through `lambda_http::run`, wrapped in [`FlushOnResponse`] so `flush`
/// runs synchronously after each invocation's response future resolves and before the response
/// is returned to the Lambda runtime API.
pub async fn run_lambda(
    app: axum::Router,
    flush: Arc<dyn Fn() + Send + Sync>,
) -> Result<(), lambda_http::Error> {
    lambda_http::run(FlushOnResponse::new(app, flush)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use axum::Router;
    use std::sync::atomic::{AtomicUsize, Ordering};

    async fn ok_handler() -> &'static str {
        "ok"
    }

    async fn error_handler() -> StatusCode {
        StatusCode::INTERNAL_SERVER_ERROR
    }

    fn test_app() -> Router {
        Router::new()
            .route("/ok", get(ok_handler))
            .route("/error", get(error_handler))
    }

    /// The flush hook fires exactly once per invocation across two sequential invocations —
    /// one that resolves `200`, one that resolves a non-200 error status — proving the flush
    /// runs on both the success and error paths, and runs *after* the response is produced
    /// (not before, and not skipped for the error path).
    #[tokio::test]
    async fn flush_fires_once_per_invocation_on_success_and_error_paths() {
        let counter = Arc::new(AtomicUsize::new(0));
        let flush_counter = Arc::clone(&counter);
        let mut wrapped = FlushOnResponse::new(
            test_app(),
            Arc::new(move || {
                flush_counter.fetch_add(1, Ordering::SeqCst);
            }),
        );

        let ok_response = wrapped
            .call(Request::get("/ok").body(Body::empty()).unwrap())
            .await
            .expect("router call never fails");
        assert_eq!(ok_response.status(), StatusCode::OK);
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "flush must fire exactly once after the first (success) invocation resolves"
        );

        let error_response = wrapped
            .call(Request::get("/error").body(Body::empty()).unwrap())
            .await
            .expect("router call never fails");
        assert_eq!(error_response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            counter.load(Ordering::SeqCst),
            2,
            "flush must fire exactly once more after the second (error) invocation resolves — \
             the error/non-200 path must not skip the flush"
        );
    }

    /// Negative-space: a request to a route no handler serves (404, produced entirely by
    /// axum's router rather than a handler body) still triggers the flush — the hook must not
    /// depend on a matched route or a handler having run.
    #[tokio::test]
    async fn flush_fires_even_when_no_route_matches() {
        let counter = Arc::new(AtomicUsize::new(0));
        let flush_counter = Arc::clone(&counter);
        let mut wrapped = FlushOnResponse::new(
            test_app(),
            Arc::new(move || {
                flush_counter.fetch_add(1, Ordering::SeqCst);
            }),
        );

        let response = wrapped
            .call(Request::get("/does-not-exist").body(Body::empty()).unwrap())
            .await
            .expect("router call never fails");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "flush must fire even for a router-level 404 with no matched handler"
        );
    }
}
