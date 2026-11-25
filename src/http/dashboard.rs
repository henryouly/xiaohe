use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::config::DashboardConfig;
use crate::http::AppState;
use crate::http::auth::{AdminUser, AuthUser, api_error};

/// `GET /api/dashboard` — returns the persisted Dashboard widget layout (or smart default).
pub async fn get_dashboard(_user: AuthUser, State(state): State<AppState>) -> Response {
    match state.config_store.load_dashboard() {
        Ok(cfg) => Json(cfg).into_response(),
        Err(err) => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            Some(err.to_string()),
        ),
    }
}

/// `PUT /api/dashboard` *(Admin)* — validates and atomically persists `config/dashboard.toml`.
pub async fn update_dashboard(
    _admin: AdminUser,
    State(state): State<AppState>,
    Json(cfg): Json<DashboardConfig>,
) -> Response {
    if let Err(errors) = cfg.validate() {
        return api_error(
            StatusCode::BAD_REQUEST,
            "validation_error",
            Some(errors.join("; ")),
        );
    }

    if let Err(err) = state.config_store.save_dashboard(&cfg) {
        return api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            Some(err.to_string()),
        );
    }

    Json(cfg).into_response()
}
