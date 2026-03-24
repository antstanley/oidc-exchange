use axum::http::Request;
use axum::middleware::Next;
use axum::response::IntoResponse;

/// Request-scoped audit metadata extracted from standard HTTP headers.
///
/// Inserted into request extensions by [`audit_context_layer`] so that
/// downstream handlers can access client identity information for audit logs.
#[derive(Clone, Debug, Default)]
pub struct AuditContext {
    /// Client IP address from `X-Forwarded-For`.
    pub ip_address: Option<String>,
    /// Client user-agent string.
    pub user_agent: Option<String>,
    /// Device identifier from `X-Device-Id`.
    pub device_id: Option<String>,
}

/// Middleware that extracts audit-relevant headers and stores them in request
/// extensions as an [`AuditContext`].
pub async fn audit_context_layer(
    mut request: Request<axum::body::Body>,
    next: Next,
) -> impl IntoResponse {
    let ctx = AuditContext {
        ip_address: request
            .headers()
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .map(String::from),
        user_agent: request
            .headers()
            .get("user-agent")
            .and_then(|v| v.to_str().ok())
            .map(String::from),
        device_id: request
            .headers()
            .get("x-device-id")
            .and_then(|v| v.to_str().ok())
            .map(String::from),
    };
    request.extensions_mut().insert(ctx);
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::extract::Extension;
    use axum::http::{Request, StatusCode};
    use axum::middleware;
    use axum::response::Json;
    use axum::routing::get;
    use axum::Router;
    use http_body_util::BodyExt;
    use serde_json::json;
    use tower::ServiceExt;

    async fn echo_ctx(Extension(ctx): Extension<AuditContext>) -> Json<serde_json::Value> {
        Json(json!({
            "ip": ctx.ip_address,
            "ua": ctx.user_agent,
            "device": ctx.device_id,
        }))
    }

    fn app() -> Router {
        Router::new()
            .route("/", get(echo_ctx))
            .layer(middleware::from_fn(audit_context_layer))
    }

    #[tokio::test]
    async fn extracts_all_headers() {
        let response = app()
            .oneshot(
                Request::get("/")
                    .header("x-forwarded-for", "10.0.0.1")
                    .header("user-agent", "test-agent/1.0")
                    .header("x-device-id", "device-abc")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(value["ip"], "10.0.0.1");
        assert_eq!(value["ua"], "test-agent/1.0");
        assert_eq!(value["device"], "device-abc");
    }

    #[tokio::test]
    async fn handles_missing_headers() {
        let response = app()
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert!(value["ip"].is_null());
        // user-agent may be null if not set by the test client
        assert!(value["device"].is_null());
    }
}
