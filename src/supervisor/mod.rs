pub mod env;
pub mod manifest;
pub mod process;
pub mod uds;

use std::env as std_env;
use std::sync::Arc;

use tokio::sync::{Mutex as AsyncMutex, mpsc, watch};
use tracing::{info, warn};

use crate::protocol::rpc::PROTOCOL_VERSION;
pub use manifest::{DiscoveredSkill, ManifestError, SkillManifest, scan_skills_dir};
pub use process::{
    DEFAULT_LOG_RING_CAPACITY, LogEntry, LogRingBuffer, SkillRuntimeSummary, SupervisorError,
    SupervisorEvent, SupervisorManager, SupervisorTimingConfig,
};
pub use uds::run_uds_server;

pub const DEFAULT_RUNTIME_SOCKET: &str = "/run/xiaohe/rpc.sock";
pub const DEFAULT_SKILLS_DIR: &str = "/skills";

/// Entrypoint for `xiaohe supervisor`.
pub async fn run() -> std::io::Result<()> {
    let socket =
        std_env::var("RUNTIME_SOCKET").unwrap_or_else(|_| DEFAULT_RUNTIME_SOCKET.to_owned());
    let skills_dir = std_env::var("SKILLS_DIR").unwrap_or_else(|_| DEFAULT_SKILLS_DIR.to_owned());

    let (events_tx, events_rx) = mpsc::unbounded_channel();
    let mut manager = SupervisorManager::new(&skills_dir, events_tx);
    let errors = manager.scan();
    if !errors.is_empty() {
        warn!(
            error_count = errors.len(),
            "some skill manifests failed validation"
        );
    }

    info!(
        version = env!("CARGO_PKG_VERSION"),
        protocol_version = PROTOCOL_VERSION,
        socket = %socket,
        skills_dir = %skills_dir,
        discovered_skills = manager.list_summaries().len(),
        "xiaohe supervisor initialized"
    );

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    tokio::spawn(async move {
        shutdown_signal().await;
        let _ = shutdown_tx.send(true);
    });

    let manager = Arc::new(AsyncMutex::new(manager));
    run_uds_server(&socket, manager, events_rx, shutdown_rx).await
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            tracing::error!(error = %err, "failed to install Ctrl+C signal handler");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sigterm) => {
                sigterm.recv().await;
            }
            Err(err) => {
                tracing::error!(error = %err, "failed to install SIGTERM signal handler");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => info!("supervisor received SIGINT, initiating graceful shutdown"),
        () = terminate => info!("supervisor received SIGTERM, initiating graceful shutdown"),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::time::Duration;

    use serde_json::{Map, json};
    use tempfile::TempDir;
    use tokio::sync::mpsc;

    use super::*;
    use crate::protocol::rpc::{EntityCommandParams, SkillInvokeParams};
    use crate::protocol::skill::{SkillMode, SkillStatus};

    fn fast_timing() -> SupervisorTimingConfig {
        SupervisorTimingConfig {
            init_timeout: Duration::from_secs(5),
            shutdown_grace: Duration::from_secs(2),
            initial_backoff: Duration::from_millis(50),
            max_backoff: Duration::from_millis(200),
            ping_timeout: Duration::from_millis(100),
            heartbeat_interval_override: Some(Duration::from_millis(80)),
        }
    }

    #[test]
    fn manifest_scanner_validates_and_isolates_broken_skills() {
        let dir = TempDir::new().unwrap();

        // Valid device skill
        let valid_dir = dir.path().join("miio-airpurifier");
        fs::create_dir_all(&valid_dir).unwrap();
        fs::write(
            valid_dir.join("skill.toml"),
            r#"
[skill]
id = "miio-airpurifier"
name = "Xiaomi Air Purifier"
version = "0.1.0"
mode = "device"

[runtime]
type = "python-uv"
python = ">=3.11"
entrypoint = ["python", "src/main.py"]
heartbeat_interval_secs = 30
invoke_timeout_secs = 15
restart_policy = "exponential"

[secrets.miio_token]
required = true
description = "32-char hex token"
"#,
        )
        .unwrap();

        // Broken action skill (missing [actions.*])
        let bad_dir = dir.path().join("bad-action");
        fs::create_dir_all(&bad_dir).unwrap();
        fs::write(
            bad_dir.join("skill.toml"),
            r#"
[skill]
id = "bad-action"
name = "Broken Action"
version = "0.1.0"
mode = "action"

[runtime]
type = "executable"
entrypoint = ["python3", "main.py"]
"#,
        )
        .unwrap();

        let (found, errors) = scan_skills_dir(dir.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].manifest.skill.id, "miio-airpurifier");
        assert_eq!(found[0].manifest.skill.mode, SkillMode::Device);
        assert_eq!(
            found[0].manifest.declared_secret_keys(),
            vec!["miio_token".to_owned()]
        );
        assert_eq!(errors.len(), 1);
        assert!(matches!(errors[0], ManifestError::Validation { .. }));
    }

    #[test]
    fn log_ring_buffer_keeps_only_latest_entries() {
        let ring = LogRingBuffer::new(3);
        for i in 1..=5 {
            ring.push(format!("line-{i}"));
        }
        let tail = ring.tail(10);
        let lines: Vec<&str> = tail.iter().map(|e| e.line.as_str()).collect();
        assert_eq!(lines, vec!["line-3", "line-4", "line-5"]);
        assert_eq!(tail[0].stream, "stderr");
    }

    #[tokio::test]
    async fn device_skill_lifecycle_commands_logs_and_env_isolation() {
        let dir = TempDir::new().unwrap();
        let skill_dir = dir.path().join("mock-device");
        fs::create_dir_all(&skill_dir).unwrap();

        fs::write(
            skill_dir.join("skill.toml"),
            r#"
[skill]
id = "mock-device"
name = "Mock Device"
version = "0.1.0"
mode = "device"

[runtime]
type = "executable"
entrypoint = ["python3", "main.py"]
heartbeat_interval_secs = 30
invoke_timeout_secs = 5
restart_policy = "never"

[secrets.api_token]
required = true
"#,
        )
        .unwrap();

        fs::write(
            skill_dir.join("main.py"),
            r#"
import json, os, sys

def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()

print("booting mock device", file=sys.stderr, flush=True)

for raw in sys.stdin:
    msg = json.loads(raw)
    method = msg.get("method")
    req_id = msg.get("id")
    if method == "skill.init":
        # Verify host secrets are NOT leaked via os.environ
        leaked = "UNRELATED_HOST_SECRET" in os.environ
        print(f"init ok leaked={leaked}", file=sys.stderr, flush=True)
        send({"jsonrpc": "2.0", "id": req_id, "result": {"ok": not leaked}})
        send({
            "jsonrpc": "2.0",
            "method": "entity.state_changed",
            "params": {
                "entity_id": "switch.mock_power",
                "availability": "online",
                "state": {"domain": "switch", "on": False}
            }
        })
    elif method == "skill.ping":
        send({"jsonrpc": "2.0", "id": req_id, "result": {"ok": True}})
    elif method == "entity.command":
        p = msg["params"]
        send({"jsonrpc": "2.0", "id": req_id, "result": {"ok": True, "echo_action": p["action"]}})
    elif method == "skill.shutdown":
        print("shutting down cleanly", file=sys.stderr, flush=True)
        break
"#,
        )
        .unwrap();

        let (events_tx, mut events_rx) = mpsc::unbounded_channel();
        let mut mgr = SupervisorManager::with_timing(dir.path(), events_tx, fast_timing());
        assert!(mgr.scan().is_empty());

        let mut secrets = BTreeMap::new();
        secrets.insert("api_token".into(), "tok-123".into());
        mgr.start_device_skill("mock-device", Map::new(), secrets)
            .await
            .expect("start device skill");

        // Wait for entity.state_changed notification
        let mut saw_state = false;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        while tokio::time::Instant::now() < deadline {
            if let Ok(Some(ev)) =
                tokio::time::timeout(Duration::from_millis(200), events_rx.recv()).await
                && let SupervisorEvent::SkillNotification {
                    skill_id, method, ..
                } = ev
                && skill_id == "mock-device"
                && method == "entity.state_changed"
            {
                saw_state = true;
                break;
            }
        }
        assert!(saw_state, "expected entity.state_changed notification");

        // Send entity.command and verify JSON-RPC response
        let res = mgr
            .send_entity_command(
                "mock-device",
                EntityCommandParams {
                    entity_id: "switch.mock_power".into(),
                    action: "turn_on".into(),
                    params: Map::new(),
                },
            )
            .await
            .expect("entity.command");
        assert_eq!(res["ok"], true);
        assert_eq!(res["echo_action"], "turn_on");

        // Verify stderr logs captured in ring buffer
        let logs = mgr.get_logs("mock-device", 50).unwrap();
        assert!(logs.iter().any(|l| l.line.contains("init ok leaked=False")));

        mgr.stop_skill("mock-device").await.expect("stop skill");
        let summary = mgr
            .list_summaries()
            .into_iter()
            .find(|s| s.skill_id == "mock-device")
            .unwrap();
        assert_eq!(summary.status, SkillStatus::Stopped);
    }

    #[tokio::test]
    async fn missed_heartbeats_trigger_restart() {
        let dir = TempDir::new().unwrap();
        let skill_dir = dir.path().join("silent-ping");
        fs::create_dir_all(&skill_dir).unwrap();

        fs::write(
            skill_dir.join("skill.toml"),
            r#"
[skill]
id = "silent-ping"
name = "Silent Ping"
version = "0.1.0"
mode = "device"

[runtime]
type = "executable"
entrypoint = ["python3", "main.py"]
heartbeat_interval_secs = 1
invoke_timeout_secs = 5
restart_policy = "exponential"
"#,
        )
        .unwrap();

        // Responds to skill.init, then intentionally ignores skill.ping
        fs::write(
            skill_dir.join("main.py"),
            r#"
import json, sys
for raw in sys.stdin:
    msg = json.loads(raw)
    if msg.get("method") == "skill.init":
        sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": msg["id"], "result": {"ok": True}}) + "\n")
        sys.stdout.flush()
    elif msg.get("method") == "skill.shutdown":
        break
"#,
        )
        .unwrap();

        let (events_tx, _events_rx) = mpsc::unbounded_channel();
        let mut mgr = SupervisorManager::with_timing(dir.path(), events_tx, fast_timing());
        mgr.scan();
        mgr.start_device_skill("silent-ping", Map::new(), BTreeMap::new())
            .await
            .unwrap();

        let mut restarted = false;
        for _ in 0..40 {
            if let Some(s) = mgr
                .list_summaries()
                .into_iter()
                .find(|s| s.skill_id == "silent-ping")
                && s.restart_count >= 1
            {
                restarted = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(restarted, "expected skill to restart after 3 missed pings");
        mgr.stop_all().await;
    }

    #[tokio::test]
    async fn action_skill_invoke_with_core_callback_and_uv_venv() {
        let dir = TempDir::new().unwrap();
        let skill_dir = dir.path().join("calc-action");
        fs::create_dir_all(&skill_dir).unwrap();

        fs::write(
            skill_dir.join("skill.toml"),
            r#"
[skill]
id = "calc-action"
name = "Calc Action"
version = "0.1.0"
mode = "action"

[runtime]
type = "python-uv"
entrypoint = ["python", "main.py"]
invoke_timeout_secs = 10

[actions.compute_comfort]
description = "Compute comfort using core.get_entity callback"
"#,
        )
        .unwrap();

        fs::write(
            skill_dir.join("main.py"),
            r#"
import json, sys

def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()

for raw in sys.stdin:
    msg = json.loads(raw)
    method = msg.get("method")
    req_id = msg.get("id")
    if method == "skill.init":
        send({"jsonrpc": "2.0", "id": req_id, "result": {"ok": True}})
    elif method == "skill.invoke":
        # Call back to Core via core.get_entity before answering
        send({"jsonrpc": "2.0", "id": 100, "method": "core.get_entity", "params": {"entity_id": "sensor.temp"}})
        cb_resp = json.loads(sys.stdin.readline())
        temp = cb_resp["result"]["value"]
        send({"jsonrpc": "2.0", "id": req_id, "result": {"comfort": "warm" if temp > 25 else "ok", "temp": temp}})
    elif method == "skill.shutdown":
        break
"#,
        )
        .unwrap();

        let (events_tx, mut events_rx) = mpsc::unbounded_channel();
        let cb_task = tokio::spawn(async move {
            while let Some(ev) = events_rx.recv().await {
                if let SupervisorEvent::CoreCallback {
                    method, reply_tx, ..
                } = ev
                    && method == "core.get_entity"
                {
                    let _ = reply_tx.send(Ok(json!({ "value": 27.5 })));
                }
            }
        });

        let mut mgr = SupervisorManager::with_timing(dir.path(), events_tx, fast_timing());
        assert!(mgr.scan().is_empty());

        let result = mgr
            .invoke_action_skill(
                "calc-action",
                Map::new(),
                BTreeMap::new(),
                SkillInvokeParams {
                    action: "compute_comfort".into(),
                    params: Map::new(),
                    context: Map::new(),
                },
            )
            .await
            .expect("invoke action skill");

        assert_eq!(result["comfort"], "warm");
        assert_eq!(result["temp"], 27.5);
        assert!(
            skill_dir.join(".venv").join("bin").join("python").exists(),
            "uv venv should have provisioned .venv/bin/python"
        );

        cb_task.abort();
    }

    #[test]
    fn duplicate_skill_id_is_reported_as_validation_error() {
        let dir = TempDir::new().unwrap();
        for folder in ["dup-a", "dup-b"] {
            let d = dir.path().join(folder);
            fs::create_dir_all(&d).unwrap();
            fs::write(
                d.join("skill.toml"),
                r#"
[skill]
id = "same-id"
name = "Dup"
version = "0.1.0"
mode = "device"

[runtime]
type = "executable"
entrypoint = ["python3", "main.py"]
"#,
            )
            .unwrap();
        }

        let (found, errors) = scan_skills_dir(dir.path());
        assert_eq!(found.len(), 1);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].to_string().contains("duplicate skill.id"));
    }

    #[tokio::test]
    async fn entity_command_times_out_when_skill_hangs() {
        let dir = TempDir::new().unwrap();
        let skill_dir = dir.path().join("hung-cmd");
        fs::create_dir_all(&skill_dir).unwrap();

        fs::write(
            skill_dir.join("skill.toml"),
            r#"
[skill]
id = "hung-cmd"
name = "Hung Command"
version = "0.1.0"
mode = "device"

[runtime]
type = "executable"
entrypoint = ["python3", "main.py"]
heartbeat_interval_secs = 30
invoke_timeout_secs = 1
restart_policy = "never"
"#,
        )
        .unwrap();

        fs::write(
            skill_dir.join("main.py"),
            r#"
import json, sys
for raw in sys.stdin:
    msg = json.loads(raw)
    m = msg.get("method")
    if m == "skill.init":
        sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": msg["id"], "result": {"ok": True}}) + "\n")
        sys.stdout.flush()
    elif m == "skill.ping":
        sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": msg["id"], "result": {"ok": True}}) + "\n")
        sys.stdout.flush()
    elif m == "skill.shutdown":
        break
    # Intentionally ignore entity.command to trigger invoke_timeout_secs
"#,
        )
        .unwrap();

        let (events_tx, mut events_rx) = mpsc::unbounded_channel();
        let mut mgr = SupervisorManager::with_timing(dir.path(), events_tx, fast_timing());
        assert!(mgr.scan().is_empty());
        mgr.start_device_skill("hung-cmd", Map::new(), BTreeMap::new())
            .await
            .unwrap();

        // Wait until Running
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        while tokio::time::Instant::now() < deadline {
            if let Ok(Some(SupervisorEvent::StatusChanged {
                status: SkillStatus::Running,
                ..
            })) = tokio::time::timeout(Duration::from_millis(200), events_rx.recv()).await
            {
                break;
            }
        }

        let err = mgr
            .send_entity_command(
                "hung-cmd",
                EntityCommandParams {
                    entity_id: "switch.x".into(),
                    action: "turn_on".into(),
                    params: Map::new(),
                },
            )
            .await
            .expect_err("command should time out");
        assert!(matches!(err, SupervisorError::Timeout(..)));
        mgr.stop_all().await;
    }

    #[tokio::test]
    async fn stop_skill_cancels_hung_init_promptly() {
        let dir = TempDir::new().unwrap();
        let skill_dir = dir.path().join("hung-init");
        fs::create_dir_all(&skill_dir).unwrap();

        fs::write(
            skill_dir.join("skill.toml"),
            r#"
[skill]
id = "hung-init"
name = "Hung Init"
version = "0.1.0"
mode = "device"

[runtime]
type = "executable"
entrypoint = ["python3", "main.py"]
restart_policy = "never"
"#,
        )
        .unwrap();

        // Never responds to skill.init, exits on stdin close / skill.shutdown
        fs::write(
            skill_dir.join("main.py"),
            r#"
import sys
for _ in sys.stdin:
    pass
"#,
        )
        .unwrap();

        let (events_tx, _events_rx) = mpsc::unbounded_channel();
        let timing = SupervisorTimingConfig {
            init_timeout: Duration::from_secs(30),
            shutdown_grace: Duration::from_millis(500),
            ..fast_timing()
        };
        let mut mgr = SupervisorManager::with_timing(dir.path(), events_tx, timing);
        assert!(mgr.scan().is_empty());
        mgr.start_device_skill("hung-init", Map::new(), BTreeMap::new())
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(100)).await;
        let started = tokio::time::Instant::now();
        mgr.stop_skill("hung-init").await.unwrap();
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "stop_skill should cancel hung skill.init well before init_timeout (30s)"
        );
    }
}
