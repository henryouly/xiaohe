use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use crate::config::AreaConfig;
use crate::http::AppState;
use crate::http::auth::{AdminUser, AuthUser, api_error};

#[derive(Debug, Serialize)]
pub struct AreaSummary {
    pub area_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub entity_count: usize,
}

#[derive(Debug, Serialize)]
pub struct AreasListResponse {
    pub areas: Vec<AreaSummary>,
}

#[derive(Debug, Deserialize)]
pub struct CreateAreaRequest {
    pub area_id: String,
    pub name: String,
    #[serde(default)]
    pub icon: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateAreaRequest {
    pub name: String,
    #[serde(default)]
    pub icon: Option<String>,
}

/// `GET /api/areas` — filtered by caller's `allowed_areas` (Admins see all areas).
pub async fn list_areas(
    AuthUser(caller): AuthUser,
    State(state): State<AppState>,
) -> Json<AreasListResponse> {
    let all_entities = state.state.list(&Default::default());
    let configured = state
        .areas
        .read()
        .expect("areas rwlock poisoned")
        .areas
        .clone();

    let areas = configured
        .into_iter()
        .filter(|a| caller.can_access_area(Some(&a.area_id)))
        .map(|a| {
            let entity_count = all_entities
                .iter()
                .filter(|e| e.area_id.as_deref() == Some(a.area_id.as_str()))
                .count();
            AreaSummary {
                area_id: a.area_id,
                name: a.name,
                icon: a.icon,
                entity_count,
            }
        })
        .collect();

    Json(AreasListResponse { areas })
}

/// `POST /api/areas` *(Admin)*
pub async fn create_area(
    _admin: AdminUser,
    State(state): State<AppState>,
    Json(req): Json<CreateAreaRequest>,
) -> Response {
    let area_id = req.area_id.trim();
    if area_id.is_empty()
        || !area_id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
    {
        return api_error(
            StatusCode::BAD_REQUEST,
            "validation_error",
            Some("area_id must be non-empty lowercase ASCII [a-z0-9_-]".into()),
        );
    }
    if req.name.trim().is_empty() {
        return api_error(
            StatusCode::BAD_REQUEST,
            "validation_error",
            Some("name must not be empty".into()),
        );
    }

    let area = AreaConfig {
        area_id: area_id.to_owned(),
        name: req.name.trim().to_owned(),
        icon: req.icon,
    };

    let mut guard = state.areas.write().expect("areas rwlock poisoned");
    if guard.areas.iter().any(|a| a.area_id == area.area_id) {
        return api_error(
            StatusCode::CONFLICT,
            "area_exists",
            Some(format!("area '{}' already exists", area.area_id)),
        );
    }
    guard.areas.push(area.clone());
    if let Err(err) = state.config_store.save_areas(&guard) {
        guard.areas.pop();
        return api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            Some(err.to_string()),
        );
    }

    (
        StatusCode::CREATED,
        Json(serde_json::json!({ "area": area })),
    )
        .into_response()
}

/// `PUT /api/areas/{area_id}` *(Admin)*
pub async fn update_area(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(area_id): Path<String>,
    Json(req): Json<UpdateAreaRequest>,
) -> Response {
    if req.name.trim().is_empty() {
        return api_error(
            StatusCode::BAD_REQUEST,
            "validation_error",
            Some("name must not be empty".into()),
        );
    }

    let mut guard = state.areas.write().expect("areas rwlock poisoned");
    let Some(area) = guard.areas.iter_mut().find(|a| a.area_id == area_id) else {
        return api_error(StatusCode::NOT_FOUND, "not_found", None);
    };
    let prev = area.clone();
    area.name = req.name.trim().to_owned();
    area.icon = req.icon;
    let updated = area.clone();

    if let Err(err) = state.config_store.save_areas(&guard) {
        if let Some(a) = guard.areas.iter_mut().find(|a| a.area_id == area_id) {
            *a = prev;
        }
        return api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            Some(err.to_string()),
        );
    }

    (StatusCode::OK, Json(serde_json::json!({ "area": updated }))).into_response()
}

/// `DELETE /api/areas/{area_id}` *(Admin)*
pub async fn delete_area(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(area_id): Path<String>,
) -> Response {
    let mut guard = state.areas.write().expect("areas rwlock poisoned");
    let Some(pos) = guard.areas.iter().position(|a| a.area_id == area_id) else {
        return api_error(StatusCode::NOT_FOUND, "not_found", None);
    };
    let removed = guard.areas.remove(pos);
    if let Err(err) = state.config_store.save_areas(&guard) {
        guard.areas.insert(pos, removed);
        return api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            Some(err.to_string()),
        );
    }
    StatusCode::NO_CONTENT.into_response()
}
