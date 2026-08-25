use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Form, Json};
use serde::{Deserialize, Serialize};

use crate::middleware::audit_context::AuditContext;
use crate::state::AppState;
use oidc_exchange_core::service::revoke::RevokeRequest;

#[derive(Deserialize)]
pub struct RevokeForm {
    pub token: String,
    pub token_type_hint: Option<String>,
}

#[derive(Serialize)]
struct RevokeErrorBody {
    error: &'static str,
    error_description: &'static str,
}

pub async fn revoke_handler(
    State(state): State<AppState>,
    Extension(audit_ctx): Extension<AuditContext>,
    Form(form): Form<RevokeForm>,
) -> impl IntoResponse {
    // Validate at the boundary: axum's `Form` extractor accepts an empty
    // `token=` value as a valid (blank) `String`, but a blank token can
    // never verify or hash to a real session. Reject it here as
    // `invalid_request` rather than letting untrusted client input reach
    // the core service's non-empty-token precondition.
    if form.token.is_empty() {
        let body = RevokeErrorBody {
            error: "invalid_request",
            error_description: "the token parameter must not be empty",
        };
        return (StatusCode::BAD_REQUEST, Json(body)).into_response();
    }
    // Precondition, now guaranteed to hold by construction: the blank case
    // returned above, so every remaining line in this handler sees a
    // non-empty token — matches the invariant `AppService::revoke` asserts.
    assert!(
        !form.token.is_empty(),
        "revoke_handler: token must be non-empty past the boundary check"
    );

    let result = state
        .service
        .revoke(RevokeRequest {
            token: form.token,
            token_type_hint: form.token_type_hint,
            client_addr: audit_ctx.client_addr.clone(),
            user_agent: audit_ctx.user_agent,
            device_id: audit_ctx.device_id,
        })
        .await;

    let response: Response = match result {
        // Per RFC 7009: token-state outcomes (revoked, invalid, unknown)
        // always report success toward the client.
        Ok(()) => StatusCode::OK.into_response(),
        // A session-repo/backend failure is not a token-state outcome — log
        // the detail server-side (captured under the request span) and tell
        // the client to retry, without leaking infrastructure detail.
        Err(e) => {
            tracing::error!(error = %e, "revoke: session repository failed");
            let body = RevokeErrorBody {
                error: "server_error",
                error_description: "internal server error",
            };
            (StatusCode::SERVICE_UNAVAILABLE, Json(body)).into_response()
        }
    };
    // Postcondition: this handler only ever produces one of the three
    // documented outcomes — a future refactor that lets some other status
    // leak out (e.g. a bare 200 default) trips this rather than shipping.
    let status = response.status();
    assert!(
        status == StatusCode::OK
            || status == StatusCode::BAD_REQUEST
            || status == StatusCode::SERVICE_UNAVAILABLE,
        "revoke_handler: response must be 200, 400, or 503 — got {status}"
    );
    response
}
