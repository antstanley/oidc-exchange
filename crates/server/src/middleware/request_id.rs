use axum::http::Request;
use axum::middleware::Next;
use axum::response::IntoResponse;

/// Middleware that ensures every request/response carries an `X-Request-Id` header.
///
/// If the incoming request already contains an `X-Request-Id` header it is reused;
/// otherwise a new UUID v4 is generated.  The value is recorded on the current
/// tracing span and propagated back on the response.
pub async fn request_id_layer(
    request: Request<axum::body::Body>,
    next: Next,
) -> impl IntoResponse {
    let request_id = request
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(String::from)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    tracing::Span::current().record("request_id", &request_id.as_str());

    let mut response = next.run(request).await;
    if let Ok(value) = request_id.parse() {
        response.headers_mut().insert("x-request-id", value);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::middleware;
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    async fn ok_handler() -> &'static str {
        "ok"
    }

    fn app() -> Router {
        Router::new()
            .route("/", get(ok_handler))
            .layer(middleware::from_fn(request_id_layer))
    }

    #[tokio::test]
    async fn generates_request_id_when_absent() {
        let response = app()
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let id = response
            .headers()
            .get("x-request-id")
            .expect("should have x-request-id header")
            .to_str()
            .unwrap();

        // Should be a valid UUID v4 (36 chars with dashes)
        assert_eq!(id.len(), 36);
        assert!(uuid::Uuid::parse_str(id).is_ok());
    }

    #[tokio::test]
    async fn preserves_existing_request_id() {
        let response = app()
            .oneshot(
                Request::get("/")
                    .header("x-request-id", "custom-id-123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let id = response
            .headers()
            .get("x-request-id")
            .expect("should have x-request-id header")
            .to_str()
            .unwrap();

        assert_eq!(id, "custom-id-123");
    }
}
