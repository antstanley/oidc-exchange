//! Internal-API authentication middleware.
//!
//! Authenticates `/internal/*` requests into a named
//! [`OperatorPrincipal`] and inserts it as a request extension for the
//! handlers. The mechanisms come from `internal_api.auth_methods`, tried in
//! configured order by the [`OperatorAuthGate`] built at startup.
//!
//! Ordering contract (`04-http-api.md` → Middleware stack):
//!
//! 1. **Throttle first.** Before any credential is evaluated, the layer
//!    consults the `RateLimitKey::OperatorAuth(peer)` budget keyed by the
//!    connection's real peer address — a `ClientAddr::Peer`, never a
//!    forwarded or asserted one, because the admin listener sits behind no
//!    untrusted proxy. A lockout short-circuits to `429` with `Retry-After`
//!    and emits one `ThrottleExceeded` security event.
//! 2. **Credentials.** A success inserts the principal extension and serves
//!    the request; it never draws down the budget. A failure consumes one
//!    unit from the budget and emits exactly one
//!    `OperatorAuthenticationFailed` security event carrying a fixed reason
//!    (`missing_credential` | `invalid_credential` | `not_configured`) plus a
//!    `tracing::warn!` inside the request span — the presented credential is
//!    never recorded anywhere.

use axum::extract::State;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::middleware::operator_auth::{auth_event_detail, AuthInput};
use crate::state::AppState;
use oidc_exchange_core::domain::{
    ClientAddr, OperatorAuthFailureReason, RateLimitKey, SecurityEvent,
};

/// Response header carrying the remaining lockout seconds on a 429.
const RETRY_AFTER_HEADER: &str = "Retry-After";

/// Middleware that authenticates the operator behind an `/internal/*`
/// request and attaches the resulting [`OperatorPrincipal`] extension.
pub async fn internal_auth_layer(
    State(state): State<AppState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let gate = state.operator_auth.as_ref().map(|g| g.as_ref());
    let gate = match gate {
        Some(gate) => gate,
        None => {
            // The layer only mounts where the admin plane is served, and a
            // served plane always has a gate — reaching here is a wiring bug,
            // not a runtime condition, so fail closed rather than serve an
            // unauthenticated internal API.
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "server_error",
                    "error_description": "operator authentication is not wired",
                })),
            )
                .into_response();
        }
    };

    // Peer provenance comes from the socket via the connect-info make-service;
    // runtimes without connection info yield Unknown, which draws no budget.
    // That fail-open is deliberate but never silent: bootstrap warns when the
    // admin plane is served on such a runtime (an API-Gateway-fronted
    // function *is* an externally reachable guessing surface) that per-peer
    // lockout and peer-attributed audit are inactive there.
    let client_addr = request
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|ci| ci.0.ip())
        .map(ClientAddr::Peer)
        .unwrap_or(ClientAddr::Unknown);
    let peer_key = client_addr.rate_limit_key().map(RateLimitKey::OperatorAuth);

    // 1. Consult the failed-auth budget before evaluating any credential.
    if let Some(key) = &peer_key {
        match state.service.rate_limiter().check(key).await {
            Ok(oidc_exchange_core::domain::RateLimitDecision::Allow) => {}
            Ok(oidc_exchange_core::domain::RateLimitDecision::Deny { retry_after_secs }) => {
                let route = request.uri().path().to_string();
                emit_security_event_or_log(
                    &state,
                    SecurityEvent::ThrottleExceeded,
                    &client_addr,
                    &route,
                )
                .await;
                tracing::warn!(
                    route = %route,
                    retry_after_secs,
                    "admin-plane auth budget exhausted; denying before credential evaluation"
                );
                let mut response = (
                    StatusCode::TOO_MANY_REQUESTS,
                    Json(json!({
                        "error": "slow_down",
                        "error_description": "too many requests; retry later",
                    })),
                )
                    .into_response();
                if let Ok(value) = retry_after_secs.to_string().parse() {
                    response.headers_mut().insert(RETRY_AFTER_HEADER, value);
                }
                return response;
            }
            Err(err) => {
                tracing::error!(error = %err, "rate limiter failed during consultation");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({
                        "error": "server_error",
                        "error_description": "internal server error",
                    })),
                )
                    .into_response();
            }
        }
    }

    // 2. Try the configured mechanisms in order.
    let bearer = bearer_token(request.headers());
    let input = AuthInput::from_parts(bearer.as_deref(), request.headers());
    let route = request.uri().path().to_string();

    match gate.authenticate(&input).await {
        Ok(principal) => {
            let mut request = request;
            request.extensions_mut().insert(principal);
            next.run(request).await
        }
        Err(reason) => {
            // A unit is consumed only by a failed attempt; successes never
            // reach this arm, so working credentials draw nothing down.
            if let Some(key) = &peer_key {
                if let Err(err) = state.service.rate_limiter().consume(key).await {
                    tracing::error!(error = %err, "rate limiter failed recording an auth failure");
                }
            }
            emit_security_event_or_log(
                &state,
                SecurityEvent::OperatorAuthenticationFailed { reason },
                &client_addr,
                &route,
            )
            .await;
            tracing::warn!(reason = reason.as_str(), route = %route, "operator authentication failed");
            unauthorized_response(reason)
        }
    }
}

/// The presented bearer token, if any (`Authorization: Bearer <token>`).
fn bearer_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string)
}

/// Emit a mandatory-channel security event; on sink failure, honour the
/// blocking-threshold contract by surfacing a 500 (the event's severity meets
/// the default threshold), otherwise continue having logged the fallback.
async fn emit_security_event_or_log(
    state: &AppState,
    event: SecurityEvent,
    client_addr: &ClientAddr,
    route: &str,
) {
    if let Err(err) = state
        .service
        .emit_security_event(
            event,
            oidc_exchange_core::domain::AuditOutcome::Failure {
                reason: match event {
                    SecurityEvent::OperatorAuthenticationFailed { reason } => {
                        reason.as_str().to_string()
                    }
                    SecurityEvent::ThrottleExceeded => {
                        oidc_exchange_core::domain::security_failure_reasons::THROTTLE_EXCEEDED
                            .to_string()
                    }
                },
            },
            None,
            client_addr.clone(),
            auth_event_detail(route),
        )
        .await
    {
        tracing::error!(error = %err, "failed to persist operator-auth security event");
    }
}

/// Map a rejection reason to its 401 response. Descriptions are fixed strings:
/// they explain what class of thing went wrong without leaking which
/// mechanism rejected the attempt or any part of the presented credential.
fn unauthorized_response(reason: OperatorAuthFailureReason) -> Response {
    let description = match reason {
        OperatorAuthFailureReason::MissingCredential => "authentication required",
        OperatorAuthFailureReason::InvalidCredential => "invalid credential",
        OperatorAuthFailureReason::NotConfigured => "internal API authentication is not configured",
    };
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({
            "error": "unauthorized",
            "error_description": description,
        })),
    )
        .into_response()
}
