use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::http::AppState;
use crate::http::auth::{AuthUser, api_error};
use crate::protocol::domain::{EntityRecord, EntitySource};
use crate::state::{EntityFilter, now_unix_ms};
use crate::storage::TimeRange;

const DEFAULT_HISTORY_WINDOW_MS: i64 = 24 * 60 * 60 * 1_000;

#[derive(Debug, Default, Deserialize)]
pub struct EntityQuery {
    #[serde(default)]
    pub area_id: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct EntitiesListResponse {
    pub entities: Vec<EntityRecord>,
}

#[derive(Debug, Deserialize)]
pub struct EntityCommandRequest {
    pub action: String,
    #[serde(default)]
    pub params: Map<String, Value>,
}

#[derive(Debug, Default, Deserialize)]
pub struct HistoryQuery {
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub bucket_secs: Option<u32>,
}

/// `GET /api/entities?area_id=...&domain=...&source=...` — strictly filtered by caller's `allowed_areas`.
pub async fn list_entities(
    AuthUser(caller): AuthUser,
    State(state): State<AppState>,
    Query(q): Query<EntityQuery>,
) -> Json<EntitiesListResponse> {
    let filter = EntityFilter {
        area_id: q.area_id,
        domain: q.domain,
        ..Default::default()
    };
    let source_filter = q.source.as_deref();
    let entities = state
        .state
        .list(&filter)
        .into_iter()
        .filter(|e| caller.can_access_area(e.area_id.as_deref()))
        .filter(|e| match source_filter {
            None => true,
            Some("esphome") => matches!(e.source, EntitySource::EspHome { .. }),
            Some("skill") => matches!(e.source, EntitySource::Skill { .. }),
            Some(specific) => match &e.source {
                EntitySource::EspHome { node_id } => {
                    specific == node_id || specific == format!("esphome:{node_id}")
                }
                EntitySource::Skill { skill_id } => {
                    specific == skill_id || specific == format!("skill:{skill_id}")
                }
            },
        })
        .collect();
    Json(EntitiesListResponse { entities })
}

/// `GET /api/entities/{entity_id}` — returns `404` if unknown or `403` if outside `allowed_areas`.
pub async fn get_entity(
    AuthUser(caller): AuthUser,
    State(state): State<AppState>,
    Path(entity_id): Path<String>,
) -> Response {
    let Some(entity) = state.state.get(&entity_id) else {
        return api_error(StatusCode::NOT_FOUND, "not_found", None);
    };
    if !caller.can_access_area(entity.area_id.as_deref()) {
        return api_error(StatusCode::FORBIDDEN, "forbidden", None);
    }
    (StatusCode::OK, Json(entity)).into_response()
}

/// `POST /api/entities/{entity_id}/command` — RBAC area & Guest lock check + dispatch.
pub async fn command_entity(
    AuthUser(caller): AuthUser,
    State(state): State<AppState>,
    Path(entity_id): Path<String>,
    Json(req): Json<EntityCommandRequest>,
) -> Response {
    if req.action.trim().is_empty() {
        return api_error(
            StatusCode::BAD_REQUEST,
            "validation_error",
            Some("action must not be empty".into()),
        );
    }
    let Some(entity) = state.state.get(&entity_id) else {
        return api_error(StatusCode::NOT_FOUND, "not_found", None);
    };
    if !caller.can_command_entity(&entity, &req.action) {
        return api_error(StatusCode::FORBIDDEN, "forbidden", None);
    }

    match state
        .executor
        .call_entity(&entity_id, &req.action, req.params)
        .await
    {
        Ok(_) => (
            StatusCode::OK,
            Json(json!({ "ok": true, "entity_id": entity_id })),
        )
            .into_response(),
        Err(msg) => {
            let lower = msg.to_ascii_lowercase();
            let status = if lower.contains("timeout") {
                StatusCode::GATEWAY_TIMEOUT
            } else if lower.contains("offline")
                || lower.contains("not connected")
                || lower.contains("not running")
                || lower.contains("disconnected")
            {
                StatusCode::SERVICE_UNAVAILABLE
            } else {
                StatusCode::BAD_REQUEST
            };
            api_error(status, "command_failed", Some(msg))
        }
    }
}

/// `GET /api/entities/{entity_id}/history?from=...&to=...&bucket_secs=...`
pub async fn entity_history(
    AuthUser(caller): AuthUser,
    State(state): State<AppState>,
    Path(entity_id): Path<String>,
    Query(q): Query<HistoryQuery>,
) -> Response {
    if let Some(entity) = state.state.get(&entity_id) {
        if !caller.can_access_area(entity.area_id.as_deref()) {
            return api_error(StatusCode::FORBIDDEN, "forbidden", None);
        }
    } else if !caller.is_admin() {
        return api_error(StatusCode::NOT_FOUND, "not_found", None);
    }

    let now = now_unix_ms();
    let to_ms = match q.to.as_deref() {
        Some(s) => match parse_timestamp_ms(s) {
            Ok(ms) => ms,
            Err(e) => return api_error(StatusCode::BAD_REQUEST, "invalid_query", Some(e)),
        },
        None => now,
    };
    let from_ms = match q.from.as_deref() {
        Some(s) => match parse_timestamp_ms(s) {
            Ok(ms) => ms,
            Err(e) => return api_error(StatusCode::BAD_REQUEST, "invalid_query", Some(e)),
        },
        None => to_ms.saturating_sub(DEFAULT_HISTORY_WINDOW_MS),
    };

    let range = match TimeRange::new(from_ms, to_ms) {
        Ok(r) => r,
        Err(e) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "invalid_query",
                Some(e.to_string()),
            );
        }
    };

    match state
        .history
        .query_entity_history(&entity_id, range, q.bucket_secs)
        .await
    {
        Ok(series) => (StatusCode::OK, Json(series)).into_response(),
        Err(e) => api_error(
            StatusCode::BAD_REQUEST,
            "history_error",
            Some(e.to_string()),
        ),
    }
}

fn parse_timestamp_ms(raw: &str) -> Result<i64, String> {
    let trimmed = raw.trim();
    if let Ok(ms) = trimmed.parse::<i64>() {
        return Ok(ms);
    }
    DateTime::parse_from_rfc3339(trimmed)
        .map(|dt| dt.timestamp_millis())
        .map_err(|_| format!("invalid timestamp '{raw}' (expected RFC3339 or Unix ms)"))
}
