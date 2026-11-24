use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::protocol::domain::{Availability, DomainState};

pub const PROTOCOL_VERSION: &str = "1.0";
pub const JSONRPC_VERSION: &str = "2.0";

// Standard JSON-RPC 2.0 error codes.
pub const ERR_PARSE_ERROR: i32 = -32700;
pub const ERR_INVALID_REQUEST: i32 = -32600;
pub const ERR_METHOD_NOT_FOUND: i32 = -32601;
pub const ERR_INVALID_PARAMS: i32 = -32602;
pub const ERR_INTERNAL_ERROR: i32 = -32603;

/// JSON-RPC 2.0 request/response identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RpcId {
    Number(i64),
    String(String),
}

/// JSON-RPC 2.0 request expecting a response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcRequest {
    pub jsonrpc: String,
    pub id: RpcId,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl RpcRequest {
    pub fn new(id: RpcId, method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            id,
            method: method.into(),
            params,
        }
    }
}

/// JSON-RPC 2.0 notification (no `id`, no response expected).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl RpcNotification {
    pub fn new(method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            method: method.into(),
            params,
        }
    }
}

/// JSON-RPC 2.0 error object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// JSON-RPC 2.0 response containing either `result` or `error`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcResponse {
    pub jsonrpc: String,
    pub id: Option<RpcId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl RpcResponse {
    pub fn ok(id: RpcId, result: Value) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            id: Some(id),
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: Option<RpcId>, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            id,
            result: None,
            error: Some(RpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }
}

/// Parameters for `skill.init` handshake (Core/Supervisor -> Skill).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillInitParams {
    pub protocol_version: String,
    pub skill_id: String,
    #[serde(default)]
    pub config: Map<String, Value>,
    #[serde(default)]
    pub secrets: BTreeMap<String, String>,
}

/// Standard `{ "ok": true }` acknowledgment payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OkResult {
    pub ok: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRegistration {
    pub device_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityRegistration {
    pub entity_id: String,
    pub name: String,
    pub domain: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_class: Option<String>,
}

/// Parameters for `entity.register` notification (Skill -> Core).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityRegisterParams {
    pub device: DeviceRegistration,
    pub entities: Vec<EntityRegistration>,
}

/// Parameters for `entity.state_changed` notification (Skill -> Core).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityStateChangedParams {
    pub entity_id: String,
    pub availability: Availability,
    pub state: DomainState,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

/// Parameters for `entity.command` request (Core -> Skill).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityCommandParams {
    pub entity_id: String,
    pub action: String,
    #[serde(default)]
    pub params: Map<String, Value>,
}

/// Parameters for `skill.invoke` request (Core -> Action Skill).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillInvokeParams {
    pub action: String,
    #[serde(default)]
    pub params: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub context: Map<String, Value>,
}

/// Parameters for `supervisor.start_skill` (Core -> Supervisor over UDS).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SupervisorStartSkillParams {
    pub skill_id: String,
    #[serde(default)]
    pub config: Map<String, Value>,
    #[serde(default)]
    pub secrets: BTreeMap<String, String>,
}

/// Parameters for `supervisor.stop_skill` / `supervisor.reinstall_skill` (Core -> Supervisor).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupervisorSkillIdParams {
    pub skill_id: String,
}

/// Parameters for `supervisor.get_logs` (Core -> Supervisor).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupervisorGetLogsParams {
    pub skill_id: String,
    #[serde(default = "default_log_tail")]
    pub tail: usize,
}

/// Parameters for `supervisor.entity_command` (Core -> Supervisor).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SupervisorEntityCommandParams {
    pub skill_id: String,
    pub command: EntityCommandParams,
}

/// Parameters for `supervisor.invoke_skill` (Core -> Supervisor).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SupervisorInvokeSkillParams {
    pub skill_id: String,
    #[serde(default)]
    pub config: Map<String, Value>,
    #[serde(default)]
    pub secrets: BTreeMap<String, String>,
    pub invoke: SkillInvokeParams,
}

/// Parameters for `skill.status_changed` notification (Supervisor -> Core).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillStatusChangedParams {
    pub skill_id: String,
    pub status: crate::protocol::skill::SkillStatus,
    pub restart_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

/// Parameters for `skill.event` notification forwarding a Skill's notification to Core.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillForwardedEventParams {
    pub skill_id: String,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// Parameters for `core.get_entity` callback (Skill -> Supervisor -> Core).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreGetEntityParams {
    pub entity_id: String,
}

/// Parameters for `core.call_entity` callback (Skill -> Supervisor -> Core).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoreCallEntityParams {
    pub entity_id: String,
    pub action: String,
    #[serde(default)]
    pub params: Map<String, Value>,
}

fn default_log_tail() -> usize {
    100
}
