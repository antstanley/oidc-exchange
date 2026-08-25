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
//!
//! Both events travel the *mandatory* audit channel. When the channel's sink
//! fails at a severity meeting `[audit] blocking_threshold` (default
//! `warning`; both events warn), the blocking contract applies and the
//! request fails closed with a generic `500` rather than being answered on a
//! silently-dropped mandatory record.

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
                if require_security_event(
                    &state,
                    SecurityEvent::ThrottleExceeded,
                    &client_addr,
                    &route,
                )
                .await
                .is_err()
                {
                    // The lockout event could not be durably recorded at a
                    // severity meeting the blocking threshold: fail closed.
                    return audit_blocking_error_response();
                }
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
            if require_security_event(
                &state,
                SecurityEvent::OperatorAuthenticationFailed { reason },
                &client_addr,
                &route,
            )
            .await
            .is_err()
            {
                // The mandatory event could not be recorded at a severity
                // meeting the blocking threshold: fail closed instead of
                // answering 401 on a dropped record.
                return audit_blocking_error_response();
            }
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

/// Emit a mandatory-channel security event, honouring the
/// blocking-threshold contract. These events warn, which meets the default
/// `[audit] blocking_threshold`, so a sink failure surfaces here as `Err` —
/// `emit_security_event` has already logged the fallback rendering — and the
/// caller must fail the request closed (500) rather than serve on a
/// silently-dropped mandatory record. Below-threshold sink failures never
/// reach the caller: they log-and-continue inside `emit_security_event`.
async fn require_security_event(
    state: &AppState,
    event: SecurityEvent,
    client_addr: &ClientAddr,
    route: &str,
) -> Result<(), oidc_exchange_core::error::Error> {
    let failure = match event {
        SecurityEvent::OperatorAuthenticationFailed { reason } => reason.audit_failure(),
        _ => oidc_exchange_core::domain::AuditFailure::ThrottleExceeded,
    };
    state
        .service
        .emit_security_event_with_detail(
            event,
            oidc_exchange_core::domain::AuditOutcome::Failure(failure),
            None,
            None,
            client_addr.clone(),
            None,
            auth_event_detail(route),
        )
        .await
}


/// The generic 500 surfaced when a mandatory security event cannot be durably
/// recorded at a severity meeting `[audit] blocking_threshold`. Deliberately
/// opaque: no audit-adapter internals reach the wire.
fn audit_blocking_error_response() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({
            "error": "server_error",
            "error_description": "internal server error",
        })),
    )
        .into_response()
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
