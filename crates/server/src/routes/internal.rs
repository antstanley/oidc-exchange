use std::collections::HashMap;

use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{middleware, Json, Router};
use serde::Serialize;
use serde_json::Value;

use crate::error::ApiError;
use crate::middleware::internal_auth::internal_auth_layer;
use crate::state::AppState;
use oidc_exchange_core::domain::{NewUser, OperatorPrincipal, UserPatch};
use oidc_exchange_core::error::Error;

/// Build the internal API surface with relative route paths, behind the
/// operator-auth layer.
///
/// The auth layer is scoped to *this* router only: callers mount it under
/// `/internal` via [`routes::internal_routes`] (`nest`), so the layer can never
/// wrap the admin listener's other routes or its fallback — an unmatched path
/// on the admin plane must render a routing-level 404, not an authentication
/// rejection, and `/health` must stay reachable without a credential.
///
/// The layer inserts the authenticated [`OperatorPrincipal`] as a request
/// extension; every mutating handler below takes it as an `Extension`
/// extractor and threads it into its service call, so attribution always
/// records *the principal that was actually authenticated* — never a value the
/// handler chose for itself. A request reaching these handlers without the
/// extension is a wiring bug; extraction fails with 500 rather than mutating
/// data unattributed.
pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/stats", get(stats))
        .route("/users", get(list_users).post(create_user))
        .route("/sessions/cleanup", post(cleanup_sessions))
        .route(
            "/users/{id}",
            get(get_user).patch(update_user).delete(delete_user),
        )
        .route(
            "/users/{id}/claims",
            get(get_claims)
                .put(set_claims)
                .patch(merge_claims)
                .delete(clear_claims),
        )
        .layer(middleware::from_fn_with_state(state, internal_auth_layer))
}

// ---------------------------------------------------------------------------
// Stats
// ---------------------------------------------------------------------------

pub async fn stats(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let stats = state.service.admin_stats().await?;
    Ok(Json(stats))
}

// ---------------------------------------------------------------------------
// Session-store cleanup
// ---------------------------------------------------------------------------

/// Response body of `POST /internal/sessions/cleanup`: only the deleted count.
/// No session, token, hash, or subject data ever appears here — the endpoint
/// is an operator lever and a scheduler target, not an inspection surface
/// (`04-http-api.md` → Internal routes).
#[derive(Debug, Serialize)]
pub struct CleanupResponse {
    pub deleted: u64,
}

/// Run `cleanup_expired_sessions` once — the same sweep the bootstrap-spawned
/// session reaper runs on its interval — for runtimes that cannot host a
/// periodic task (Lambda above all) and as the operator's manual lever. Safe
/// to invoke on any schedule alongside a running reaper: it mutates nothing
/// but expired rows (`04-http-api.md` → Bootstrap step 7 / Internal routes).
pub async fn cleanup_sessions(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ApiError> {
    let deleted = state.service.cleanup_expired_sessions().await?;
    Ok(Json(CleanupResponse { deleted }))
}

// ---------------------------------------------------------------------------
// User list
// ---------------------------------------------------------------------------

/// The `GET /internal/users` query contract, exactly as the published schema
/// documents it: an opaque `cursor` and a `limit` the core clamps.
///
/// `offset` is *removed*, not deprecated — a caller that still sends one gets
/// a deterministic rejection naming the replacement rather than a silently
/// ignored parameter that would appear to work while always starting from the
/// first page. The field exists on this struct only so its presence can be
/// detected; serde ignores unknown fields, so without it an old caller would
/// never learn it was speaking a dead contract.
#[derive(serde::Deserialize)]
pub struct ListUsersQuery {
    cursor: Option<String>,
    limit: Option<u32>,
    #[serde(rename = "offset")]
    removed_offset: Option<String>,
}

pub async fn list_users(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<ListUsersQuery>,
) -> Result<impl IntoResponse, ApiError> {
    // Negative space: an explicit `offset` (even `offset=0`) is refused with
    // the migration message instead of being honoured or dropped.
    if query.removed_offset.is_some() {
        return Err(Error::InvalidRequest {
            reason: "the offset parameter has been removed; page with cursor/limit".to_string(),
        }
        .into());
    }

    let page = state
        .service
        .admin_list_users(query.cursor.as_deref(), query.limit)
        .await?;
    Ok(Json(page))
}

// ---------------------------------------------------------------------------
// User CRUD
// ---------------------------------------------------------------------------

pub async fn create_user(
    State(state): State<AppState>,
    Extension(operator): Extension<OperatorPrincipal>,
    Json(new_user): Json<NewUser>,
) -> Result<impl IntoResponse, ApiError> {
    let user = state
        .service
        .admin_create_user(&operator, &new_user)
        .await?;
    Ok((StatusCode::CREATED, Json(user)))
}

pub async fn get_user(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let user = state.service.admin_get_user(&id).await?;
    match user {
        Some(u) => Ok(Json(serde_json::to_value(u).unwrap()).into_response()),
        None => Ok((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "not_found",
                "error_description": format!("user not found: {}", id),
            })),
        )
            .into_response()),
    }
}

pub async fn update_user(
    State(state): State<AppState>,
    Extension(operator): Extension<OperatorPrincipal>,
    Path(id): Path<String>,
    Json(patch): Json<UserPatch>,
) -> Result<impl IntoResponse, ApiError> {
    let user = state
        .service
        .admin_update_user(&operator, &id, &patch)
        .await?;
    Ok(Json(user))
}

pub async fn delete_user(
    State(state): State<AppState>,
    Extension(operator): Extension<OperatorPrincipal>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    state.service.admin_delete_user(&operator, &id).await?;
    Ok(StatusCode::OK)
}

// ---------------------------------------------------------------------------
// Claims management
// ---------------------------------------------------------------------------

pub async fn get_claims(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let claims = state.service.admin_get_claims(&id).await?;
    Ok(Json(claims))
}

pub async fn set_claims(
    State(state): State<AppState>,
    Extension(operator): Extension<OperatorPrincipal>,
    Path(id): Path<String>,
    Json(claims): Json<HashMap<String, Value>>,
) -> Result<impl IntoResponse, ApiError> {
    state
        .service
        .admin_set_claims(&operator, &id, claims)
        .await?;
    Ok(StatusCode::OK)
}

pub async fn merge_claims(
    State(state): State<AppState>,
    Extension(operator): Extension<OperatorPrincipal>,
    Path(id): Path<String>,
    Json(claims): Json<HashMap<String, Value>>,
) -> Result<impl IntoResponse, ApiError> {
    state
        .service
        .admin_merge_claims(&operator, &id, claims)
        .await?;
    Ok(StatusCode::OK)
}

pub async fn clear_claims(
    State(state): State<AppState>,
    Extension(operator): Extension<OperatorPrincipal>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    state.service.admin_clear_claims(&operator, &id).await?;
    Ok(StatusCode::OK)
}
