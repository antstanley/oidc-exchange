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

#[cfg(feature = "conformance")]
pub(crate) fn with_base_path_strip_and_observe(
    inner: Router,
    base_path: Option<String>,
    max_request_body_bytes: usize,
) -> Router {
    let service = BasePathStripService {
        inner,
        base_path: base_path.map(Arc::from),
    };
    Router::new().fallback_service(ConformanceBasePathService {
        inner: service,
        max_request_body_bytes,
    })
}

#[cfg(feature = "conformance")]
#[derive(Clone)]
struct ConformanceBasePathService {
    inner: BasePathStripService,
    max_request_body_bytes: usize,
}

#[cfg(feature = "conformance")]
impl Service<Request<Body>> for ConformanceBasePathService {
    type Response = Response;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Response, Infallible>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request<Body>) -> Self::Future {
        let base_path = self.inner.base_path.clone();
        let mut request = request;
        if let Some(path) = request
            .headers_mut()
            .remove("x-oidc-conformance-path")
            .and_then(|value| value.to_str().ok().map(str::to_string))
        {
            request.extensions_mut().insert(ConformancePath(path));
        }
        if request.headers().contains_key("x-oidc-conformance-observe")
            && qualifies_for_conformance_observation(
                request.uri().path(),
                base_path.as_deref(),
                request.extensions().get::<ConformancePath>(),
            )
        {
            let cap = self.max_request_body_bytes;
            Box::pin(async move {
                let request = decode_and_strip_base_path(request, base_path.as_deref());
                Ok(crate::bootstrap::conformance_observe(request, cap).await)
            })
        } else {
            self.inner.call(request)
        }
    }
}

#[cfg(feature = "conformance")]
#[derive(Clone)]
pub struct ConformancePath(pub String);

#[cfg(feature = "conformance")]
fn qualifies_for_conformance_observation(
    path: &str,
    base_path: Option<&str>,
    conformance_path: Option<&ConformancePath>,
) -> bool {
    let path = conformance_path.map_or(path, |path| path.0.as_str());
    if path == "/" {
        return true;
    }
    match base_path.filter(|prefix| !prefix.is_empty() && *prefix != "/") {
        Some(prefix) => {
            strip_prefix_at_segment_boundary(path, prefix).is_some()
                || percent_decode_path(path)
                    .as_deref()
                    .and_then(|decoded| strip_prefix_at_segment_boundary(decoded, prefix))
                    .is_some()
        }
        None => true,
    }
}

#[cfg(feature = "conformance")]
fn percent_decode_path(path: &str) -> Option<String> {
    percent_encoding::percent_decode_str(path)
        .decode_utf8()
        .ok()
        .map(|decoded| decoded.into_owned())
}

#[cfg(feature = "conformance")]
fn decode_and_strip_base_path(request: Request<Body>, base_path: Option<&str>) -> Request<Body> {
    let request = strip_base_path(request, base_path);
    let mut parts = request.uri().clone().into_parts();
    let Some(decoded) = percent_decode_path(request.uri().path()) else {
        return request;
    };
    let path_and_query = match request.uri().query() {
        Some(query) => format!("{decoded}?{query}"),
        None => decoded,
    };
    parts.path_and_query = path_and_query.parse().ok();
    let Some(uri) = axum::http::Uri::from_parts(parts).ok() else {
        return request;
    };
    let (mut request_parts, body) = request.into_parts();
    request_parts.uri = uri;
    Request::from_parts(request_parts, body)
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
pub(crate) struct BasePathStripService {
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
/// This is the pure core [`with_base_path_strip`] wraps: no I/O, just a URI rewrite that is
/// total over host-supplied request data — which is what makes it directly unit-testable
/// without spinning up a router. Every step that could fail on adversarial input (the URI
/// reconstruction in particular) is fallible here and degrades to leaving the request
/// untouched with a structured warning, never to a panic: this middleware runs on every
/// request, so any panic would take the whole service down repeatably rather than failing
/// once, closed, at config load. The outer catch-panic layer in `build_router` backs this up
/// as defence in depth, but nothing on this path is expected to reach it.
fn strip_base_path(mut request: Request<Body>, prefix: Option<&str>) -> Request<Body> {
    // Treat "no prefix" and the two degenerate spellings — empty string and bare root
    // "/" — identically: `Some("")` is not rejected by config deserialization, and
    // `str::strip_prefix("")` would trivially match every path (a no-op strip in effect,
    // since the "stripped" remainder is the whole original path); a `Some("/")` prefix is
    // equally degenerate, since every origin-form path already starts with `/`, and
    // stripping it would rewrite e.g. `//x` to `/x` by consuming one byte of path
    // structure. Config load folds both spellings to unset (`AppConfig::normalise`), so
    // reaching this branch means a hand-built router skipped that step; folding them in
    // here as well keeps a misconfigured degenerate prefix a harmless pass-through instead
    // of a per-request failure mode: this middleware runs on every request, so any panic or
    // wrong-path routing here would hit every request repeatedly rather than failing once,
    // closed, at config load.
    let Some(prefix) = prefix.filter(|p| !p.is_empty() && *p != "/") else {
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

    let Some(new_uri) = rebuild_uri_with_path_and_query(request.uri(), &new_path_and_query) else {
        // Unreachable for a URI that already parsed (the replacement is assembled only
        // from substrings of it), but a hostile or exotic wire value must degrade to
        // pass-through, not unwind: the inner router then routes the original path and
        // answers 404, exactly as it would have without this layer. The raw path/query is
        // deliberately absent from the log — request URLs routinely carry codes, tokens,
        // or PII in their query strings — so only coarse facts are recorded.
        tracing::warn!(
            reason = "rewritten-path-and-query-failed-uri-validation",
            prefix = %prefix,
            path_len_bytes = path.len(),
            had_query = request.uri().query().is_some(),
            "base-path strip discarded; forwarding the request unmodified"
        );
        return request;
    };
    // Checked postcondition (was an `assert_ne!`): a genuine boundary-matched strip removes
    // at least `prefix.len()` bytes or collapses the bare prefix to `/`, so the rewritten
    // path can never equal the original. Reaching this branch would mean the boundary check
    // above let a no-op "match" through — refuse the rewrite and pass the request through
    // instead of trusting it.
    if new_uri.path() == path {
        tracing::warn!(
            reason = "base-path-strip-produced-no-op-rewrite",
            prefix = %prefix,
            path_len_bytes = path.len(),
            "base-path strip was a no-op; forwarding the request unmodified"
        );
        return request;
    }

    // Programmer-error assertion pairing the checks above: unreachable while they hold,
    // kept enabled in tests so an invariant-breaking edit fails loudly there.
    debug_assert!(
        !new_uri.path().is_empty(),
        "rewritten URI path must never be empty, got {:?} from prefix {prefix:?} on {path:?}",
        new_uri.path()
    );

    *request.uri_mut() = new_uri;
    request
}

/// Rebuild `uri` with its path-and-query replaced by `new_path_and_query`, preserving every
/// other component (scheme, authority). Returns `None` when the replacement is not a valid
/// URI path-and-query or the assembled URI fails validation — the caller then passes the
/// request through unmodified instead of panicking on host-supplied bytes.
fn rebuild_uri_with_path_and_query(
    uri: &axum::http::Uri,
    new_path_and_query: &str,
) -> Option<axum::http::Uri> {
    let mut parts = uri.clone().into_parts();
    parts.path_and_query = Some(new_path_and_query.parse().ok()?);
    let new_uri = axum::http::Uri::from_parts(parts).ok()?;
    // Postcondition: the rewritten URI must remain an absolute path the router can match
    // against — never empty, which axum's matcher does not treat as `/`. Returning `None`
    // steers the caller to its pass-through branch.
    if new_uri.path().is_empty() {
        return None;
    }
    Some(new_uri)
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
///
/// Public because this is the repository's single segment-boundary implementation: the
/// server's strip layer applies it here and the FFI request normaliser must reproduce
/// exactly these semantics for embedded hosts, so it reuses this function rather than
/// keeping a second copy that can drift.
pub fn strip_prefix_at_segment_boundary<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
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

    /// The fallible rebuild accepts a well-formed replacement and preserves the URI's other
    /// components (scheme + authority survive when the original was absolute-form), which is
    /// the property every success-path assertion above leans on.
    #[test]
    fn rebuild_uri_with_path_and_query_rebuilds_and_preserves_authority() {
        let uri: axum::http::Uri = "/prod/health?x=1".parse().unwrap();
        let rebuilt =
            rebuild_uri_with_path_and_query(&uri, "/health?x=1").expect("valid rewrite must pass");
        assert_eq!(rebuilt.path(), "/health");
        assert_eq!(rebuilt.query(), Some("x=1"));

        let absolute: axum::http::Uri = "http://backend.internal/prod/keys".parse().unwrap();
        let rebuilt = rebuild_uri_with_path_and_query(&absolute, "/keys")
            .expect("absolute-form rewrite must pass");
        assert_eq!(rebuilt.path(), "/keys");
        assert_eq!(
            rebuilt.authority().map(|a| a.as_str()),
            Some("backend.internal"),
            "only the path-and-query component may change"
        );
    }

    /// Negative space: a replacement that is not a valid URI path-and-query yields `None`
    /// instead of a panic — this is the containment seam the middleware maps to
    /// pass-through.
    #[test]
    fn rebuild_uri_with_path_and_query_rejects_malformed_target() {
        let uri: axum::http::Uri = "/prod/health".parse().unwrap();

        assert!(
            rebuild_uri_with_path_and_query(&uri, "/bad\nnewline").is_none(),
            "a control character must fail URI validation"
        );
        assert!(
            rebuild_uri_with_path_and_query(&uri, "/bad percent").is_none(),
            "an unescaped space must fail URI validation"
        );
    }

    /// Totality over hostile wire data: for every combination of a parseable-but-nasty
    /// request URI and any prefix configuration, stripping either leaves the request
    /// unmodified or produces a non-empty absolute path — and never panics. This is the
    /// regression test for the removed URI-reconstruction panics: a panic anywhere in
    /// `strip_base_path` fails this test.
    #[test]
    fn strip_base_path_is_total_over_hostile_request_uris() {
        let hostile_paths = [
            "/",
            "//",
            "/%2F%2E%2E",
            "/%",
            "/%zz",
            "/seg with space",
            "/authorize",
            "/auth/keys",
            "/prod/../..",
            "http://host.example/prod/health?q=%FF",
            "http://host.example",
            "/x?y=",
            "/x?",
        ];
        let prefixes = [
            None,
            Some(""),
            Some("/"),
            Some("//"),
            Some("/prod"),
            Some("/prod/"),
            Some("/auth"),
        ];

        for raw_path in hostile_paths {
            let Ok(original) = raw_path.parse::<axum::http::Uri>() else {
                continue; // not expressible on the wire at all; nothing to contain
            };
            for prefix in prefixes {
                let request = Request::builder()
                    .uri(original.clone())
                    .body(Body::empty())
                    .unwrap();
                let stripped = strip_base_path(request, prefix);

                let result_path = stripped.uri().path();
                assert!(
                    !result_path.is_empty(),
                    "stripped path must never be empty (input {raw_path:?}, prefix {prefix:?})"
                );
                assert!(
                    result_path.starts_with('/'),
                    "stripped path must stay origin-form (input {raw_path:?}, prefix {prefix:?}), \
                     got {result_path:?}"
                );
            }
        }
    }

    /// Sibling-segment regression at the routing level: an `/auth` base path strips its own
    /// subtree but never mangles the sibling `/authorize` into `orize` — the request passes
    /// through untouched and 404s against the unprefixed routes.
    #[tokio::test]
    async fn sibling_authorize_path_survives_an_auth_base_path_untouched() {
        let inner = Router::new()
            .route("/keys", get(|| async { StatusCode::OK }))
            .route("/health", get(|| async { StatusCode::OK }));
        let app = with_base_path_strip(inner, Some("/auth".to_string()));

        let response = app.clone().oneshot(get_req("/auth/keys")).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "own subtree strips");

        let response = app.oneshot(get_req("/authorize")).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "sibling /authorize must not be treated as prefixed by /auth"
        );
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
