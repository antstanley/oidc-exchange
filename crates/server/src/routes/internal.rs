use std::collections::HashMap;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{middleware, Json, Router};
use serde_json::Value;

use crate::error::ApiError;
use crate::middleware::internal_auth::internal_auth_layer;
use crate::state::AppState;
use oidc_exchange_core::domain::{NewUser, UserPatch};

/// Build the internal API router with shared-secret auth middleware.
pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/internal/stats", get(stats))
        .route("/internal/users", get(list_users).post(create_user))
        .route(
            "/internal/users/{id}",
            get(get_user).patch(update_user).delete(delete_user),
        )
        .route(
            "/internal/users/{id}/claims",
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
// User list
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
pub struct ListUsersQuery {
    offset: Option<u64>,
    limit: Option<u64>,
}

pub async fn list_users(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<ListUsersQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(50).min(200);
    let users = state.service.admin_list_users(offset, limit).await?;
    Ok(Json(users))
}

// ---------------------------------------------------------------------------
// User CRUD
// ---------------------------------------------------------------------------

pub async fn create_user(
    State(state): State<AppState>,
    Json(new_user): Json<NewUser>,
) -> Result<impl IntoResponse, ApiError> {
    let user = state.service.admin_create_user(&new_user).await?;
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
    Path(id): Path<String>,
    Json(patch): Json<UserPatch>,
) -> Result<impl IntoResponse, ApiError> {
    let user = state.service.admin_update_user(&id, &patch).await?;
    Ok(Json(user))
}

pub async fn delete_user(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    state.service.admin_delete_user(&id).await?;
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
    Path(id): Path<String>,
    Json(claims): Json<HashMap<String, Value>>,
) -> Result<impl IntoResponse, ApiError> {
    state.service.admin_set_claims(&id, claims).await?;
    Ok(StatusCode::OK)
}

pub async fn merge_claims(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(claims): Json<HashMap<String, Value>>,
) -> Result<impl IntoResponse, ApiError> {
    state.service.admin_merge_claims(&id, claims).await?;
    Ok(StatusCode::OK)
}

pub async fn clear_claims(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    state.service.admin_clear_claims(&id).await?;
    Ok(StatusCode::OK)
}
