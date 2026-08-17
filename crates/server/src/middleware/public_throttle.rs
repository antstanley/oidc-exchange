use axum::extract::State;
use axum::http::{header, HeaderValue, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use oidc_exchange_core::domain::{RateLimitDecision, RateLimitKey};
use serde::Serialize;

use crate::middleware::audit_context::AuditContext;
use crate::state::AppState;

#[derive(Serialize)]
struct ThrottleBody {
    error: &'static str,
    error_description: &'static str,
}

/// Applies the normal, server-established IP budget before any public handler or provider work.
pub async fn public_throttle_layer(
    State(state): State<AppState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let Some(context) = request.extensions().get::<AuditContext>() else {
        return next.run(request).await;
    };
    let Some(address) = context.client_addr.rate_limit_key() else {
        return next.run(request).await;
    };
    match state
        .rate_limiter
        .check_and_consume(&RateLimitKey::ClientAddr(address))
        .await
    {
        Ok(RateLimitDecision::Allow) | Err(_) => next.run(request).await,
        Ok(RateLimitDecision::Deny { retry_after_secs }) => throttle_response(retry_after_secs),
    }
}

fn throttle_response(retry_after_secs: u64) -> Response {
    let mut response = (
        StatusCode::TOO_MANY_REQUESTS,
        Json(ThrottleBody {
            error: "slow_down",
            error_description: "too many authentication attempts",
        }),
    )
        .into_response();
    let value = HeaderValue::from_str(&retry_after_secs.max(1).to_string())
        .expect("positive retry-after seconds always form a valid header value");
    response.headers_mut().insert(header::RETRY_AFTER, value);
    response
}
