use std::fs;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use crate::automation::{AutomationConfig, ValidationContext, validate_automation};
use crate::config::atomic_write;
use crate::http::AppState;
use crate::http::auth::{AdminUser, api_error};

#[derive(Debug, Deserialize)]
pub struct RawTomlRequest {
    pub raw_toml: String,
}

#[derive(Debug, Deserialize)]
pub struct ToggleAutomationRequest {
    pub enabled: bool,
}

#[derive(Debug, Default, Deserialize)]
pub struct TriggerAutomationRequest {
    #[serde(default)]
    pub skip_conditions: bool,
}

#[derive(Debug, Default, Deserialize)]
pub struct AutomationRunsQuery {
    #[serde(default)]
    pub automation_id: Option<String>,
    #[serde(default = "default_runs_limit")]
    pub limit: u32,
    #[serde(default)]
    pub offset: u32,
}

fn default_runs_limit() -> u32 {
    50
}

fn validation_context(state: &AppState) -> ValidationContext {
    let mut ctx = ValidationContext::default();
    for ent in state.state.list(&Default::default()) {
        ctx.known_entities.insert(ent.entity_id);
    }
    ctx
}

/// `GET /api/automations` *(Admin)*
pub async fn list_automations(_admin: AdminUser, State(state): State<AppState>) -> Response {
    let Some(engine) = &state.automations else {
        return (StatusCode::OK, Json(json!({ "automations": [] }))).into_response();
    };

    let rules = engine.list_rules();
    let mut items = Vec::with_capacity(rules.len());
    for rule in rules {
        let file = engine
            .get_rule_file(&rule.id)
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| format!("{}.toml", rule.id));
        let last_run = state
            .history
            .query_automation_runs(Some(&rule.id), 1, 0)
            .await
            .ok()
            .and_then(|mut v| v.pop());
        items.push(json!({
            "id": rule.id,
            "name": rule.name,
            "enabled": rule.enabled,
            "mode": rule.mode,
            "file": file,
            "last_run_at": last_run.as_ref().map(|r| r.started_at),
            "last_outcome": last_run.as_ref().map(|r| r.outcome),
        }));
    }

    (StatusCode::OK, Json(json!({ "automations": items }))).into_response()
}

/// `GET /api/automations/{id}` *(Admin)*
pub async fn get_automation(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let Some(engine) = &state.automations else {
        return api_error(StatusCode::NOT_FOUND, "not_found", None);
    };
    let Some(rule) = engine.list_rules().into_iter().find(|r| r.id == id) else {
        return api_error(StatusCode::NOT_FOUND, "not_found", None);
    };
    let Some(path) = engine.get_rule_file(&id) else {
        return api_error(StatusCode::NOT_FOUND, "not_found", None);
    };
    let raw_toml = fs::read_to_string(&path).unwrap_or_default();
    let file = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| format!("{id}.toml"));

    (
        StatusCode::OK,
        Json(json!({
            "id": rule.id,
            "file": file,
            "parsed": rule,
            "raw_toml": raw_toml,
        })),
    )
        .into_response()
}

/// `POST /api/automations` *(Admin)* — validate raw TOML, atomic write, hot-reload.
pub async fn create_automation(
    _admin: AdminUser,
    State(state): State<AppState>,
    Json(req): Json<RawTomlRequest>,
) -> Response {
    let Some(engine) = &state.automations else {
        return api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "automations_unavailable",
            None,
        );
    };

    let parsed: AutomationConfig = match toml::from_str(&req.raw_toml) {
        Ok(c) => c,
        Err(err) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({
                    "error": "validation_failed",
                    "details": [{ "line": 1, "message": err.to_string() }]
                })),
            )
                .into_response();
        }
    };

    let ctx = validation_context(&state);
    let warnings = match validate_automation(&parsed, Some(&ctx)) {
        Ok(w) => w,
        Err(reasons) => {
            let details: Vec<_> = reasons
                .into_iter()
                .map(|m| json!({ "line": 1, "message": m }))
                .collect();
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "error": "validation_failed", "details": details })),
            )
                .into_response();
        }
    };

    if engine.get_rule_file(&parsed.id).is_some() {
        return api_error(
            StatusCode::CONFLICT,
            "automation_exists",
            Some(format!("automation '{}' already exists", parsed.id)),
        );
    }

    let dest = engine.automations_dir().join(format!("{}.toml", parsed.id));
    if dest.exists() {
        return api_error(
            StatusCode::CONFLICT,
            "automation_file_exists",
            Some(format!(
                "file '{}.toml' already exists for another automation",
                parsed.id
            )),
        );
    }
    if let Err(err) = atomic_write(&dest, &req.raw_toml, false) {
        return api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            Some(err.to_string()),
        );
    }
    engine.reload();

    let warning_msgs: Vec<String> = warnings.into_iter().map(|w| w.message).collect();
    (
        StatusCode::CREATED,
        Json(json!({ "id": parsed.id, "warnings": warning_msgs })),
    )
        .into_response()
}

/// `PUT /api/automations/{id}` *(Admin)* — validate raw TOML, atomic write, hot-reload.
pub async fn update_automation(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<RawTomlRequest>,
) -> Response {
    let Some(engine) = &state.automations else {
        return api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "automations_unavailable",
            None,
        );
    };

    let Some(dest) = engine.get_rule_file(&id) else {
        return api_error(StatusCode::NOT_FOUND, "not_found", None);
    };

    let parsed: AutomationConfig = match toml::from_str(&req.raw_toml) {
        Ok(c) => c,
        Err(err) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({
                    "error": "validation_failed",
                    "details": [{ "line": 1, "message": err.to_string() }]
                })),
            )
                .into_response();
        }
    };

    if parsed.id != id {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "validation_failed",
                "details": [{ "line": 1, "message": format!("TOML id '{}' does not match path id '{id}'", parsed.id) }]
            })),
        )
            .into_response();
    }

    let ctx = validation_context(&state);
    let warnings = match validate_automation(&parsed, Some(&ctx)) {
        Ok(w) => w,
        Err(reasons) => {
            let details: Vec<_> = reasons
                .into_iter()
                .map(|m| json!({ "line": 1, "message": m }))
                .collect();
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "error": "validation_failed", "details": details })),
            )
                .into_response();
        }
    };

    if let Err(err) = atomic_write(&dest, &req.raw_toml, false) {
        return api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            Some(err.to_string()),
        );
    }
    engine.reload();

    let warning_msgs: Vec<String> = warnings.into_iter().map(|w| w.message).collect();
    (
        StatusCode::OK,
        Json(json!({ "id": parsed.id, "warnings": warning_msgs })),
    )
        .into_response()
}

/// `POST /api/automations/{id}/toggle` *(Admin)*
pub async fn toggle_automation(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ToggleAutomationRequest>,
) -> Response {
    let Some(engine) = &state.automations else {
        return api_error(StatusCode::NOT_FOUND, "not_found", None);
    };
    let Some(path) = engine.get_rule_file(&id) else {
        return api_error(StatusCode::NOT_FOUND, "not_found", None);
    };
    let Some(mut rule) = engine.list_rules().into_iter().find(|r| r.id == id) else {
        return api_error(StatusCode::NOT_FOUND, "not_found", None);
    };
    rule.enabled = req.enabled;
    let serialized = match toml::to_string_pretty(&rule) {
        Ok(s) => s,
        Err(err) => {
            return api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                Some(err.to_string()),
            );
        }
    };
    if let Err(err) = atomic_write(&path, &serialized, false) {
        return api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            Some(err.to_string()),
        );
    }
    engine.reload();
    (
        StatusCode::OK,
        Json(json!({ "id": id, "enabled": req.enabled })),
    )
        .into_response()
}

/// `POST /api/automations/{id}/trigger` *(Admin)*
pub async fn trigger_automation(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Option<Json<TriggerAutomationRequest>>,
) -> Response {
    let Some(engine) = &state.automations else {
        return api_error(StatusCode::NOT_FOUND, "not_found", None);
    };
    let skip_conditions = body.map(|b| b.0.skip_conditions).unwrap_or(false);
    match engine
        .trigger_manual_with_options(&id, skip_conditions)
        .await
    {
        Ok(run_id) => (
            StatusCode::OK,
            Json(json!({ "run_id": run_id, "outcome": "started" })),
        )
            .into_response(),
        Err(msg) => api_error(StatusCode::NOT_FOUND, "not_found", Some(msg)),
    }
}

/// `DELETE /api/automations/{id}` *(Admin)*
pub async fn delete_automation(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let Some(engine) = &state.automations else {
        return api_error(StatusCode::NOT_FOUND, "not_found", None);
    };
    let Some(path) = engine.get_rule_file(&id) else {
        return api_error(StatusCode::NOT_FOUND, "not_found", None);
    };
    if let Err(err) = fs::remove_file(&path) {
        return api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            Some(err.to_string()),
        );
    }
    engine.reload();
    StatusCode::NO_CONTENT.into_response()
}

/// `POST /api/automations/reload` *(Admin)*
pub async fn reload_automations(_admin: AdminUser, State(state): State<AppState>) -> Response {
    let Some(engine) = &state.automations else {
        return (StatusCode::OK, Json(json!({ "loaded": 0, "errors": [] }))).into_response();
    };
    let (loaded, errors) = engine.reload();
    let err_msgs: Vec<String> = errors.into_iter().map(|e| e.to_string()).collect();
    (
        StatusCode::OK,
        Json(json!({ "loaded": loaded.len(), "errors": err_msgs })),
    )
        .into_response()
}

/// `GET /api/automations/runs?automation_id=...&limit=50&offset=0` *(Admin)*
pub async fn list_automation_runs(
    _admin: AdminUser,
    State(state): State<AppState>,
    Query(q): Query<AutomationRunsQuery>,
) -> Response {
    let limit = q.limit.clamp(1, 500);
    match state
        .history
        .query_automation_runs(q.automation_id.as_deref(), limit, q.offset)
        .await
    {
        Ok(runs) => (StatusCode::OK, Json(json!({ "runs": runs }))).into_response(),
        Err(err) => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "history_error",
            Some(err.to_string()),
        ),
    }
}
