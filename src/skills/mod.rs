use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use serde_json::{Map, Value};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::config::{SecretsConfig, SkillUserConfig};
use crate::protocol::domain::{
    Availability, ClimateMode, CoverState, DomainState, EntityRecord, EntitySource, LockState,
    MediaState, SensorValue,
};
use crate::protocol::rpc::{
    CoreCallEntityParams, CoreGetEntityParams, ERR_INTERNAL_ERROR, ERR_INVALID_PARAMS,
    ERR_METHOD_NOT_FOUND, EntityCommandParams, EntityRegisterParams, EntityStateChangedParams,
    RpcId, RpcNotification, RpcRequest, RpcResponse, SkillForwardedEventParams, SkillInvokeParams,
    SkillStatusChangedParams, SupervisorEntityCommandParams, SupervisorGetLogsParams,
    SupervisorInvokeSkillParams, SupervisorSkillIdParams, SupervisorStartSkillParams,
};
use crate::protocol::skill::{SkillMode, SkillStatus};
use crate::state::{CoreEvent, EventBus, StateStore, now_unix_ms};
use crate::supervisor::{LogEntry, SkillRuntimeSummary};

const INITIAL_RECONNECT_BACKOFF: Duration = Duration::from_millis(250);
const MAX_RECONNECT_BACKOFF: Duration = Duration::from_secs(5);
const DEFAULT_RPC_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Error)]
pub enum SkillBridgeError {
    #[error("skill runtime supervisor is not connected")]
    Disconnected,
    #[error("JSON-RPC error (code {code}): {message}")]
    Rpc { code: i32, message: String },
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("timeout waiting for supervisor response to '{0}'")]
    Timeout(String),
}

/// Strictly resolve only the secret keys declared in a skill's `skill.toml` `[secrets.*]`.
///
/// Keys not listed in `declared_keys` are never passed to the supervisor or child skill process.
pub fn resolve_scoped_secrets(
    declared_keys: &[String],
    secrets: &SecretsConfig,
) -> BTreeMap<String, String> {
    let mut scoped = BTreeMap::new();
    for key in declared_keys {
        if let Some(val) = secrets.get(key) {
            scoped.insert(key.clone(), val);
        }
    }
    scoped
}

/// Convert a skill's `config` table from `devices.toml` (`toml::Value`) to JSON `Map`.
pub fn toml_config_to_json(config: &BTreeMap<String, toml::Value>) -> Map<String, Value> {
    let mut out = Map::new();
    for (k, v) in config {
        if let Ok(jv) = serde_json::to_value(v) {
            out.insert(k.clone(), jv);
        }
    }
    out
}

type PendingMap = StdMutex<HashMap<RpcId, oneshot::Sender<Result<Value, SkillBridgeError>>>>;

struct SkillBridgeInner {
    socket_path: PathBuf,
    state: StateStore,
    events: EventBus,
    secrets: std::sync::RwLock<SecretsConfig>,
    skill_configs: Arc<BTreeMap<String, SkillUserConfig>>,
    connected: AtomicBool,
    next_id: AtomicI64,
    out_tx: StdMutex<Option<mpsc::UnboundedSender<String>>>,
    pending: PendingMap,
    known_skills: StdMutex<HashMap<String, SkillRuntimeSummary>>,
    rpc_timeout: Duration,
}

/// Core-side bidirectional JSON-RPC 2.0 client connected to `xiaohe supervisor` over UDS.
#[derive(Clone)]
pub struct SkillBridge {
    inner: Arc<SkillBridgeInner>,
    task: Arc<StdMutex<Option<JoinHandle<()>>>>,
}

impl SkillBridge {
    /// Spawn the background UDS connection & reconnect loop for `xiaohe core`.
    pub fn start(
        socket_path: impl Into<PathBuf>,
        skill_configs: BTreeMap<String, SkillUserConfig>,
        secrets: SecretsConfig,
        state: StateStore,
        events: EventBus,
        shutdown_rx: watch::Receiver<bool>,
    ) -> Self {
        let inner = Arc::new(SkillBridgeInner {
            socket_path: socket_path.into(),
            state,
            events,
            secrets: std::sync::RwLock::new(secrets),
            skill_configs: Arc::new(skill_configs),
            connected: AtomicBool::new(false),
            next_id: AtomicI64::new(1),
            out_tx: StdMutex::new(None),
            pending: StdMutex::new(HashMap::new()),
            known_skills: StdMutex::new(HashMap::new()),
            rpc_timeout: DEFAULT_RPC_TIMEOUT,
        });

        let bridge = Self {
            inner: Arc::clone(&inner),
            task: Arc::new(StdMutex::new(None)),
        };

        let loop_bridge = bridge.clone();
        let handle = tokio::spawn(async move {
            run_bridge_loop(loop_bridge, shutdown_rx).await;
        });

        *bridge.task.lock().expect("task mutex poisoned") = Some(handle);
        bridge
    }

    pub fn is_connected(&self) -> bool {
        self.inner.connected.load(Ordering::Acquire)
    }

    /// List all skills discovered by the supervisor along with runtime status and declared secrets.
    pub async fn list_skills(&self) -> Result<Vec<SkillRuntimeSummary>, SkillBridgeError> {
        let val = self.call("supervisor.list_skills", None).await?;
        let summaries: Vec<SkillRuntimeSummary> = serde_json::from_value(val)?;
        {
            let mut known = self
                .inner
                .known_skills
                .lock()
                .expect("known_skills mutex poisoned");
            known.clear();
            for s in &summaries {
                known.insert(s.skill_id.clone(), s.clone());
            }
        }
        Ok(summaries)
    }

    /// Trigger a manifest rescan in the supervisor.
    pub async fn rescan_skills(&self) -> Result<Value, SkillBridgeError> {
        let val = self.call("supervisor.rescan", None).await?;
        if let Some(skills_val) = val.get("skills")
            && let Ok(summaries) =
                serde_json::from_value::<Vec<SkillRuntimeSummary>>(skills_val.clone())
        {
            let mut known = self
                .inner
                .known_skills
                .lock()
                .expect("known_skills mutex poisoned");
            known.clear();
            for s in summaries {
                known.insert(s.skill_id.clone(), s);
            }
        }
        Ok(val)
    }

    /// Replace the in-memory secrets snapshot (e.g. after `PUT /api/skills/{id}/config`).
    pub fn update_secrets(&self, secrets: SecretsConfig) {
        *self.inner.secrets.write().expect("secrets rwlock poisoned") = secrets;
    }

    /// Start (or restart) a `device` skill with **scoped secret injection**.
    pub async fn start_skill(
        &self,
        skill_id: &str,
        config: Map<String, Value>,
    ) -> Result<(), SkillBridgeError> {
        let declared = self.declared_secrets_for(skill_id).await?;
        let scoped_secrets = {
            let guard = self.inner.secrets.read().expect("secrets rwlock poisoned");
            resolve_scoped_secrets(&declared, &guard)
        };
        let params = SupervisorStartSkillParams {
            skill_id: skill_id.to_owned(),
            config,
            secrets: scoped_secrets,
        };
        let _ = self
            .call(
                "supervisor.start_skill",
                Some(serde_json::to_value(params)?),
            )
            .await?;
        Ok(())
    }

    /// Stop a running `device` skill.
    pub async fn stop_skill(&self, skill_id: &str) -> Result<(), SkillBridgeError> {
        let params = SupervisorSkillIdParams {
            skill_id: skill_id.to_owned(),
        };
        let _ = self
            .call("supervisor.stop_skill", Some(serde_json::to_value(params)?))
            .await?;
        Ok(())
    }

    /// Re-run `uv sync` / `uv venv` for a skill.
    pub async fn reinstall_skill(&self, skill_id: &str) -> Result<(), SkillBridgeError> {
        let params = SupervisorSkillIdParams {
            skill_id: skill_id.to_owned(),
        };
        let _ = self
            .call(
                "supervisor.reinstall_skill",
                Some(serde_json::to_value(params)?),
            )
            .await?;
        Ok(())
    }

    /// Fetch the last `tail` stderr log lines captured by the supervisor for `skill_id`.
    pub async fn get_logs(
        &self,
        skill_id: &str,
        tail: usize,
    ) -> Result<Vec<LogEntry>, SkillBridgeError> {
        let params = SupervisorGetLogsParams {
            skill_id: skill_id.to_owned(),
            tail,
        };
        let val = self
            .call("supervisor.get_logs", Some(serde_json::to_value(params)?))
            .await?;
        Ok(serde_json::from_value(val)?)
    }

    /// Send an `entity.command` to a running `device` skill.
    pub async fn send_entity_command(
        &self,
        skill_id: &str,
        command: EntityCommandParams,
    ) -> Result<Value, SkillBridgeError> {
        let params = SupervisorEntityCommandParams {
            skill_id: skill_id.to_owned(),
            command,
        };
        self.call(
            "supervisor.entity_command",
            Some(serde_json::to_value(params)?),
        )
        .await
    }

    /// Invoke an on-demand `action` skill with **scoped secret injection**.
    pub async fn invoke_skill(
        &self,
        skill_id: &str,
        config: Map<String, Value>,
        invoke: SkillInvokeParams,
    ) -> Result<Value, SkillBridgeError> {
        let declared = self.declared_secrets_for(skill_id).await?;
        let scoped_secrets = {
            let guard = self.inner.secrets.read().expect("secrets rwlock poisoned");
            resolve_scoped_secrets(&declared, &guard)
        };
        let params = SupervisorInvokeSkillParams {
            skill_id: skill_id.to_owned(),
            config,
            secrets: scoped_secrets,
            invoke,
        };
        self.call(
            "supervisor.invoke_skill",
            Some(serde_json::to_value(params)?),
        )
        .await
    }

    /// Await the background bridge task after `shutdown_rx` has been triggered.
    pub async fn shutdown(&self) {
        let handle = self.task.lock().expect("task mutex poisoned").take();
        if let Some(h) = handle
            && let Err(err) = h.await
        {
            warn!(error = %err, "skill bridge task join error");
        }
    }

    async fn declared_secrets_for(&self, skill_id: &str) -> Result<Vec<String>, SkillBridgeError> {
        if let Some(summary) = self
            .inner
            .known_skills
            .lock()
            .expect("known_skills mutex poisoned")
            .get(skill_id)
        {
            return Ok(summary.declared_secrets.clone());
        }
        // Cache miss — refresh from supervisor once.
        let summaries = self.list_skills().await?;
        Ok(summaries
            .into_iter()
            .find(|s| s.skill_id == skill_id)
            .map(|s| s.declared_secrets)
            .unwrap_or_default())
    }

    async fn call(&self, method: &str, params: Option<Value>) -> Result<Value, SkillBridgeError> {
        let tx = self
            .inner
            .out_tx
            .lock()
            .expect("out_tx mutex poisoned")
            .clone()
            .ok_or(SkillBridgeError::Disconnected)?;

        let id = RpcId::Number(self.inner.next_id.fetch_add(1, Ordering::Relaxed));
        let req = RpcRequest::new(id.clone(), method, params);
        let encoded = serde_json::to_string(&req)?;

        let (ack_tx, ack_rx) = oneshot::channel();
        self.inner
            .pending
            .lock()
            .expect("pending mutex poisoned")
            .insert(id.clone(), ack_tx);

        if tx.send(encoded).is_err() {
            self.inner
                .pending
                .lock()
                .expect("pending mutex poisoned")
                .remove(&id);
            return Err(SkillBridgeError::Disconnected);
        }

        match tokio::time::timeout(self.inner.rpc_timeout, ack_rx).await {
            Ok(Ok(res)) => res,
            Ok(Err(_)) => Err(SkillBridgeError::Disconnected),
            Err(_) => {
                self.inner
                    .pending
                    .lock()
                    .expect("pending mutex poisoned")
                    .remove(&id);
                Err(SkillBridgeError::Timeout(method.to_owned()))
            }
        }
    }
}

async fn run_bridge_loop(bridge: SkillBridge, mut shutdown_rx: watch::Receiver<bool>) {
    let mut backoff = INITIAL_RECONNECT_BACKOFF;

    loop {
        if *shutdown_rx.borrow() || shutdown_rx.has_changed().is_err() {
            break;
        }

        match UnixStream::connect(&bridge.inner.socket_path).await {
            Ok(stream) => {
                backoff = INITIAL_RECONNECT_BACKOFF;
                info!(
                    socket = %bridge.inner.socket_path.display(),
                    "connected to xiaohe supervisor over UDS"
                );
                run_bridge_session(&bridge, stream, &mut shutdown_rx).await;
                mark_disconnected(&bridge);
            }
            Err(err) => {
                debug!(
                    socket = %bridge.inner.socket_path.display(),
                    error = %err,
                    "supervisor UDS not reachable; retrying"
                );
            }
        }

        tokio::select! {
            biased;
            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() {
                    break;
                }
            }
            () = tokio::time::sleep(backoff) => {}
        }
        backoff = (backoff.saturating_mul(2)).min(MAX_RECONNECT_BACKOFF);
    }
}

async fn run_bridge_session(
    bridge: &SkillBridge,
    stream: UnixStream,
    shutdown_rx: &mut watch::Receiver<bool>,
) {
    let (read_half, mut write_half) = stream.into_split();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();

    {
        *bridge.inner.out_tx.lock().expect("out_tx mutex poisoned") = Some(out_tx.clone());
        bridge.inner.connected.store(true, Ordering::Release);
    }

    let writer_task = tokio::spawn(async move {
        while let Some(mut line) = out_rx.recv().await {
            line.push('\n');
            if write_half.write_all(line.as_bytes()).await.is_err() {
                break;
            }
            if write_half.flush().await.is_err() {
                break;
            }
        }
    });

    // Auto-start enabled device skills from devices.toml in a background task so the reader loop
    // processes responses concurrently.
    let autostart_bridge = bridge.clone();
    let autostart_task = tokio::spawn(async move {
        if let Err(err) = autostart_configured_skills(&autostart_bridge).await {
            warn!(error = %err, "failed to auto-start configured device skills");
        }
    });

    let mut lines = BufReader::new(read_half).lines();
    loop {
        tokio::select! {
            biased;
            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() {
                    break;
                }
            }
            line_res = lines.next_line() => {
                match line_res {
                    Ok(Some(line)) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        handle_supervisor_line(bridge, trimmed, &out_tx);
                    }
                    Ok(None) => {
                        info!("supervisor UDS connection closed");
                        break;
                    }
                    Err(err) => {
                        warn!(error = %err, "error reading from supervisor UDS");
                        break;
                    }
                }
            }
        }
    }

    autostart_task.abort();
    bridge.inner.connected.store(false, Ordering::Release);
    *bridge.inner.out_tx.lock().expect("out_tx mutex poisoned") = None;
    drop(out_tx);
    let _ = writer_task.await;
}

async fn autostart_configured_skills(bridge: &SkillBridge) -> Result<(), SkillBridgeError> {
    let summaries = bridge.list_skills().await?;
    for summary in summaries {
        if summary.mode != SkillMode::Device {
            continue;
        }
        let Some(user_cfg) = bridge.inner.skill_configs.get(&summary.skill_id) else {
            continue;
        };
        if !user_cfg.enabled {
            continue;
        }
        let config_json = toml_config_to_json(&user_cfg.config);
        info!(skill_id = %summary.skill_id, "auto-starting configured device skill");
        if let Err(err) = bridge.start_skill(&summary.skill_id, config_json).await {
            warn!(skill_id = %summary.skill_id, error = %err, "failed to start device skill");
        }
    }
    Ok(())
}

fn mark_disconnected(bridge: &SkillBridge) {
    bridge.inner.connected.store(false, Ordering::Release);
    *bridge.inner.out_tx.lock().expect("out_tx mutex poisoned") = None;

    let mut pending = bridge.inner.pending.lock().expect("pending mutex poisoned");
    for (_, tx) in pending.drain() {
        let _ = tx.send(Err(SkillBridgeError::Disconnected));
    }
    drop(pending);

    // Mark all entities belonging to known skills as Offline.
    let skill_ids: Vec<String> = bridge
        .inner
        .known_skills
        .lock()
        .expect("known_skills mutex poisoned")
        .keys()
        .cloned()
        .collect();
    for skill_id in skill_ids {
        bridge
            .inner
            .state
            .set_source_availability(&EntitySource::Skill { skill_id }, Availability::Offline);
    }
}

fn handle_supervisor_line(
    bridge: &SkillBridge,
    line: &str,
    out_tx: &mpsc::UnboundedSender<String>,
) {
    let val: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(err) => {
            warn!(error = %err, "ignoring non-JSON line from supervisor");
            return;
        }
    };

    // Case 1: JSON-RPC Response to a Core -> Supervisor request.
    if val.get("method").is_none() {
        let Ok(resp) = serde_json::from_value::<RpcResponse>(val) else {
            return;
        };
        if let Some(id) = resp.id
            && let Some(ack) = bridge
                .inner
                .pending
                .lock()
                .expect("pending mutex poisoned")
                .remove(&id)
        {
            let res = match (resp.result, resp.error) {
                (_, Some(err)) => Err(SkillBridgeError::Rpc {
                    code: err.code,
                    message: err.message,
                }),
                (Some(r), None) => Ok(r),
                (None, None) => Ok(Value::Null),
            };
            let _ = ack.send(res);
        }
        return;
    }

    // Case 2: Bidirectional JSON-RPC Request from Supervisor (core.get_entity / core.call_entity).
    if val.get("id").is_some() {
        let Ok(req) = serde_json::from_value::<RpcRequest>(val) else {
            return;
        };
        let b = bridge.clone();
        let tx = out_tx.clone();
        tokio::spawn(async move {
            let resp = handle_core_callback(&b, req).await;
            if let Ok(encoded) = serde_json::to_string(&resp) {
                let _ = tx.send(encoded);
            }
        });
        return;
    }

    // Case 3: Notification from Supervisor (skill.status_changed / skill.event).
    let Ok(notif) = serde_json::from_value::<RpcNotification>(val) else {
        return;
    };
    handle_supervisor_notification(bridge, notif);
}

async fn handle_core_callback(bridge: &SkillBridge, req: RpcRequest) -> RpcResponse {
    let id = req.id;
    let params = req.params.unwrap_or(Value::Null);

    match req.method.as_str() {
        "core.get_entity" => {
            let parsed: CoreGetEntityParams = match serde_json::from_value(params) {
                Ok(p) => p,
                Err(e) => return RpcResponse::err(Some(id), ERR_INVALID_PARAMS, e.to_string()),
            };
            match bridge.inner.state.get(&parsed.entity_id) {
                Some(entity) => match serde_json::to_value(entity) {
                    Ok(v) => RpcResponse::ok(id, v),
                    Err(e) => RpcResponse::err(Some(id), ERR_INTERNAL_ERROR, e.to_string()),
                },
                None => RpcResponse::err(
                    Some(id),
                    -32004,
                    format!("entity '{}' not found", parsed.entity_id),
                ),
            }
        }
        "core.call_entity" => {
            let parsed: CoreCallEntityParams = match serde_json::from_value(params) {
                Ok(p) => p,
                Err(e) => return RpcResponse::err(Some(id), ERR_INVALID_PARAMS, e.to_string()),
            };
            let Some(entity) = bridge.inner.state.get(&parsed.entity_id) else {
                return RpcResponse::err(
                    Some(id),
                    -32004,
                    format!("entity '{}' not found", parsed.entity_id),
                );
            };
            match entity.source {
                EntitySource::Skill { skill_id } => {
                    let cmd = EntityCommandParams {
                        entity_id: parsed.entity_id,
                        action: parsed.action,
                        params: parsed.params,
                    };
                    match bridge.send_entity_command(&skill_id, cmd).await {
                        Ok(res) => RpcResponse::ok(id, res),
                        Err(err) => RpcResponse::err(Some(id), ERR_INTERNAL_ERROR, err.to_string()),
                    }
                }
                other => RpcResponse::err(
                    Some(id),
                    ERR_INTERNAL_ERROR,
                    format!("core.call_entity routing for source {other:?} is not yet wired"),
                ),
            }
        }
        other => RpcResponse::err(
            Some(id),
            ERR_METHOD_NOT_FOUND,
            format!("unknown core callback '{other}'"),
        ),
    }
}

fn handle_supervisor_notification(bridge: &SkillBridge, notif: RpcNotification) {
    let params = notif.params.unwrap_or(Value::Null);
    match notif.method.as_str() {
        "skill.status_changed" => {
            let Ok(ev) = serde_json::from_value::<SkillStatusChangedParams>(params) else {
                return;
            };
            if let Some(summary) = bridge
                .inner
                .known_skills
                .lock()
                .expect("known_skills mutex poisoned")
                .get_mut(&ev.skill_id)
            {
                summary.status = ev.status;
                summary.restart_count = ev.restart_count;
                summary.last_error = ev.last_error.clone();
            }

            if matches!(ev.status, SkillStatus::Stopped | SkillStatus::Error) {
                bridge.inner.state.set_source_availability(
                    &EntitySource::Skill {
                        skill_id: ev.skill_id.clone(),
                    },
                    Availability::Offline,
                );
            }

            bridge.inner.events.publish(CoreEvent::SkillStatusChanged {
                skill_id: ev.skill_id,
                status: ev.status,
                message: ev.last_error,
                at: now_unix_ms(),
            });
        }
        "skill.event" => {
            let Ok(ev) = serde_json::from_value::<SkillForwardedEventParams>(params) else {
                return;
            };
            let ev_params = ev.params.unwrap_or(Value::Null);
            match ev.method.as_str() {
                "entity.register" => {
                    let Ok(reg) = serde_json::from_value::<EntityRegisterParams>(ev_params) else {
                        warn!(skill_id = %ev.skill_id, "invalid entity.register params from skill");
                        return;
                    };
                    for ent in reg.entities {
                        let placeholder = placeholder_domain_state(
                            &ent.domain,
                            ent.unit.clone(),
                            ent.device_class.clone(),
                        );
                        let record = EntityRecord {
                            entity_id: ent.entity_id,
                            device_id: Some(reg.device.device_id.clone()),
                            area_id: reg.device.area_id.clone(),
                            name: ent.name,
                            source: EntitySource::Skill {
                                skill_id: ev.skill_id.clone(),
                            },
                            availability: Availability::Unknown,
                            state: placeholder,
                            capabilities: ent.capabilities,
                            extra: Map::new(),
                            custom_actions: vec![],
                            last_changed_at: 0,
                            last_updated_at: 0,
                        };
                        // register_entity avoids emitting placeholder state to SQLite history.
                        bridge.inner.state.register_entity(record);
                    }
                }
                "entity.state_changed" => {
                    let Ok(sc) = serde_json::from_value::<EntityStateChangedParams>(ev_params)
                    else {
                        warn!(
                            skill_id = %ev.skill_id,
                            "invalid entity.state_changed params from skill"
                        );
                        return;
                    };
                    let extra_opt = (!sc.extra.is_empty()).then_some(sc.extra.clone());
                    let updated = bridge.inner.state.update_state(
                        &sc.entity_id,
                        sc.state.clone(),
                        extra_opt,
                        Some(sc.availability),
                    );
                    if updated.is_none() {
                        // Skill emitted state before entity.register — upsert cleanly.
                        let record = EntityRecord {
                            entity_id: sc.entity_id.clone(),
                            device_id: Some(format!("skill:{}", ev.skill_id)),
                            area_id: None,
                            name: sc.entity_id,
                            source: EntitySource::Skill {
                                skill_id: ev.skill_id,
                            },
                            availability: sc.availability,
                            state: sc.state,
                            capabilities: vec![],
                            extra: sc.extra,
                            custom_actions: vec![],
                            last_changed_at: 0,
                            last_updated_at: 0,
                        };
                        bridge.inner.state.upsert_entity(record);
                    }
                }
                other => {
                    debug!(skill_id = %ev.skill_id, method = %other, "unhandled skill notification");
                }
            }
        }
        other => {
            debug!(method = %other, "unhandled supervisor notification");
        }
    }
}

fn placeholder_domain_state(
    domain: &str,
    unit: Option<String>,
    device_class: Option<String>,
) -> DomainState {
    match domain {
        "light" => DomainState::Light {
            on: false,
            brightness: None,
            color_temp_k: None,
            rgb: None,
        },
        "switch" => DomainState::Switch { on: false },
        "climate" => DomainState::Climate {
            mode: ClimateMode::Off,
            target_temp_c: None,
            current_temp_c: None,
            fan_mode: None,
        },
        "cover" => DomainState::Cover {
            state: CoverState::Closed,
            position: None,
        },
        "lock" => DomainState::Lock {
            state: LockState::Locked,
        },
        "media_player" => DomainState::MediaPlayer {
            state: MediaState::Off,
            volume: None,
            muted: None,
            title: None,
        },
        "sensor" => DomainState::Sensor {
            value: SensorValue::Text(String::new()),
            unit,
            device_class,
        },
        "binary_sensor" => DomainState::BinarySensor {
            active: false,
            device_class,
        },
        "button" => DomainState::Button {
            last_pressed_at: None,
        },
        _ => DomainState::Custom {
            state: "unknown".to_owned(),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use tempfile::TempDir;
    use tokio::sync::Mutex as AsyncMutex;

    use super::*;
    use crate::state::spawn_history_writer_with_config;
    use crate::storage::{HistoryStore, SqliteStore, TimeRange};
    use crate::supervisor::{SupervisorManager, SupervisorTimingConfig, run_uds_server};

    fn fast_timing() -> SupervisorTimingConfig {
        SupervisorTimingConfig {
            init_timeout: Duration::from_secs(5),
            shutdown_grace: Duration::from_secs(2),
            initial_backoff: Duration::from_millis(50),
            max_backoff: Duration::from_millis(200),
            ping_timeout: Duration::from_millis(150),
            heartbeat_interval_override: Some(Duration::from_millis(120)),
        }
    }

    #[tokio::test]
    async fn uds_bridge_end_to_end_scoped_secrets_state_history_and_callbacks() {
        let dir = TempDir::new().unwrap();
        let skills_dir = dir.path().join("skills");
        let sock_path = dir.path().join("rpc.sock");

        // Skill A: device skill that declares ONLY `skill_a_token`
        let dev_dir = skills_dir.join("demo-purifier");
        fs::create_dir_all(&dev_dir).unwrap();
        fs::write(
            dev_dir.join("skill.toml"),
            r#"
[skill]
id = "demo-purifier"
name = "Demo Purifier"
version = "0.1.0"
mode = "device"

[runtime]
type = "executable"
entrypoint = ["python3", "main.py"]
heartbeat_interval_secs = 30
invoke_timeout_secs = 5
restart_policy = "never"

[secrets.skill_a_token]
required = true
"#,
        )
        .unwrap();

        fs::write(
            dev_dir.join("main.py"),
            r#"
import json, sys

def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()

received_secrets = {}

for raw in sys.stdin:
    msg = json.loads(raw)
    m = msg.get("method")
    rid = msg.get("id")
    if m == "skill.init":
        received_secrets = msg["params"].get("secrets", {})
        # Must receive skill_a_token and MUST NOT receive skill_b_secret
        ok = (
            received_secrets.get("skill_a_token") == "secret-alpha"
            and "skill_b_secret" not in received_secrets
        )
        send({"jsonrpc": "2.0", "id": rid, "result": {"ok": ok}})
        send({
            "jsonrpc": "2.0",
            "method": "entity.register",
            "params": {
                "device": {"device_id": "skill:demo-purifier", "name": "Demo Purifier", "area_id": "living_room"},
                "entities": [
                    {"entity_id": "switch.demo_purifier", "name": "Purifier", "domain": "switch"},
                    {"entity_id": "sensor.demo_pm25", "name": "PM2.5", "domain": "sensor", "unit": "µg/m³", "device_class": "pm25"}
                ]
            }
        })
        send({
            "jsonrpc": "2.0",
            "method": "entity.state_changed",
            "params": {
                "entity_id": "sensor.demo_pm25",
                "availability": "online",
                "state": {"domain": "sensor", "value": 42.0, "unit": "µg/m³", "device_class": "pm25"}
            }
        })
    elif m == "skill.ping":
        send({"jsonrpc": "2.0", "id": rid, "result": {"ok": True}})
    elif m == "entity.command":
        p = msg["params"]
        send({
            "jsonrpc": "2.0",
            "method": "entity.state_changed",
            "params": {
                "entity_id": p["entity_id"],
                "availability": "online",
                "state": {"domain": "switch", "on": True},
                "extra": {"speed_pct": p["params"].get("speed_pct", 50)}
            }
        })
        send({"jsonrpc": "2.0", "id": rid, "result": {"ok": True, "secret_keys": sorted(received_secrets.keys())}})
    elif m == "skill.shutdown":
        break
"#,
        )
        .unwrap();

        // Skill B: action skill that declares ONLY `skill_b_secret`, reads sensor via core.get_entity,
        // and controls the purifier via core.call_entity!
        let act_dir = skills_dir.join("air-advisor");
        fs::create_dir_all(&act_dir).unwrap();
        fs::write(
            act_dir.join("skill.toml"),
            r#"
[skill]
id = "air-advisor"
name = "Air Advisor"
version = "0.1.0"
mode = "action"

[runtime]
type = "executable"
entrypoint = ["python3", "main.py"]
invoke_timeout_secs = 10

[secrets.skill_b_secret]
required = true

[actions.boost_if_dirty]
description = "Boost purifier if PM2.5 > 35"
"#,
        )
        .unwrap();

        fs::write(
            act_dir.join("main.py"),
            r#"
import json, sys

def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()

received_secrets = {}

for raw in sys.stdin:
    msg = json.loads(raw)
    m = msg.get("method")
    rid = msg.get("id")
    if m == "skill.init":
        received_secrets = msg["params"].get("secrets", {})
        ok = (
            received_secrets.get("skill_b_secret") == "secret-beta"
            and "skill_a_token" not in received_secrets
        )
        send({"jsonrpc": "2.0", "id": rid, "result": {"ok": ok}})
    elif m == "skill.invoke":
        # 1. Read PM2.5 via core.get_entity
        send({"jsonrpc": "2.0", "id": 101, "method": "core.get_entity", "params": {"entity_id": "sensor.demo_pm25"}})
        ent = json.loads(sys.stdin.readline())["result"]
        pm25 = ent["state"]["value"]
        # 2. Trigger purifier via core.call_entity
        send({
            "jsonrpc": "2.0",
            "id": 102,
            "method": "core.call_entity",
            "params": {"entity_id": "switch.demo_purifier", "action": "turn_on", "params": {"speed_pct": 90}}
        })
        call_res = json.loads(sys.stdin.readline())["result"]
        send({
            "jsonrpc": "2.0",
            "id": rid,
            "result": {
                "pm25": pm25,
                "boosted": call_res["ok"],
                "secret_keys": sorted(received_secrets.keys())
            }
        })
    elif m == "skill.shutdown":
        break
"#,
        )
        .unwrap();

        // Start Supervisor UDS server
        let (sup_events_tx, sup_events_rx) = mpsc::unbounded_channel();
        let mut sup_mgr = SupervisorManager::with_timing(&skills_dir, sup_events_tx, fast_timing());
        assert!(sup_mgr.scan().is_empty());
        let sup_mgr = Arc::new(AsyncMutex::new(sup_mgr));
        let (sup_shutdown_tx, sup_shutdown_rx) = watch::channel(false);
        let sock_clone = sock_path.clone();
        let sup_handle = tokio::spawn(async move {
            run_uds_server(&sock_clone, sup_mgr, sup_events_rx, sup_shutdown_rx)
                .await
                .expect("uds server");
        });

        // Wait for socket to bind
        for _ in 0..40 {
            if sock_path.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(sock_path.exists(), "supervisor socket should exist");

        // Start Core StateStore, HistoryWriter, and SkillBridge
        let history = Arc::new(SqliteStore::open(":memory:").await.expect("sqlite"));
        let bus = EventBus::new();
        let state = StateStore::new(bus.clone());
        let (core_shutdown_tx, core_shutdown_rx) = watch::channel(false);
        let hist_handle = spawn_history_writer_with_config(
            Arc::clone(&history) as Arc<dyn HistoryStore>,
            &bus,
            core_shutdown_rx.clone(),
            16,
            Duration::from_millis(50),
        );

        let mut secrets = SecretsConfig::default();
        secrets.set("skill_a_token", "secret-alpha");
        secrets.set("skill_b_secret", "secret-beta");
        secrets.set("unrelated_core_secret", "never-sent");

        let mut skill_configs = BTreeMap::new();
        skill_configs.insert("demo-purifier".to_owned(), SkillUserConfig::default());

        let bridge = SkillBridge::start(
            &sock_path,
            skill_configs,
            secrets,
            state.clone(),
            bus.clone(),
            core_shutdown_rx,
        );

        // Wait until bridge connects and auto-started demo-purifier emits PM2.5 = 42.0
        let mut ready = false;
        for _ in 0..60 {
            if bridge.is_connected()
                && let Some(ent) = state.get("sensor.demo_pm25")
                && ent.availability == Availability::Online
            {
                ready = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(
            ready,
            "expected bridge to connect, auto-start demo-purifier, and receive sensor.demo_pm25"
        );

        // Invoke action skill over UDS -> triggers core.get_entity + core.call_entity -> purifier on
        let invoke_res = bridge
            .invoke_skill(
                "air-advisor",
                Map::new(),
                SkillInvokeParams {
                    action: "boost_if_dirty".into(),
                    params: Map::new(),
                    context: Map::new(),
                },
            )
            .await
            .expect("invoke action skill");
        assert_eq!(invoke_res["pm25"], 42.0);
        assert_eq!(invoke_res["boosted"], true);
        assert_eq!(invoke_res["secret_keys"], json!(["skill_b_secret"]));

        // Verify switch.demo_purifier state was updated via core.call_entity
        tokio::time::sleep(Duration::from_millis(100)).await;
        let purifier = state.get("switch.demo_purifier").expect("purifier exists");
        assert_eq!(purifier.state, DomainState::Switch { on: true });
        assert_eq!(purifier.extra.get("speed_pct"), Some(&json!(90)));

        // Verify sensor.demo_pm25 history was persisted to SQLite (and NO bogus placeholder rows)
        let now_ms = now_unix_ms();
        let series = history
            .query_entity_history(
                "sensor.demo_pm25",
                TimeRange::new(0, now_ms + 5_000).unwrap(),
                None,
            )
            .await
            .expect("query pm25 history");
        match series {
            crate::storage::EntityHistorySeries::Numeric { points, .. } => {
                assert_eq!(points.len(), 1);
                assert!((points[0].avg - 42.0).abs() < 1e-6);
            }
            other => panic!("expected numeric series, got {other:?}"),
        }

        // Shut down supervisor -> bridge detects disconnect and marks skill entities Offline
        let _ = sup_shutdown_tx.send(true);
        sup_handle.await.expect("supervisor shutdown");
        assert!(
            !sock_path.exists(),
            "supervisor should clean up socket file"
        );

        let mut marked_offline = false;
        for _ in 0..40 {
            if !bridge.is_connected()
                && state
                    .get("sensor.demo_pm25")
                    .is_some_and(|e| e.availability == Availability::Offline)
            {
                marked_offline = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(
            marked_offline,
            "skill entities should become Offline when supervisor disconnects"
        );

        let _ = core_shutdown_tx.send(true);
        bridge.shutdown().await;
        hist_handle.await.expect("history writer shutdown");
    }

    #[tokio::test]
    async fn reference_skills_miio_demo_and_climate_optimizer_end_to_end() {
        let repo_skills = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("skills");
        let tmp = TempDir::new().unwrap();
        let sock_path = tmp.path().join("ref-rpc.sock");

        let (sup_events_tx, sup_events_rx) = mpsc::unbounded_channel();
        let mut sup_mgr =
            SupervisorManager::with_timing(&repo_skills, sup_events_tx, fast_timing());
        let scan_errs = sup_mgr.scan();
        assert!(
            scan_errs.is_empty(),
            "reference skills manifests must validate cleanly: {scan_errs:?}"
        );

        let sup_mgr = Arc::new(AsyncMutex::new(sup_mgr));
        let (sup_shutdown_tx, sup_shutdown_rx) = watch::channel(false);
        let sock_clone = sock_path.clone();
        let sup_handle = tokio::spawn(async move {
            run_uds_server(&sock_clone, sup_mgr, sup_events_rx, sup_shutdown_rx)
                .await
                .expect("uds server");
        });

        for _ in 0..40 {
            if sock_path.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(sock_path.exists());

        let bus = EventBus::new();
        let state = StateStore::new(bus.clone());
        let (core_shutdown_tx, core_shutdown_rx) = watch::channel(false);

        let mut purifier_cfg = BTreeMap::new();
        purifier_cfg.insert("demo_mode".into(), toml::Value::Boolean(true));
        purifier_cfg.insert("poll_interval_secs".into(), toml::Value::Integer(1));
        let mut skill_configs = BTreeMap::new();
        skill_configs.insert(
            "miio-airpurifier".to_owned(),
            SkillUserConfig {
                enabled: true,
                config: purifier_cfg,
            },
        );

        let bridge = SkillBridge::start(
            &sock_path,
            skill_configs,
            SecretsConfig::default(),
            state.clone(),
            bus,
            core_shutdown_rx,
        );

        // Wait for uv sync + miio-airpurifier startup + first poll
        let mut ready = false;
        for _ in 0..120 {
            if bridge.is_connected()
                && state
                    .get("sensor.purifier_temp")
                    .is_some_and(|e| e.availability == Availability::Online)
                && state
                    .get("sensor.purifier_humidity")
                    .is_some_and(|e| e.availability == Availability::Online)
                && state
                    .get("switch.purifier_power")
                    .is_some_and(|e| e.state == DomainState::Switch { on: false })
            {
                ready = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(
            ready,
            "expected miio-airpurifier demo_mode to publish temp, humidity, and power switch"
        );

        // Invoke climate-optimizer action skill with threshold_c = 24.0 so it triggers switch.purifier_power
        let mut invoke_params = Map::new();
        invoke_params.insert("temp_entity".into(), json!("sensor.purifier_temp"));
        invoke_params.insert("humidity_entity".into(), json!("sensor.purifier_humidity"));
        invoke_params.insert("target_switch".into(), json!("switch.purifier_power"));
        invoke_params.insert("threshold_c".into(), json!(24.0));

        let res = bridge
            .invoke_skill(
                "climate-optimizer",
                Map::new(),
                SkillInvokeParams {
                    action: "evaluate_comfort".into(),
                    params: invoke_params,
                    context: Map::new(),
                },
            )
            .await
            .expect("invoke climate-optimizer");

        assert_eq!(res["temperature_c"], 24.5);
        assert_eq!(res["humidity_pct"], 52.0);
        assert_eq!(res["triggered_switch"], "switch.purifier_power");
        assert!(res["heat_index_c"].as_f64().unwrap() >= 24.0);

        // Confirm switch.purifier_power is now ON in StateStore
        let mut turned_on = false;
        for _ in 0..30 {
            if state
                .get("switch.purifier_power")
                .is_some_and(|e| e.state == DomainState::Switch { on: true })
            {
                turned_on = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(
            turned_on,
            "climate-optimizer core.call_entity should have turned on switch.purifier_power"
        );

        let _ = core_shutdown_tx.send(true);
        bridge.shutdown().await;
        let _ = sup_shutdown_tx.send(true);
        sup_handle.await.expect("supervisor shutdown");
    }
}
