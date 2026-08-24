use axum::extract::Request;
use axum::http::{header, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;

/// Insert `Cache-Control: no-store` and `Pragma: no-cache` on the response
/// produced downstream (handler success or `ApiError` error envelope alike).
///
/// RFC 6749 §5.1/§5.2 and OIDC Core §3.1.3.3: the body of a successful token
/// response *is* the credential, and a `200` to a `POST` is heuristically
/// cacheable under RFC 9111 §3, so the header is the origin's sole mechanism
/// for marking the response non-storable. The layer runs after
/// `ApiError::into_response` (a `Router::layer` wraps the route's endpoint),
/// so error envelopes on the same route group are covered too. Responses
/// manufactured by router-wide layers — the timeout `408`, the catch-panic
/// `500` — carry no credential and are deliberately outside this group.
pub async fn no_store_layer(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));

    // Postconditions: both directives are present exactly as issued — a
    // silent header-insertion failure (e.g. a malformed static value) would
    // leave a credential-bearing response cacheable.
    assert_eq!(
        response
            .headers()
            .get(header::CACHE_CONTROL)
            .map(|v| v.as_bytes()),
        Some(&b"no-store"[..]),
        "no_store_layer: Cache-Control: no-store must be set on the response"
    );
    assert_eq!(
        response.headers().get(header::PRAGMA).map(|v| v.as_bytes()),
        Some(&b"no-cache"[..]),
        "no_store_layer: Pragma: no-cache must be set on the response"
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::middleware;
    use axum::routing::{get, post};
    use axum::Router;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    async fn credential_handler() -> (StatusCode, &'static str) {
        (StatusCode::OK, "token-response-body")
    }

    async fn not_found_handler() -> StatusCode {
        StatusCode::NOT_FOUND
    }

    fn app() -> Router {
        Router::new()
            .route("/token", post(credential_handler))
            .route("/missing", get(not_found_handler))
            .layer(middleware::from_fn(no_store_layer))
    }

    #[tokio::test]
    async fn sets_both_directives_on_downstream_success() {
        let response = app()
            .oneshot(Request::post("/token").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            "no-store"
        );
        assert_eq!(response.headers().get(header::PRAGMA).unwrap(), "no-cache");
        // The downstream body passes through unmodified — the layer marks the
        // response, it must not transform it.
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"token-response-body");
    }

    #[tokio::test]
    async fn preserves_downstream_status_while_marking_the_response() {
        let response = app()
            .oneshot(Request::get("/missing").body(Body::empty()).unwrap())
            .await
            .unwrap();

        // An error status from the wrapped handler keeps its status code and
        // still carries both directives — token-endpoint errors are
        // credential-adjacent and must not be cacheable either.
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            "no-store"
        );
        assert_eq!(response.headers().get(header::PRAGMA).unwrap(), "no-cache");
    }
}
