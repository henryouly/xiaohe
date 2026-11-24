use std::collections::BTreeMap;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::config::SkillUserConfig;
use crate::http::AppState;
use crate::http::auth::{AdminUser, api_error};
use crate::protocol::domain::EntitySource;
use crate::protocol::rpc::SkillInvokeParams;
use crate::protocol::skill::{SkillMode, SkillStatus};
use crate::skills::toml_config_to_json;

#[derive(Debug, Serialize)]
pub struct SkillApiSummary {
    pub skill_id: String,
    pub name: String,
    pub version: String,
    pub mode: SkillMode,
    pub enabled: bool,
    pub status: SkillStatus,
    pub restart_count: u32,
    pub last_error: Option<String>,
    pub entity_count: usize,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSkillConfigRequest {
    #[serde(default)]
    pub config: Map<String, Value>,
    #[serde(default)]
    pub secrets: BTreeMap<String, String>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
pub struct LogsQuery {
    #[serde(default = "default_tail")]
    pub tail: usize,
}

#[derive(Debug, Deserialize)]
pub struct InvokeSkillRequest {
    pub action: String,
    #[serde(default)]
    pub params: Map<String, Value>,
}

fn default_tail() -> usize {
    200
}

/// `GET /api/skills` *(Admin)*
pub async fn list_skills(_admin: AdminUser, State(state): State<AppState>) -> Response {
    let Some(bridge) = &state.skills else {
        return api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "runtime_unavailable",
            Some("skill bridge is disabled".into()),
        );
    };
    let summaries = match bridge.list_skills().await {
        Ok(s) => s,
        Err(err) => {
            return api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "runtime_unavailable",
                Some(err.to_string()),
            );
        }
    };

    let all_entities = state.state.list(&Default::default());
    let devices = state.devices.read().expect("devices rwlock poisoned");

    let skills: Vec<SkillApiSummary> = summaries
        .into_iter()
        .map(|s| {
            let enabled = devices
                .skills
                .get(&s.skill_id)
                .map(|c| c.enabled)
                .unwrap_or(false);
            let entity_count = all_entities
                .iter()
                .filter(|e| matches!(&e.source, EntitySource::Skill { skill_id } if skill_id == &s.skill_id))
                .count();
            SkillApiSummary {
                skill_id: s.skill_id,
                name: s.name,
                version: s.version,
                mode: s.mode,
                enabled,
                status: s.status,
                restart_count: s.restart_count,
                last_error: s.last_error,
                entity_count,
            }
        })
        .collect();

    (StatusCode::OK, Json(json!({ "skills": skills }))).into_response()
}

/// `GET /api/skills/{skill_id}` *(Admin)* — never returns plaintext secrets; only `is_set: bool`.
pub async fn get_skill(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(skill_id): Path<String>,
) -> Response {
    let Some(bridge) = &state.skills else {
        return api_error(StatusCode::SERVICE_UNAVAILABLE, "runtime_unavailable", None);
    };
    let summaries = match bridge.list_skills().await {
        Ok(s) => s,
        Err(err) => {
            return api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "runtime_unavailable",
                Some(err.to_string()),
            );
        }
    };
    let Some(summary) = summaries.into_iter().find(|s| s.skill_id == skill_id) else {
        return api_error(StatusCode::NOT_FOUND, "not_found", None);
    };

    let (enabled, config_json) = {
        let devices = state.devices.read().expect("devices rwlock poisoned");
        match devices.skills.get(&skill_id) {
            Some(c) => (c.enabled, toml_config_to_json(&c.config)),
            None => (false, Map::new()),
        }
    };

    let secrets_status: BTreeMap<String, Value> = {
        let secrets = state.secrets.read().expect("secrets rwlock poisoned");
        summary
            .declared_secrets
            .iter()
            .map(|k| (k.clone(), json!({ "is_set": secrets.get(k).is_some() })))
            .collect()
    };

    (
        StatusCode::OK,
        Json(json!({
            "skill_id": summary.skill_id,
            "name": summary.name,
            "version": summary.version,
            "mode": summary.mode,
            "enabled": enabled,
            "status": summary.status,
            "restart_count": summary.restart_count,
            "last_error": summary.last_error,
            "config": config_json,
            "secrets": secrets_status,
        })),
    )
        .into_response()
}

/// `PUT /api/skills/{skill_id}/config` *(Admin)*
pub async fn update_skill_config(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(skill_id): Path<String>,
    Json(req): Json<UpdateSkillConfigRequest>,
) -> Response {
    let mut toml_map = BTreeMap::new();
    for (k, v) in &req.config {
        match serde_json::from_value::<toml::Value>(v.clone()) {
            Ok(tv) => {
                toml_map.insert(k.clone(), tv);
            }
            Err(err) => {
                return api_error(
                    StatusCode::BAD_REQUEST,
                    "validation_error",
                    Some(format!("invalid config value for '{k}': {err}")),
                );
            }
        }
    }

    // Persist non-secret config to config/devices.toml
    {
        let mut devices = state.devices.write().expect("devices rwlock poisoned");
        let entry = devices
            .skills
            .entry(skill_id.clone())
            .or_insert_with(SkillUserConfig::default);
        if let Some(en) = req.enabled {
            entry.enabled = en;
        }
        entry.config = toml_map;
        if let Err(err) = state.config_store.save_devices(&devices) {
            return api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                Some(err.to_string()),
            );
        }
    }

    // Persist write-only secrets to 0600 config/secrets.toml and hot-update SkillBridge
    if !req.secrets.is_empty() {
        let updated_secrets = {
            let mut secrets = state.secrets.write().expect("secrets rwlock poisoned");
            for (k, v) in req.secrets {
                if !v.is_empty() {
                    secrets.set(k, v);
                }
            }
            if let Err(err) = state.config_store.save_secrets(&secrets) {
                return api_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    Some(err.to_string()),
                );
            }
            secrets.clone()
        };
        if let Some(bridge) = &state.skills {
            bridge.update_secrets(updated_secrets);
        }
    }

    // Reload running device skill if connected
    let mut reloaded = false;
    if let Some(bridge) = &state.skills
        && bridge.is_connected()
        && let Ok(list) = bridge.list_skills().await
        && list.iter().any(|s| {
            s.skill_id == skill_id
                && s.mode == SkillMode::Device
                && s.status == SkillStatus::Running
        })
        && bridge.start_skill(&skill_id, req.config).await.is_ok()
    {
        reloaded = true;
    }

    (
        StatusCode::OK,
        Json(json!({ "ok": true, "reloaded": reloaded })),
    )
        .into_response()
}

/// `POST /api/skills/{skill_id}/start` *(Admin)*
pub async fn start_skill(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(skill_id): Path<String>,
) -> Response {
    let Some(bridge) = &state.skills else {
        return api_error(StatusCode::SERVICE_UNAVAILABLE, "runtime_unavailable", None);
    };
    let config = {
        let devices = state.devices.read().expect("devices rwlock poisoned");
        devices
            .skills
            .get(&skill_id)
            .map(|c| toml_config_to_json(&c.config))
            .unwrap_or_default()
    };
    match bridge.start_skill(&skill_id, config).await {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({ "skill_id": skill_id, "status": "running" })),
        )
            .into_response(),
        Err(err) => api_error(
            StatusCode::BAD_REQUEST,
            "skill_error",
            Some(err.to_string()),
        ),
    }
}

/// `POST /api/skills/{skill_id}/stop` *(Admin)*
pub async fn stop_skill(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(skill_id): Path<String>,
) -> Response {
    let Some(bridge) = &state.skills else {
        return api_error(StatusCode::SERVICE_UNAVAILABLE, "runtime_unavailable", None);
    };
    match bridge.stop_skill(&skill_id).await {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({ "skill_id": skill_id, "status": "stopped" })),
        )
            .into_response(),
        Err(err) => api_error(
            StatusCode::BAD_REQUEST,
            "skill_error",
            Some(err.to_string()),
        ),
    }
}

/// `POST /api/skills/{skill_id}/restart` *(Admin)*
pub async fn restart_skill(
    admin: AdminUser,
    State(state): State<AppState>,
    Path(skill_id): Path<String>,
) -> Response {
    // Supervisor's start_device_skill stops any existing instance before starting fresh.
    start_skill(admin, State(state), Path(skill_id)).await
}

/// `POST /api/skills/{skill_id}/reinstall` *(Admin)*
pub async fn reinstall_skill(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(skill_id): Path<String>,
) -> Response {
    let Some(bridge) = &state.skills else {
        return api_error(StatusCode::SERVICE_UNAVAILABLE, "runtime_unavailable", None);
    };
    match bridge.reinstall_skill(&skill_id).await {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({ "skill_id": skill_id, "status": "stopped" })),
        )
            .into_response(),
        Err(err) => api_error(
            StatusCode::BAD_REQUEST,
            "skill_error",
            Some(err.to_string()),
        ),
    }
}

/// `GET /api/skills/{skill_id}/logs?tail=200` *(Admin)*
pub async fn get_skill_logs(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(skill_id): Path<String>,
    Query(q): Query<LogsQuery>,
) -> Response {
    let Some(bridge) = &state.skills else {
        return api_error(StatusCode::SERVICE_UNAVAILABLE, "runtime_unavailable", None);
    };
    let tail = q.tail.clamp(1, 500);
    match bridge.get_logs(&skill_id, tail).await {
        Ok(lines) => (
            StatusCode::OK,
            Json(json!({ "skill_id": skill_id, "lines": lines })),
        )
            .into_response(),
        Err(err) => api_error(
            StatusCode::BAD_REQUEST,
            "skill_error",
            Some(err.to_string()),
        ),
    }
}

/// `POST /api/skills/{skill_id}/invoke` *(Admin)*
pub async fn invoke_skill(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(skill_id): Path<String>,
    Json(req): Json<InvokeSkillRequest>,
) -> Response {
    let Some(bridge) = &state.skills else {
        return api_error(StatusCode::SERVICE_UNAVAILABLE, "runtime_unavailable", None);
    };
    let config = {
        let devices = state.devices.read().expect("devices rwlock poisoned");
        devices
            .skills
            .get(&skill_id)
            .map(|c| toml_config_to_json(&c.config))
            .unwrap_or_default()
    };
    let invoke = SkillInvokeParams {
        action: req.action,
        params: req.params,
        context: Map::new(),
    };
    match bridge.invoke_skill(&skill_id, config, invoke).await {
        Ok(result) => (
            StatusCode::OK,
            Json(json!({ "ok": true, "result": result })),
        )
            .into_response(),
        Err(err) => api_error(
            StatusCode::BAD_REQUEST,
            "skill_error",
            Some(err.to_string()),
        ),
    }
}
