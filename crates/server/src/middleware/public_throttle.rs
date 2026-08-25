use axum::extract::State;
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{header, HeaderValue, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use oidc_exchange_core::domain::{
    AuditFailure, AuditOutcome, RateLimitDecision, RateLimitKey, SecurityEvent,
};
use serde::Deserialize;
use serde::Serialize;
use tokio::sync::Semaphore;

use crate::error::RenderedOAuthErrorCode;
use crate::middleware::audit_context::AuditContext;
use crate::state::AppState;

#[derive(Serialize)]
struct ThrottleBody {
    error: &'static str,
    error_description: &'static str,
}

/// Creates a public-only load-shed layer using a bounded semaphore. `try_acquire_owned`
/// refuses immediately when all permits are in-flight, preventing an unbounded wait queue.
pub fn public_concurrency_layer(
    semaphore: Arc<Semaphore>,
) -> impl Fn(
    Request<axum::body::Body>,
    Next,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Response> + Send>>
       + Clone {
    move |request, next| {
        let semaphore = Arc::clone(&semaphore);
        Box::pin(async move {
            let Ok(permit) = semaphore.try_acquire_owned() else {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(ThrottleBody {
                        error: "server_error",
                        error_description: "service is temporarily overloaded",
                    }),
                )
                    .into_response();
            };
            let response = next.run(request).await;
            drop(permit);
            response
        })
    }
}

/// Applies the normal, server-established IP budget before any public handler or provider work.
pub async fn public_throttle_layer(
    State(state): State<AppState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    if !matches!(request.uri().path(), "/token" | "/revoke") {
        return next.run(request).await;
    }
    let Some(context) = request.extensions().get::<AuditContext>().cloned() else {
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
        Ok(RateLimitDecision::Allow) | Err(_) => {
            let mut response = next.run(request).await;
            if is_public_authentication_failure(&mut response).await {
                let _ = state
                    .rate_limiter
                    .check_and_consume(&RateLimitKey::ClientAddrFailure(address))
                    .await;
            }
            response
        }
        Ok(RateLimitDecision::Deny { retry_after_secs }) => {
            // The throttle response is terminal: audit durability must not turn a safe 429
            // into a different response. `emit_security_event` still records sink degradation
            // and applies the configured mandatory-channel contract internally.
            if let Err(error) = state
                .service
                .emit_security_event(
                    SecurityEvent::ThrottleExceeded,
                    AuditOutcome::Failure(AuditFailure::ThrottleExceeded),
                    None,
                    None,
                    context.client_addr.clone(),
                    context.user_agent.clone(),
                )
                .await
            {
                tracing::error!(error = %error, "mandatory throttle audit emission failed");
            }
            throttle_response(retry_after_secs)
        }
    }
}

/// OAuth errors that prove a request reached core authentication and failed credentials.
/// Boundary parsing failures (`invalid_request`, unsupported grants, malformed forms) are not
/// counted, nor are overload/throttle responses manufactured before the handler runs.
#[derive(Deserialize)]
struct OAuthErrorBody {
    error: String,
}

async fn is_public_authentication_failure(response: &mut Response) -> bool {
    if matches!(
        response.status(),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
    ) {
        return true;
    }
    if response.status() != StatusCode::BAD_REQUEST {
        return false;
    }

    let body = std::mem::replace(response.body_mut(), Body::empty());
    let bytes = match to_bytes(body, 64 * 1024).await {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!(error = %error, "could not inspect authentication failure response body");
            *response.body_mut() = Body::from(
                r#"{"error":"server_error","error_description":"response processing failed"}"#,
            );
            return false;
        }
    };
    let failure = serde_json::from_slice::<OAuthErrorBody>(&bytes)
        .map(|body| body.error == "invalid_grant")
        .unwrap_or(false);
    *response.body_mut() = Body::from(bytes);
    failure
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
        .extensions_mut()
        .insert(RenderedOAuthErrorCode("slow_down"));
    response
}
