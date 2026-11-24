use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use crate::auth::{CreateUserInput, MintedApiToken, UpdateUserInput, UserSummary};
use crate::config::{AllowedAreas, UserRole};
use crate::http::AppState;
use crate::http::auth::{AdminUser, map_auth_error};

#[derive(Debug, Serialize)]
pub struct UsersListResponse {
    pub users: Vec<UserSummary>,
}

#[derive(Debug, Serialize)]
pub struct UserItemResponse {
    pub user: UserSummary,
}

#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    #[serde(default)]
    pub display_name: String,
    pub password: String,
    pub role: UserRole,
    #[serde(default)]
    pub allowed_areas: AllowedAreas,
    #[serde(default)]
    pub allow_guest_locks: bool,
}

#[derive(Debug, Deserialize)]
pub struct UpdateUserRequest {
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub role: Option<UserRole>,
    #[serde(default)]
    pub allowed_areas: Option<AllowedAreas>,
    #[serde(default)]
    pub allow_guest_locks: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct MintTokenRequest {
    pub name: String,
}

/// `GET /api/users` *(Admin)*
pub async fn list_users(
    _admin: AdminUser,
    State(state): State<AppState>,
) -> Json<UsersListResponse> {
    Json(UsersListResponse {
        users: state.auth.list_users(),
    })
}

/// `POST /api/users` *(Admin)*
pub async fn create_user(
    _admin: AdminUser,
    State(state): State<AppState>,
    Json(req): Json<CreateUserRequest>,
) -> Response {
    let input = CreateUserInput {
        username: req.username,
        display_name: req.display_name,
        password: req.password,
        role: req.role,
        allowed_areas: req.allowed_areas,
        allow_guest_locks: req.allow_guest_locks,
    };
    match state.auth.create_user(input) {
        Ok(user) => (StatusCode::CREATED, Json(UserItemResponse { user })).into_response(),
        Err(err) => map_auth_error(err),
    }
}

/// `PUT /api/users/{user_id}` *(Admin)*
pub async fn update_user(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(user_id): Path<String>,
    Json(req): Json<UpdateUserRequest>,
) -> Response {
    let input = UpdateUserInput {
        display_name: req.display_name,
        password: req.password,
        role: req.role,
        allowed_areas: req.allowed_areas,
        allow_guest_locks: req.allow_guest_locks,
    };
    match state.auth.update_user(&user_id, input) {
        Ok(user) => (StatusCode::OK, Json(UserItemResponse { user })).into_response(),
        Err(err) => map_auth_error(err),
    }
}

/// `DELETE /api/users/{user_id}` *(Admin)*
pub async fn delete_user(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> Response {
    match state.auth.delete_user(&user_id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => map_auth_error(err),
    }
}

/// `POST /api/users/{user_id}/tokens` *(Admin)* — plaintext token returned once.
pub async fn mint_token(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(user_id): Path<String>,
    Json(req): Json<MintTokenRequest>,
) -> Response {
    match state.auth.mint_token(&user_id, &req.name) {
        Ok(minted) => (StatusCode::CREATED, Json::<MintedApiToken>(minted)).into_response(),
        Err(err) => map_auth_error(err),
    }
}

/// `DELETE /api/users/{user_id}/tokens/{token_id}` *(Admin)*
pub async fn revoke_token(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path((user_id, token_id)): Path<(String, String)>,
) -> Response {
    match state.auth.revoke_token(&user_id, &token_id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => map_auth_error(err),
    }
}
