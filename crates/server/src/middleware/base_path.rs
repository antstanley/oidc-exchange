use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::body::Body;
use axum::http::Request;
use axum::response::Response;
use axum::Router;
use tower::Service;

/// Wrap `inner` — a fully-built, stateful `Router` (the output of
/// `bootstrap::build_router`'s route merging and `.with_state()` call) — so every request has
/// `base_path` stripped from its URI path *before* axum's routing decision is made, not merely
/// before whichever handler ends up matched.
///
/// This cannot be done with `Router::layer`: axum applies `.layer()` to each already-registered
/// route's individual endpoint (see `axum::routing::Router::layer`'s implementation), which only
/// runs *after* the router has already decided which route (if any) matches the request's
/// current path. A layer added that way can log, authenticate, or transform the response around
/// the matched handler, but it can never influence *which* handler gets chosen — by the time it
/// runs, that decision is final. Rewriting the path early enough to change the routing outcome
/// (`/prod/health` → `/health`, so the `/health` route matches) requires wrapping the *entire*
/// router as an opaque unit from the outside, before any routing occurs at all.
///
/// The technique: build a brand-new, routeless outer `Router` whose *fallback* service handles
/// every request unconditionally — since no other route is ever registered on this outer router,
/// every request "fails to match" and falls through to the fallback, with no exceptions. The
/// fallback strips the prefix and then dispatches into `inner`, which performs the real routing
/// decision against the now-rewritten path. When `base_path` is `None`, the same fallback still
/// runs on every request, but [`strip_base_path`] is a pure pass-through, so there is exactly one
/// code path (never a `None`-only shortcut that skips the wrapping entirely).
pub fn with_base_path_strip(inner: Router, base_path: Option<String>) -> Router {
    let service = BasePathStripService {
        inner,
        base_path: base_path.map(Arc::from),
    };
    Router::new().fallback_service(service)
}

/// The `tower::Service` behind [`with_base_path_strip`]'s outer router fallback.
///
/// Deliberately minimal: `poll_ready` is always ready (matching `Router`'s own `Service` impl,
/// which never applies backpressure), and `call` does exactly two things — strip the prefix,
/// then hand the (possibly rewritten) request to a cloned `inner` router. `Router` clones are
/// cheap (its internal route table is `Arc`-shared), so cloning per call rather than holding a
/// `&mut Router` across `.await` points is the standard pattern for calling a `Router` as a
/// `Service` from another `Service`.
#[derive(Clone)]
struct BasePathStripService {
    inner: Router,
    base_path: Option<Arc<str>>,
}

impl Service<Request<Body>> for BasePathStripService {
    type Response = Response;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Response, Infallible>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Request<Body>) -> Self::Future {
        let mut inner = self.inner.clone();
        let base_path = self.base_path.clone();
        Box::pin(async move {
            let request = strip_base_path(request, base_path.as_deref());
            inner.call(request).await
        })
    }
}

/// Rewrite `request`'s URI path to drop a leading `prefix`, when one is configured and the
/// request path actually starts with it at a path-segment boundary (see
/// [`strip_prefix_at_segment_boundary`]); otherwise return `request` completely unmodified.
///
/// This is the pure core [`with_base_path_strip`] wraps: no I/O, no logging, just a URI
/// rewrite, which is what makes it directly unit-testable without spinning up a router.
fn strip_base_path(mut request: Request<Body>, prefix: Option<&str>) -> Request<Body> {
    // Treat "no prefix" and "empty-string prefix" identically: `Some("")` is not rejected by
    // config deserialization, and `str::strip_prefix("")` would trivially match every path
    // (a no-op strip in effect, since the "stripped" remainder is the whole original path).
    // Folding it into the same early return as `None` here keeps that case cheap (no
    // allocation, no URI reconstruction) and — more importantly — keeps a misconfigured empty
    // prefix a harmless pass-through instead of a per-request failure mode: this middleware
    // runs on every request, so any panic here would take the whole service down repeatedly
    // rather than failing once, closed, at config load.
    let Some(prefix) = prefix.filter(|p| !p.is_empty()) else {
        return request;
    };

    let path = request.uri().path();
    let Some(stripped) = strip_prefix_at_segment_boundary(path, prefix) else {
        // Path does not start with `prefix` at a segment boundary — either it lacks the
        // prefix entirely (e.g. `/health` against `/prod`) or only shares a longer sibling
        // segment (e.g. `/production/x` against `/prod`). Leave the request untouched rather
        // than guess; the caller decides what "unmodified" resolves to downstream.
        return request;
    };

    // A request exactly equal to the bare prefix (`stripped == ""`) rewrites to the root path
    // `/`, not an empty path — axum's router (like any HTTP path matcher) treats `/` as the
    // canonical root and does not special-case an empty string.
    let new_path = if stripped.is_empty() { "/" } else { stripped };

    let new_path_and_query = match request.uri().query() {
        Some(query) => format!("{new_path}?{query}"),
        None => new_path.to_string(),
    };

    let mut parts = request.uri().clone().into_parts();
    parts.path_and_query = Some(new_path_and_query.parse().unwrap_or_else(|err| {
        panic!(
            "rewritten path/query {new_path_and_query:?} (from path {path:?}, prefix \
             {prefix:?}) must itself be a valid URI path-and-query: {err}"
        )
    }));
    let new_uri = axum::http::Uri::from_parts(parts).unwrap_or_else(|err| {
        panic!(
            "replacing only the path-and-query component of a valid request URI must not \
             invalidate it: {err}"
        )
    });
    // Postcondition: the rewritten URI must remain an absolute path the router can match
    // against — never empty, which axum's matcher does not treat as `/`.
    assert!(
        !new_uri.path().is_empty(),
        "rewritten URI path must never be empty, got {:?} from prefix {prefix:?} on {path:?}",
        new_uri.path()
    );
    // Postcondition: a genuine boundary-matched strip (this branch only, `prefix` is
    // non-empty here by construction) must always change the path — it removed at least
    // `prefix.len()` bytes, or collapsed the bare prefix down to `/`. Either way the
    // rewritten path can never equal the original; catching that here would mean the
    // boundary check above let a no-op "match" through unnoticed.
    assert_ne!(
        new_uri.path(),
        path,
        "boundary-matched strip of prefix {prefix:?} must change the path, got {path:?} unchanged"
    );

    *request.uri_mut() = new_uri;
    request
}

/// Strip `prefix` from the start of `path`, but only when `prefix` ends exactly on a
/// path-segment boundary in `path` — the byte immediately following the shared bytes must be
/// `/` or the end of the string.
///
/// This is what distinguishes `/prod` correctly stripping from `/prod/health` (and from the
/// bare `/prod` request itself) while correctly leaving `/production/x` alone: `path` shares
/// the literal bytes `/prod` with `prefix` in both cases, but only the first has a boundary
/// (`/` or end-of-string) right after them. A raw `str::strip_prefix` alone cannot tell these
/// apart, which is exactly the bug this helper exists to rule out.
fn strip_prefix_at_segment_boundary<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    let rest = path.strip_prefix(prefix)?;
    (rest.is_empty() || rest.starts_with('/')).then_some(rest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use tower::ServiceExt;

    fn get_req(uri: &str) -> Request<Body> {
        Request::builder().uri(uri).body(Body::empty()).unwrap()
    }

    #[test]
    fn boundary_strips_exact_and_nested_segments() {
        assert_eq!(
            strip_prefix_at_segment_boundary("/prod/health", "/prod"),
            Some("/health")
        );
        assert_eq!(strip_prefix_at_segment_boundary("/prod", "/prod"), Some(""));
    }

    #[test]
    fn boundary_rejects_sibling_segment_and_absent_prefix() {
        // `/production` shares the literal bytes `/prod` with the prefix but the next byte
        // (`u`) is not a segment boundary, so it must not be treated as prefixed.
        assert_eq!(
            strip_prefix_at_segment_boundary("/production/x", "/prod"),
            None
        );
        assert_eq!(strip_prefix_at_segment_boundary("/health", "/prod"), None);
    }

    #[test]
    fn strip_base_path_is_noop_when_prefix_is_none() {
        let request = get_req("/health");
        let original_path = request.uri().path().to_string();
        let request = strip_base_path(request, None);
        assert_eq!(request.uri().path(), original_path);
        assert_eq!(request.uri().path(), "/health");
    }

    #[test]
    fn strip_base_path_rewrites_prefixed_path_and_preserves_query() {
        let request = Request::builder()
            .uri("/prod/health?x=1")
            .body(Body::empty())
            .unwrap();
        let request = strip_base_path(request, Some("/prod"));
        assert_eq!(request.uri().path(), "/health");
        assert_eq!(request.uri().query(), Some("x=1"));
    }

    #[test]
    fn strip_base_path_rewrites_bare_prefix_to_root() {
        let request = strip_base_path(get_req("/prod"), Some("/prod"));
        assert_eq!(request.uri().path(), "/");
    }

    #[test]
    fn strip_base_path_leaves_mismatched_and_sibling_paths_unmodified() {
        let request = strip_base_path(get_req("/health"), Some("/prod"));
        assert_eq!(request.uri().path(), "/health");

        let request = strip_base_path(get_req("/production/x"), Some("/prod"));
        assert_eq!(request.uri().path(), "/production/x");
    }

    #[test]
    fn strip_base_path_treats_empty_prefix_as_unconfigured() {
        // A `base_path = ""` misconfiguration must degrade to a harmless pass-through, not a
        // per-request panic — this middleware runs on every request.
        let request = strip_base_path(get_req("/health"), Some(""));
        assert_eq!(request.uri().path(), "/health");
    }

    /// Regression test for the mechanism itself: proves stripping happens early enough to
    /// change axum's *routing decision*, not merely the request a matched handler observes.
    /// A naive `Router::layer(from_fn(...))` implementation looks correct in isolation but
    /// never actually reroutes anything, because axum applies `.layer()` per already-matched
    /// endpoint (see the module doc comment) — this test would fail against that
    /// implementation the same way the real router did during development.
    #[tokio::test]
    async fn with_base_path_strip_changes_the_routing_decision_not_just_the_handler_view() {
        let inner = Router::new().route("/health", get(|| async { StatusCode::OK }));
        let app = with_base_path_strip(inner, Some("/prod".to_string()));

        // The prefixed path must resolve to the handler registered at the unprefixed path —
        // impossible unless the rewrite happened before axum's route matching ran.
        let response = app.clone().oneshot(get_req("/prod/health")).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Negative space: a mismatched sibling segment must not be treated as prefixed, and
        // there is no `/production/health` route, so this must 404, not 200.
        let response = app.oneshot(get_req("/production/health")).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
