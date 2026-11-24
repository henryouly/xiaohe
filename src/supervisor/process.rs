use std::collections::{BTreeMap, HashMap, VecDeque};
use std::env;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::protocol::rpc::{
    EntityCommandParams, JSONRPC_VERSION, PROTOCOL_VERSION, RpcId, RpcNotification, RpcRequest,
    RpcResponse, SkillInitParams, SkillInvokeParams,
};
use crate::protocol::skill::{RestartPolicy, SkillMode, SkillStatus};
use crate::supervisor::env::{EnvError, ResolvedCommand, provision_skill_env};
use crate::supervisor::manifest::{DiscoveredSkill, ManifestError, scan_skills_dir};

pub const DEFAULT_LOG_RING_CAPACITY: usize = 500;
pub const MAX_MISSED_HEARTBEATS: u32 = 3;

#[derive(Debug, Error)]
pub enum SupervisorError {
    #[error("unknown skill '{0}'")]
    UnknownSkill(String),
    #[error("skill '{0}' is not running")]
    NotRunning(String),
    #[error("skill '{0}' mode mismatch: {1}")]
    ModeMismatch(String, String),
    #[error("environment provisioning error: {0}")]
    Env(#[from] EnvError),
    #[error("process I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON-RPC error: {0}")]
    Rpc(String),
    #[error("timeout waiting for skill '{0}' ({1})")]
    Timeout(String, String),
}

/// A single timestamped log line captured from a skill's `stderr`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    pub ts: i64,
    pub stream: String,
    pub line: String,
}

/// Bounded ring buffer capturing up to `capacity` `stderr` lines per skill.
#[derive(Debug, Clone)]
pub struct LogRingBuffer {
    inner: Arc<Mutex<VecDeque<LogEntry>>>,
    capacity: usize,
}

impl LogRingBuffer {
    pub fn new(capacity: usize) -> Self {
        let cap = capacity.max(1);
        Self {
            inner: Arc::new(Mutex::new(VecDeque::with_capacity(cap))),
            capacity: cap,
        }
    }

    pub fn push(&self, line: String) {
        let mut guard = self.inner.lock().expect("log ring mutex poisoned");
        if guard.len() >= self.capacity {
            guard.pop_front();
        }
        guard.push_back(LogEntry {
            ts: now_unix_ms(),
            stream: "stderr".to_owned(),
            line,
        });
    }

    pub fn tail(&self, limit: usize) -> Vec<LogEntry> {
        let guard = self.inner.lock().expect("log ring mutex poisoned");
        let n = limit.min(guard.len());
        let start = guard.len() - n;
        guard.iter().skip(start).cloned().collect()
    }
}

/// Runtime status snapshot for a discovered skill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillRuntimeSummary {
    pub skill_id: String,
    pub name: String,
    pub version: String,
    pub mode: SkillMode,
    pub status: SkillStatus,
    pub restart_count: u32,
    pub last_error: Option<String>,
    pub declared_secrets: Vec<String>,
}

/// Events emitted by the supervisor toward `xiaohe-core` (over UDS in Phase 3.2).
#[derive(Debug)]
pub enum SupervisorEvent {
    StatusChanged {
        skill_id: String,
        status: SkillStatus,
        restart_count: u32,
        last_error: Option<String>,
    },
    SkillNotification {
        skill_id: String,
        method: String,
        params: Option<Value>,
    },
    CoreCallback {
        skill_id: String,
        method: String,
        params: Option<Value>,
        reply_tx: oneshot::Sender<Result<Value, String>>,
    },
}

/// Tunable timings for supervisor actors (shortened in tests).
#[derive(Debug, Clone, Copy)]
pub struct SupervisorTimingConfig {
    pub init_timeout: Duration,
    pub shutdown_grace: Duration,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    pub ping_timeout: Duration,
    /// Optional override for `skill.toml`'s `heartbeat_interval_secs` (used in tests).
    pub heartbeat_interval_override: Option<Duration>,
}

impl Default for SupervisorTimingConfig {
    fn default() -> Self {
        Self {
            init_timeout: Duration::from_secs(15),
            shutdown_grace: Duration::from_secs(5),
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(60),
            ping_timeout: Duration::from_secs(5),
            heartbeat_interval_override: None,
        }
    }
}

#[derive(Debug, Clone)]
struct SharedSkillState {
    status: SkillStatus,
    restart_count: u32,
    last_error: Option<String>,
}

struct RunningDeviceHandle {
    cmd_tx: mpsc::Sender<DeviceActorCommand>,
    stop_tx: watch::Sender<bool>,
    join_handle: JoinHandle<()>,
}

enum DeviceActorCommand {
    EntityCommand {
        params: EntityCommandParams,
        ack: oneshot::Sender<Result<Value, SupervisorError>>,
    },
}

struct ManagedSkill {
    discovered: DiscoveredSkill,
    state: Arc<Mutex<SharedSkillState>>,
    logs: LogRingBuffer,
    running: Option<RunningDeviceHandle>,
}

/// Manages skill discovery, `uv` environment isolation, and child process lifecycles.
pub struct SupervisorManager {
    skills_dir: PathBuf,
    skills: BTreeMap<String, ManagedSkill>,
    events_tx: mpsc::UnboundedSender<SupervisorEvent>,
    timing: SupervisorTimingConfig,
}

impl SupervisorManager {
    pub fn new(
        skills_dir: impl Into<PathBuf>,
        events_tx: mpsc::UnboundedSender<SupervisorEvent>,
    ) -> Self {
        Self::with_timing(skills_dir, events_tx, SupervisorTimingConfig::default())
    }

    pub fn with_timing(
        skills_dir: impl Into<PathBuf>,
        events_tx: mpsc::UnboundedSender<SupervisorEvent>,
        timing: SupervisorTimingConfig,
    ) -> Self {
        Self {
            skills_dir: skills_dir.into(),
            skills: BTreeMap::new(),
            events_tx,
            timing,
        }
    }

    /// Scan `skills_dir` for `*/skill.toml`, registering newly discovered skills and updating
    /// manifests for existing ones (preserving logs and running handles).
    pub fn scan(&mut self) -> Vec<ManifestError> {
        let (found, errors) = scan_skills_dir(&self.skills_dir);
        for err in &errors {
            warn!(error = %err, "failed to load skill manifest");
        }
        for skill in found {
            let id = skill.manifest.skill.id.clone();
            if let Some(existing) = self.skills.get_mut(&id) {
                existing.discovered = skill;
            } else {
                self.skills.insert(
                    id,
                    ManagedSkill {
                        discovered: skill,
                        state: Arc::new(Mutex::new(SharedSkillState {
                            status: SkillStatus::Stopped,
                            restart_count: 0,
                            last_error: None,
                        })),
                        logs: LogRingBuffer::new(DEFAULT_LOG_RING_CAPACITY),
                        running: None,
                    },
                );
            }
        }
        errors
    }

    pub fn list_summaries(&self) -> Vec<SkillRuntimeSummary> {
        self.skills
            .values()
            .map(|m| {
                let st = m.state.lock().expect("skill state mutex poisoned");
                SkillRuntimeSummary {
                    skill_id: m.discovered.manifest.skill.id.clone(),
                    name: m.discovered.manifest.skill.name.clone(),
                    version: m.discovered.manifest.skill.version.clone(),
                    mode: m.discovered.manifest.skill.mode,
                    status: st.status,
                    restart_count: st.restart_count,
                    last_error: st.last_error.clone(),
                    declared_secrets: m.discovered.manifest.declared_secret_keys(),
                }
            })
            .collect()
    }

    pub fn get_discovered(&self, skill_id: &str) -> Option<&DiscoveredSkill> {
        self.skills.get(skill_id).map(|m| &m.discovered)
    }

    pub fn get_logs(&self, skill_id: &str, tail: usize) -> Result<Vec<LogEntry>, SupervisorError> {
        let managed = self
            .skills
            .get(skill_id)
            .ok_or_else(|| SupervisorError::UnknownSkill(skill_id.to_owned()))?;
        Ok(managed.logs.tail(tail.min(DEFAULT_LOG_RING_CAPACITY)))
    }

    /// Prepare a lock-free reinstall task so callers holding a mutex on `SupervisorManager` can
    /// drop the lock before running `uv sync` / `uv venv`.
    pub fn prepare_reinstall(&self, skill_id: &str) -> Result<PreparedReinstall, SupervisorError> {
        let managed = self
            .skills
            .get(skill_id)
            .ok_or_else(|| SupervisorError::UnknownSkill(skill_id.to_owned()))?;
        Ok(PreparedReinstall {
            skill_id: skill_id.to_owned(),
            discovered: managed.discovered.clone(),
            state: Arc::clone(&managed.state),
            events_tx: self.events_tx.clone(),
        })
    }

    /// Re-run `uv sync` / `uv venv` for a skill.
    pub async fn reinstall_skill(&mut self, skill_id: &str) -> Result<(), SupervisorError> {
        self.prepare_reinstall(skill_id)?.execute().await
    }

    /// Start (or restart) a long-running `mode = "device"` skill.
    pub async fn start_device_skill(
        &mut self,
        skill_id: &str,
        config: Map<String, Value>,
        secrets: BTreeMap<String, String>,
    ) -> Result<(), SupervisorError> {
        // Stop any existing instance first so restart is idempotent.
        if self
            .skills
            .get(skill_id)
            .is_some_and(|m| m.running.is_some())
        {
            self.stop_skill(skill_id).await?;
        }

        let managed = self
            .skills
            .get_mut(skill_id)
            .ok_or_else(|| SupervisorError::UnknownSkill(skill_id.to_owned()))?;

        if managed.discovered.manifest.skill.mode != SkillMode::Device {
            return Err(SupervisorError::ModeMismatch(
                skill_id.to_owned(),
                "start_device_skill requires mode = \"device\"".to_owned(),
            ));
        }

        let discovered = managed.discovered.clone();
        let state = Arc::clone(&managed.state);
        let logs = managed.logs.clone();
        let events_tx = self.events_tx.clone();
        let timing = self.timing;

        let (cmd_tx, cmd_rx) = mpsc::channel(32);
        let (stop_tx, stop_rx) = watch::channel(false);

        let join_handle = tokio::spawn(async move {
            run_device_skill_actor(
                discovered, config, secrets, state, logs, events_tx, cmd_rx, stop_rx, timing,
            )
            .await;
        });

        managed.running = Some(RunningDeviceHandle {
            cmd_tx,
            stop_tx,
            join_handle,
        });
        Ok(())
    }

    /// Stop a running `device` skill gracefully (`skill.shutdown` + grace period + kill).
    pub async fn stop_skill(&mut self, skill_id: &str) -> Result<(), SupervisorError> {
        let managed = self
            .skills
            .get_mut(skill_id)
            .ok_or_else(|| SupervisorError::UnknownSkill(skill_id.to_owned()))?;

        if let Some(handle) = managed.running.take() {
            let _ = handle.stop_tx.send(true);
            if let Err(err) = handle.join_handle.await {
                warn!(skill_id = %skill_id, error = %err, "device skill actor join error");
            }
        }
        set_skill_status(
            &managed.state,
            &self.events_tx,
            skill_id,
            SkillStatus::Stopped,
            None,
        );
        Ok(())
    }

    /// Prepare a lock-free `entity.command` handle for a running `device` skill.
    pub fn prepare_entity_command(
        &self,
        skill_id: &str,
    ) -> Result<PreparedEntityCommand, SupervisorError> {
        let managed = self
            .skills
            .get(skill_id)
            .ok_or_else(|| SupervisorError::UnknownSkill(skill_id.to_owned()))?;
        let invoke_timeout =
            Duration::from_secs(managed.discovered.manifest.runtime.invoke_timeout_secs);
        let handle = managed
            .running
            .as_ref()
            .ok_or_else(|| SupervisorError::NotRunning(skill_id.to_owned()))?;
        Ok(PreparedEntityCommand {
            skill_id: skill_id.to_owned(),
            cmd_tx: handle.cmd_tx.clone(),
            invoke_timeout,
        })
    }

    /// Send an `entity.command` JSON-RPC request to a running `device` skill.
    pub async fn send_entity_command(
        &self,
        skill_id: &str,
        params: EntityCommandParams,
    ) -> Result<Value, SupervisorError> {
        self.prepare_entity_command(skill_id)?.execute(params).await
    }

    /// Prepare a lock-free on-demand `mode = "action"` skill invocation.
    pub fn prepare_action_invoke(
        &self,
        skill_id: &str,
    ) -> Result<PreparedActionInvoke, SupervisorError> {
        let managed = self
            .skills
            .get(skill_id)
            .ok_or_else(|| SupervisorError::UnknownSkill(skill_id.to_owned()))?;
        if managed.discovered.manifest.skill.mode != SkillMode::Action {
            return Err(SupervisorError::ModeMismatch(
                skill_id.to_owned(),
                "invoke_action_skill requires mode = \"action\"".to_owned(),
            ));
        }
        Ok(PreparedActionInvoke {
            skill_id: skill_id.to_owned(),
            discovered: managed.discovered.clone(),
            state: Arc::clone(&managed.state),
            logs: managed.logs.clone(),
            events_tx: self.events_tx.clone(),
            timing: self.timing,
        })
    }

    /// Execute an on-demand `mode = "action"` skill (`skill.init` -> `skill.invoke` -> `skill.shutdown`).
    pub async fn invoke_action_skill(
        &mut self,
        skill_id: &str,
        config: Map<String, Value>,
        secrets: BTreeMap<String, String>,
        params: SkillInvokeParams,
    ) -> Result<Value, SupervisorError> {
        self.prepare_action_invoke(skill_id)?
            .execute(config, secrets, params)
            .await
    }

    /// Stop all running device skills cleanly.
    pub async fn stop_all(&mut self) {
        let ids: Vec<String> = self
            .skills
            .iter()
            .filter(|(_, m)| m.running.is_some())
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            let _ = self.stop_skill(&id).await;
        }
    }
}

/// Lock-free handle for re-provisioning a skill's virtual environment.
pub struct PreparedReinstall {
    skill_id: String,
    discovered: DiscoveredSkill,
    state: Arc<Mutex<SharedSkillState>>,
    events_tx: mpsc::UnboundedSender<SupervisorEvent>,
}

impl PreparedReinstall {
    pub async fn execute(self) -> Result<(), SupervisorError> {
        set_skill_status(
            &self.state,
            &self.events_tx,
            &self.skill_id,
            SkillStatus::Installing,
            None,
        );
        match provision_skill_env(&self.discovered, true).await {
            Ok(_) => {
                set_skill_status(
                    &self.state,
                    &self.events_tx,
                    &self.skill_id,
                    SkillStatus::Stopped,
                    None,
                );
                Ok(())
            }
            Err(err) => {
                set_skill_status(
                    &self.state,
                    &self.events_tx,
                    &self.skill_id,
                    SkillStatus::Error,
                    Some(err.to_string()),
                );
                Err(SupervisorError::Env(err))
            }
        }
    }
}

/// Lock-free handle for dispatching an `entity.command` to a running `device` skill.
pub struct PreparedEntityCommand {
    skill_id: String,
    cmd_tx: mpsc::Sender<DeviceActorCommand>,
    invoke_timeout: Duration,
}

impl PreparedEntityCommand {
    pub async fn execute(self, params: EntityCommandParams) -> Result<Value, SupervisorError> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.cmd_tx
            .send(DeviceActorCommand::EntityCommand {
                params,
                ack: ack_tx,
            })
            .await
            .map_err(|_| SupervisorError::NotRunning(self.skill_id.clone()))?;

        match tokio::time::timeout(self.invoke_timeout, ack_rx).await {
            Ok(Ok(res)) => res,
            Ok(Err(_)) => Err(SupervisorError::NotRunning(self.skill_id)),
            Err(_) => Err(SupervisorError::Timeout(
                self.skill_id,
                "entity.command timeout".into(),
            )),
        }
    }
}

/// Lock-free handle for running an on-demand `action` skill invocation.
pub struct PreparedActionInvoke {
    skill_id: String,
    discovered: DiscoveredSkill,
    state: Arc<Mutex<SharedSkillState>>,
    logs: LogRingBuffer,
    events_tx: mpsc::UnboundedSender<SupervisorEvent>,
    timing: SupervisorTimingConfig,
}

impl PreparedActionInvoke {
    pub async fn execute(
        self,
        config: Map<String, Value>,
        secrets: BTreeMap<String, String>,
        params: SkillInvokeParams,
    ) -> Result<Value, SupervisorError> {
        let skill_id = &self.skill_id;
        set_skill_status(
            &self.state,
            &self.events_tx,
            skill_id,
            SkillStatus::Installing,
            None,
        );
        let resolved = match provision_skill_env(&self.discovered, false).await {
            Ok(cmd) => cmd,
            Err(err) => {
                set_skill_status(
                    &self.state,
                    &self.events_tx,
                    skill_id,
                    SkillStatus::Error,
                    Some(err.to_string()),
                );
                return Err(SupervisorError::Env(err));
            }
        };

        let mut proc = spawn_skill_process(skill_id, &resolved, self.logs.clone())?;
        set_skill_status(
            &self.state,
            &self.events_tx,
            skill_id,
            SkillStatus::Running,
            None,
        );

        let invoke_timeout =
            Duration::from_secs(self.discovered.manifest.runtime.invoke_timeout_secs);
        let outcome = async {
            perform_init_handshake(
                &mut proc,
                skill_id,
                config,
                secrets,
                self.timing.init_timeout,
                &self.events_tx,
            )
            .await?;

            let req = RpcRequest::new(
                RpcId::Number(2),
                "skill.invoke",
                Some(
                    serde_json::to_value(&params)
                        .map_err(|e| SupervisorError::Rpc(e.to_string()))?,
                ),
            );
            proc.send_json(&req).await?;

            wait_for_rpc_response(
                &mut proc,
                skill_id,
                &RpcId::Number(2),
                invoke_timeout,
                &self.events_tx,
            )
            .await
        }
        .await;

        proc.shutdown(self.timing.shutdown_grace).await;

        match &outcome {
            Ok(_) => set_skill_status(
                &self.state,
                &self.events_tx,
                skill_id,
                SkillStatus::Stopped,
                None,
            ),
            Err(err) => set_skill_status(
                &self.state,
                &self.events_tx,
                skill_id,
                SkillStatus::Error,
                Some(err.to_string()),
            ),
        }

        outcome
    }
}

struct SkillChildProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout_lines: Lines<BufReader<ChildStdout>>,
    stderr_task: JoinHandle<()>,
}

impl SkillChildProcess {
    async fn send_json<T: Serialize>(&mut self, value: &T) -> Result<(), SupervisorError> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| SupervisorError::Rpc("skill stdin closed".into()))?;
        let mut line =
            serde_json::to_string(value).map_err(|e| SupervisorError::Rpc(e.to_string()))?;
        line.push('\n');
        stdin.write_all(line.as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }

    async fn next_line(&mut self) -> Result<Option<String>, SupervisorError> {
        Ok(self.stdout_lines.next_line().await?)
    }

    async fn shutdown(mut self, grace: Duration) {
        let notif = RpcNotification::new("skill.shutdown", None);
        let _ = self.send_json(&notif).await;
        drop(self.stdin.take());

        match tokio::time::timeout(grace, self.child.wait()).await {
            Ok(_) => {}
            Err(_) => {
                let _ = self.child.kill().await;
                let _ = self.child.wait().await;
            }
        }
        let _ = self.stderr_task.await;
    }
}

fn spawn_skill_process(
    skill_id: &str,
    resolved: &ResolvedCommand,
    logs: LogRingBuffer,
) -> Result<SkillChildProcess, SupervisorError> {
    let mut cmd = Command::new(&resolved.program);
    cmd.args(&resolved.args)
        .current_dir(&resolved.working_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        // Restricted environment: never pass Core secrets or unrelated host env vars.
        .env_clear()
        .env("PYTHONUNBUFFERED", "1");

    for allowed in ["PATH", "HOME", "LANG", "LC_ALL", "TZ"] {
        if let Ok(val) = env::var(allowed) {
            cmd.env(allowed, val);
        }
    }
    if let Some(venv) = &resolved.venv_dir {
        cmd.env("VIRTUAL_ENV", venv);
    }

    let mut child = cmd.spawn()?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| SupervisorError::Rpc("failed to open child stdin".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| SupervisorError::Rpc("failed to open child stdout".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| SupervisorError::Rpc("failed to open child stderr".into()))?;

    let sid = skill_id.to_owned();
    let stderr_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            debug!(skill_id = %sid, line = %line, "skill stderr");
            logs.push(line);
        }
    });

    Ok(SkillChildProcess {
        child,
        stdin: Some(stdin),
        stdout_lines: BufReader::new(stdout).lines(),
        stderr_task,
    })
}

#[allow(clippy::too_many_arguments)]
async fn run_device_skill_actor(
    discovered: DiscoveredSkill,
    config: Map<String, Value>,
    secrets: BTreeMap<String, String>,
    state: Arc<Mutex<SharedSkillState>>,
    logs: LogRingBuffer,
    events_tx: mpsc::UnboundedSender<SupervisorEvent>,
    mut cmd_rx: mpsc::Receiver<DeviceActorCommand>,
    mut stop_rx: watch::Receiver<bool>,
    timing: SupervisorTimingConfig,
) {
    let skill_id = discovered.manifest.skill.id.clone();
    let mut backoff = timing.initial_backoff;

    loop {
        if is_stopped(&stop_rx) {
            break;
        }

        set_skill_status(&state, &events_tx, &skill_id, SkillStatus::Installing, None);
        let provision_res = tokio::select! {
            biased;
            changed = stop_rx.changed() => {
                if changed.is_err() || *stop_rx.borrow() {
                    break;
                }
                continue;
            }
            res = provision_skill_env(&discovered, false) => res,
        };
        let resolved = match provision_res {
            Ok(cmd) => cmd,
            Err(err) => {
                set_skill_status(
                    &state,
                    &events_tx,
                    &skill_id,
                    SkillStatus::Error,
                    Some(err.to_string()),
                );
                if discovered.manifest.runtime.restart_policy == RestartPolicy::Never {
                    break;
                }
                if !sleep_or_stop(&mut stop_rx, backoff).await {
                    break;
                }
                backoff = (backoff.saturating_mul(2)).min(timing.max_backoff);
                continue;
            }
        };

        let mut proc = match spawn_skill_process(&skill_id, &resolved, logs.clone()) {
            Ok(p) => p,
            Err(err) => {
                set_skill_status(
                    &state,
                    &events_tx,
                    &skill_id,
                    SkillStatus::Error,
                    Some(err.to_string()),
                );
                if discovered.manifest.runtime.restart_policy == RestartPolicy::Never {
                    break;
                }
                if !sleep_or_stop(&mut stop_rx, backoff).await {
                    break;
                }
                backoff = (backoff.saturating_mul(2)).min(timing.max_backoff);
                continue;
            }
        };

        let init_res = tokio::select! {
            biased;
            changed = stop_rx.changed() => {
                if changed.is_err() || *stop_rx.borrow() {
                    proc.shutdown(timing.shutdown_grace).await;
                    set_skill_status(&state, &events_tx, &skill_id, SkillStatus::Stopped, None);
                    break;
                }
                Err(SupervisorError::Rpc("stop signal toggled during init".into()))
            }
            res = perform_init_handshake(
                &mut proc,
                &skill_id,
                config.clone(),
                secrets.clone(),
                timing.init_timeout,
                &events_tx,
            ) => res,
        };

        if let Err(err) = init_res {
            warn!(skill_id = %skill_id, error = %err, "skill.init handshake failed");
            proc.shutdown(timing.shutdown_grace).await;
            bump_restart_and_error(&state, &events_tx, &skill_id, err.to_string());
            if discovered.manifest.runtime.restart_policy == RestartPolicy::Never {
                break;
            }
            if !sleep_or_stop(&mut stop_rx, backoff).await {
                break;
            }
            backoff = (backoff.saturating_mul(2)).min(timing.max_backoff);
            continue;
        }

        // Handshake succeeded — mark Running and reset backoff.
        backoff = timing.initial_backoff;
        set_skill_status(&state, &events_tx, &skill_id, SkillStatus::Running, None);
        info!(skill_id = %skill_id, "device skill running");

        let hb_interval = timing.heartbeat_interval_override.unwrap_or_else(|| {
            Duration::from_secs(discovered.manifest.runtime.heartbeat_interval_secs)
        });

        let exit_reason = run_device_session(
            &mut proc,
            &skill_id,
            &events_tx,
            &mut cmd_rx,
            &mut stop_rx,
            hb_interval,
            timing.ping_timeout,
        )
        .await;

        proc.shutdown(timing.shutdown_grace).await;

        if is_stopped(&stop_rx) {
            set_skill_status(&state, &events_tx, &skill_id, SkillStatus::Stopped, None);
            break;
        }

        let err_msg = match exit_reason {
            Ok(()) => "skill process exited unexpectedly".to_owned(),
            Err(err) => err.to_string(),
        };
        warn!(skill_id = %skill_id, reason = %err_msg, "device skill crashed or failed heartbeat");
        bump_restart_and_error(&state, &events_tx, &skill_id, err_msg);

        if discovered.manifest.runtime.restart_policy == RestartPolicy::Never {
            break;
        }
        if !sleep_or_stop(&mut stop_rx, backoff).await {
            break;
        }
        backoff = (backoff.saturating_mul(2)).min(timing.max_backoff);
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_device_session(
    proc: &mut SkillChildProcess,
    skill_id: &str,
    events_tx: &mpsc::UnboundedSender<SupervisorEvent>,
    cmd_rx: &mut mpsc::Receiver<DeviceActorCommand>,
    stop_rx: &mut watch::Receiver<bool>,
    hb_interval: Duration,
    ping_timeout: Duration,
) -> Result<(), SupervisorError> {
    let mut next_id: i64 = 10;
    let mut pending_cmds: HashMap<RpcId, oneshot::Sender<Result<Value, SupervisorError>>> =
        HashMap::new();
    let mut pending_ping_id: Option<RpcId> = None;
    let mut missed_pings: u32 = 0;

    let mut hb_ticker = tokio::time::interval(hb_interval);
    hb_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    hb_ticker.tick().await; // skip immediate tick

    let ping_deadline = tokio::time::sleep(Duration::from_secs(86_400));
    tokio::pin!(ping_deadline);

    loop {
        tokio::select! {
            biased;
            changed = stop_rx.changed() => {
                if changed.is_err() || *stop_rx.borrow() {
                    fail_pending_cmds(skill_id, &mut pending_cmds);
                    return Ok(());
                }
            }
            line_res = proc.next_line() => {
                match line_res? {
                    Some(line) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        handle_skill_stdout_line(
                            proc,
                            skill_id,
                            trimmed,
                            events_tx,
                            &mut pending_cmds,
                            &mut pending_ping_id,
                            &mut missed_pings,
                        )
                        .await?;
                        if pending_ping_id.is_none() {
                            ping_deadline.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(86_400));
                        }
                    }
                    None => {
                        fail_pending_cmds(skill_id, &mut pending_cmds);
                        return Err(SupervisorError::Rpc("skill stdout closed (process exited)".into()));
                    }
                }
            }
            Some(cmd) = cmd_rx.recv() => {
                let DeviceActorCommand::EntityCommand { params, ack } = cmd;
                let id = RpcId::Number(next_id);
                next_id += 1;
                let req = RpcRequest::new(
                    id.clone(),
                    "entity.command",
                    Some(serde_json::to_value(&params).map_err(|e| SupervisorError::Rpc(e.to_string()))?),
                );
                if let Err(err) = proc.send_json(&req).await {
                    let _ = ack.send(Err(err));
                    fail_pending_cmds(skill_id, &mut pending_cmds);
                    return Err(SupervisorError::Rpc("failed to write entity.command to skill".into()));
                }
                pending_cmds.insert(id, ack);
            }
            _ = hb_ticker.tick() => {
                if pending_ping_id.is_none() {
                    let id = RpcId::Number(next_id);
                    next_id += 1;
                    let req = RpcRequest::new(id.clone(), "skill.ping", None);
                    proc.send_json(&req).await?;
                    pending_ping_id = Some(id);
                    ping_deadline.as_mut().reset(tokio::time::Instant::now() + ping_timeout);
                }
            }
            () = &mut ping_deadline, if pending_ping_id.is_some() => {
                missed_pings += 1;
                pending_ping_id = None;
                warn!(skill_id = %skill_id, missed_pings, "skill missed heartbeat ping");
                if missed_pings >= MAX_MISSED_HEARTBEATS {
                    fail_pending_cmds(skill_id, &mut pending_cmds);
                    return Err(SupervisorError::Timeout(
                        skill_id.to_owned(),
                        format!("{MAX_MISSED_HEARTBEATS} consecutive missed heartbeats"),
                    ));
                }
                ping_deadline.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(86_400));
            }
        }
    }
}

async fn handle_skill_stdout_line(
    proc: &mut SkillChildProcess,
    skill_id: &str,
    line: &str,
    events_tx: &mpsc::UnboundedSender<SupervisorEvent>,
    pending_cmds: &mut HashMap<RpcId, oneshot::Sender<Result<Value, SupervisorError>>>,
    pending_ping_id: &mut Option<RpcId>,
    missed_pings: &mut u32,
) -> Result<(), SupervisorError> {
    let val: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(err) => {
            warn!(skill_id = %skill_id, error = %err, "ignoring non-JSON line on skill stdout");
            return Ok(());
        }
    };

    // Case 1: JSON-RPC Response (has "result" or "error", no "method")
    if val.get("method").is_none() {
        let resp: RpcResponse =
            serde_json::from_value(val).map_err(|e| SupervisorError::Rpc(e.to_string()))?;
        if let Some(id) = resp.id {
            if pending_ping_id.as_ref() == Some(&id) {
                *pending_ping_id = None;
                *missed_pings = 0;
                return Ok(());
            }
            if let Some(ack) = pending_cmds.remove(&id) {
                let res = match (resp.result, resp.error) {
                    (_, Some(err)) => Err(SupervisorError::Rpc(format!(
                        "code {}: {}",
                        err.code, err.message
                    ))),
                    (Some(r), None) => Ok(r),
                    (None, None) => Ok(Value::Null),
                };
                let _ = ack.send(res);
            }
        }
        return Ok(());
    }

    // Case 2: Incoming JSON-RPC Request from Skill (e.g., core.get_entity / core.call_entity)
    if let Some(id_val) = val.get("id").cloned() {
        let req: RpcRequest =
            serde_json::from_value(val).map_err(|e| SupervisorError::Rpc(e.to_string()))?;
        let id: RpcId =
            serde_json::from_value(id_val).map_err(|e| SupervisorError::Rpc(e.to_string()))?;
        let (reply_tx, reply_rx) = oneshot::channel();
        let _ = events_tx.send(SupervisorEvent::CoreCallback {
            skill_id: skill_id.to_owned(),
            method: req.method,
            params: req.params,
            reply_tx,
        });
        let reply = match reply_rx.await {
            Ok(Ok(result)) => RpcResponse::ok(id, result),
            Ok(Err(msg)) => RpcResponse::err(Some(id), -32000, msg),
            Err(_) => RpcResponse::err(Some(id), -32603, "core callback handler unavailable"),
        };
        proc.send_json(&reply).await?;
        return Ok(());
    }

    // Case 3: Notification from Skill (e.g., entity.register, entity.state_changed)
    let notif: RpcNotification =
        serde_json::from_value(val).map_err(|e| SupervisorError::Rpc(e.to_string()))?;
    let _ = events_tx.send(SupervisorEvent::SkillNotification {
        skill_id: skill_id.to_owned(),
        method: notif.method,
        params: notif.params,
    });
    Ok(())
}

async fn perform_init_handshake(
    proc: &mut SkillChildProcess,
    skill_id: &str,
    config: Map<String, Value>,
    secrets: BTreeMap<String, String>,
    timeout: Duration,
    events_tx: &mpsc::UnboundedSender<SupervisorEvent>,
) -> Result<(), SupervisorError> {
    let init_params = SkillInitParams {
        protocol_version: PROTOCOL_VERSION.to_owned(),
        skill_id: skill_id.to_owned(),
        config,
        secrets,
    };
    let req = RpcRequest {
        jsonrpc: JSONRPC_VERSION.to_owned(),
        id: RpcId::Number(1),
        method: "skill.init".to_owned(),
        params: Some(
            serde_json::to_value(&init_params).map_err(|e| SupervisorError::Rpc(e.to_string()))?,
        ),
    };
    proc.send_json(&req).await?;

    let result =
        wait_for_rpc_response(proc, skill_id, &RpcId::Number(1), timeout, events_tx).await?;
    if result.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(SupervisorError::Rpc(format!(
            "skill.init returned non-ok result: {}",
            json!(result)
        )));
    }
    Ok(())
}

async fn wait_for_rpc_response(
    proc: &mut SkillChildProcess,
    skill_id: &str,
    expected_id: &RpcId,
    timeout: Duration,
    events_tx: &mpsc::UnboundedSender<SupervisorEvent>,
) -> Result<Value, SupervisorError> {
    tokio::time::timeout(timeout, async {
        loop {
            let Some(line) = proc.next_line().await? else {
                return Err(SupervisorError::Rpc(
                    "skill exited before responding to RPC request".into(),
                ));
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let val: Value = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(_) => continue,
            };

            if val.get("method").is_none() {
                let resp: RpcResponse =
                    serde_json::from_value(val).map_err(|e| SupervisorError::Rpc(e.to_string()))?;
                if resp.id.as_ref() == Some(expected_id) {
                    if let Some(err) = resp.error {
                        return Err(SupervisorError::Rpc(format!(
                            "code {}: {}",
                            err.code, err.message
                        )));
                    }
                    return Ok(resp.result.unwrap_or(Value::Null));
                }
            } else if val.get("id").is_some() {
                // Handle callback request from Skill during init or invoke!
                let req: RpcRequest =
                    serde_json::from_value(val).map_err(|e| SupervisorError::Rpc(e.to_string()))?;
                let (reply_tx, reply_rx) = oneshot::channel();
                let _ = events_tx.send(SupervisorEvent::CoreCallback {
                    skill_id: skill_id.to_owned(),
                    method: req.method,
                    params: req.params,
                    reply_tx,
                });
                let reply = match reply_rx.await {
                    Ok(Ok(result)) => RpcResponse::ok(req.id, result),
                    Ok(Err(msg)) => RpcResponse::err(Some(req.id), -32000, msg),
                    Err(_) => RpcResponse::err(Some(req.id), -32603, "callback unavailable"),
                };
                proc.send_json(&reply).await?;
            } else {
                let notif: RpcNotification =
                    serde_json::from_value(val).map_err(|e| SupervisorError::Rpc(e.to_string()))?;
                let _ = events_tx.send(SupervisorEvent::SkillNotification {
                    skill_id: skill_id.to_owned(),
                    method: notif.method,
                    params: notif.params,
                });
            }
        }
    })
    .await
    .map_err(|_| SupervisorError::Timeout(skill_id.to_owned(), "RPC response timeout".into()))?
}

fn fail_pending_cmds(
    skill_id: &str,
    pending: &mut HashMap<RpcId, oneshot::Sender<Result<Value, SupervisorError>>>,
) {
    for (_, ack) in pending.drain() {
        let _ = ack.send(Err(SupervisorError::NotRunning(skill_id.to_owned())));
    }
}

fn set_skill_status(
    state: &Arc<Mutex<SharedSkillState>>,
    events_tx: &mpsc::UnboundedSender<SupervisorEvent>,
    skill_id: &str,
    status: SkillStatus,
    last_error: Option<String>,
) {
    let restart_count = {
        let mut st = state.lock().expect("skill state mutex poisoned");
        st.status = status;
        st.last_error = last_error.clone();
        st.restart_count
    };
    let _ = events_tx.send(SupervisorEvent::StatusChanged {
        skill_id: skill_id.to_owned(),
        status,
        restart_count,
        last_error,
    });
}

fn bump_restart_and_error(
    state: &Arc<Mutex<SharedSkillState>>,
    events_tx: &mpsc::UnboundedSender<SupervisorEvent>,
    skill_id: &str,
    error: String,
) {
    let restart_count = {
        let mut st = state.lock().expect("skill state mutex poisoned");
        st.status = SkillStatus::Error;
        st.restart_count = st.restart_count.saturating_add(1);
        st.last_error = Some(error.clone());
        st.restart_count
    };
    let _ = events_tx.send(SupervisorEvent::StatusChanged {
        skill_id: skill_id.to_owned(),
        status: SkillStatus::Error,
        restart_count,
        last_error: Some(error),
    });
}

fn is_stopped(rx: &watch::Receiver<bool>) -> bool {
    *rx.borrow() || rx.has_changed().is_err()
}

async fn sleep_or_stop(stop_rx: &mut watch::Receiver<bool>, dur: Duration) -> bool {
    tokio::select! {
        biased;
        changed = stop_rx.changed() => {
            !(changed.is_err() || *stop_rx.borrow())
        }
        () = tokio::time::sleep(dur) => true,
    }
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
