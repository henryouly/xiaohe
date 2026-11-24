use axum::Json;
use axum::extract::{FromRequestParts, State};
use axum::http::header::{AUTHORIZATION, COOKIE, SET_COOKIE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use crate::auth::{AuthError, AuthenticatedUser, SESSION_COOKIE_NAME, UserSummary};
use crate::http::AppState;

/// Shared JSON error response helper for HTTP API endpoints.
pub fn api_error(status: StatusCode, code: &'static str, detail: Option<String>) -> Response {
    let body = match detail {
        Some(msg) => serde_json::json!({ "error": code, "message": msg }),
        None => serde_json::json!({ "error": code }),
    };
    (status, Json(body)).into_response()
}

pub fn map_auth_error(err: AuthError) -> Response {
    match err {
        AuthError::AlreadyInitialized => {
            api_error(StatusCode::CONFLICT, "already_initialized", None)
        }
        AuthError::InvalidCredentials => api_error(StatusCode::UNAUTHORIZED, "unauthorized", None),
        AuthError::UserExists(u) => api_error(
            StatusCode::CONFLICT,
            "user_exists",
            Some(format!("username '{u}' already exists")),
        ),
        AuthError::UserNotFound(u) => api_error(StatusCode::NOT_FOUND, "not_found", Some(u)),
        AuthError::TokenNotFound(t) => api_error(StatusCode::NOT_FOUND, "not_found", Some(t)),
        AuthError::LastAdminProtected => api_error(
            StatusCode::CONFLICT,
            "last_admin_protected",
            Some("cannot remove or demote the last remaining admin".into()),
        ),
        AuthError::Validation(msg) => {
            api_error(StatusCode::BAD_REQUEST, "validation_error", Some(msg))
        }
        AuthError::Config(e) => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            Some(e.to_string()),
        ),
        AuthError::Hash(e) => {
            api_error(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", Some(e))
        }
    }
}

/// Axum extractor enforcing valid session cookie or Bearer token authentication (`401`).
#[derive(Debug, Clone)]
pub struct AuthUser(pub AuthenticatedUser);

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        resolve_user_from_headers(&parts.headers, state)
            .map(AuthUser)
            .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "unauthorized", None))
    }
}

/// Axum extractor enforcing authenticated `Admin` role (`401` if unauthenticated, `403` otherwise).
#[derive(Debug, Clone)]
pub struct AdminUser(pub AuthenticatedUser);

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let AuthUser(user) = AuthUser::from_request_parts(parts, state).await?;
        if !user.is_admin() {
            return Err(api_error(StatusCode::FORBIDDEN, "forbidden", None));
        }
        Ok(AdminUser(user))
    }
}

/// Optional authenticated user (never rejects; used by `GET /api/auth/me`).
#[derive(Debug, Clone)]
pub struct OptionalAuthUser(pub Option<AuthenticatedUser>);

impl FromRequestParts<AppState> for OptionalAuthUser {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        Ok(OptionalAuthUser(resolve_user_from_headers(
            &parts.headers,
            state,
        )))
    }
}

fn resolve_user_from_headers(headers: &HeaderMap, state: &AppState) -> Option<AuthenticatedUser> {
    // 1. Prefer Authorization: Bearer <token>
    if let Some(auth_val) = headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok())
        && let Some((scheme, token)) = auth_val.trim().split_once(char::is_whitespace)
        && scheme.eq_ignore_ascii_case("bearer")
    {
        let token = token.trim();
        if !token.is_empty() {
            return state.auth.authenticate_bearer(token);
        }
    }

    // 2. Fall back to HttpOnly session cookie `xiaohe_session`
    if let Some(sid) = extract_session_cookie(headers) {
        return state.auth.authenticate_session(&sid);
    }

    None
}

pub fn extract_session_cookie(headers: &HeaderMap) -> Option<String> {
    for raw in headers.get_all(COOKIE) {
        let Ok(s) = raw.to_str() else { continue };
        for pair in s.split(';') {
            let Some((k, v)) = pair.trim().split_once('=') else {
                continue;
            };
            if k.trim() == SESSION_COOKIE_NAME {
                let val = v.trim();
                if !val.is_empty() {
                    return Some(val.to_owned());
                }
            }
        }
    }
    None
}

fn build_session_cookie(session_id: &str) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "{SESSION_COOKIE_NAME}={session_id}; HttpOnly; SameSite=Lax; Path=/"
    ))
    .expect("valid session cookie header")
}

fn clear_session_cookie() -> HeaderValue {
    HeaderValue::from_static("xiaohe_session=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0")
}

#[derive(Debug, Deserialize)]
pub struct BootstrapRequest {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub display_name: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct AuthUserResponse {
    pub user: UserSummary,
}

#[derive(Debug, Serialize)]
pub struct AuthMeResponse {
    pub initialized: bool,
    pub user: Option<UserSummary>,
}

/// `POST /api/auth/bootstrap` — first-boot initial Admin creation.
pub async fn bootstrap(
    State(state): State<AppState>,
    Json(req): Json<BootstrapRequest>,
) -> Response {
    match state
        .auth
        .bootstrap_admin(&req.username, &req.password, &req.display_name)
    {
        Ok((user, sid)) => {
            let mut resp = (StatusCode::CREATED, Json(AuthUserResponse { user })).into_response();
            resp.headers_mut()
                .insert(SET_COOKIE, build_session_cookie(&sid));
            resp
        }
        Err(err) => map_auth_error(err),
    }
}

/// `POST /api/auth/login` — verify Argon2id password and set `xiaohe_session` cookie.
pub async fn login(State(state): State<AppState>, Json(req): Json<LoginRequest>) -> Response {
    match state.auth.login(&req.username, &req.password) {
        Ok((user, sid)) => {
            let mut resp = (StatusCode::OK, Json(AuthUserResponse { user })).into_response();
            resp.headers_mut()
                .insert(SET_COOKIE, build_session_cookie(&sid));
            resp
        }
        Err(err) => map_auth_error(err),
    }
}

/// `POST /api/auth/logout` — revoke current session and clear cookie.
pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(sid) = extract_session_cookie(&headers) {
        state.auth.logout(&sid);
    }
    let mut resp = StatusCode::NO_CONTENT.into_response();
    resp.headers_mut()
        .insert(SET_COOKIE, clear_session_cookie());
    resp
}

/// `GET /api/auth/me` — public SPA auth guard check.
pub async fn me(
    State(state): State<AppState>,
    OptionalAuthUser(caller): OptionalAuthUser,
) -> Json<AuthMeResponse> {
    let initialized = state.auth.is_initialized();
    let user = caller.and_then(|u| state.auth.get_user_summary(&u.id));
    Json(AuthMeResponse { initialized, user })
}
