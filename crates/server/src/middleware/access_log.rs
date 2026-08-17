use axum::http::Request;
use axum::middleware::Next;
use axum::response::Response;

use crate::middleware::audit_context::AuditContext;

/// Emits one safe, request-correlated access log record for every public request.
pub const ACCESS_LOG_REQUEST_ID_HEADER: &str = "x-oidc-exchange-access-log-request-id";
pub const ACCESS_LOG_CLIENT_ADDR_SOURCE_HEADER: &str =
    "x-oidc-exchange-access-log-client-addr-source";

pub async fn access_log_layer(request: Request<axum::body::Body>, next: Next) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let client_addr_source = request
        .extensions()
        .get::<AuditContext>()
        .map(|context| format!("{:?}", context.client_addr.source()).to_lowercase())
        .unwrap_or_else(|| "unknown".to_owned());
    let request_id = request
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("unknown")
        .to_owned();
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        ACCESS_LOG_REQUEST_ID_HEADER,
        request_id.parse().expect("validated request id header"),
    );
    response.headers_mut().insert(
        ACCESS_LOG_CLIENT_ADDR_SOURCE_HEADER,
        client_addr_source
            .parse()
            .expect("fixed client address source value"),
    );
    tracing::info!(
        request_id = %request_id,
        method = %method,
        path = %path,
        status = %response.status(),
        client_addr_source = %client_addr_source,
        "public access"
    );
    response
}
