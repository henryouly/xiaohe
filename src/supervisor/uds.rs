use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Mutex as AsyncMutex, mpsc, oneshot, watch};
use tracing::{debug, info, warn};

use crate::protocol::rpc::{
    ERR_INTERNAL_ERROR, ERR_INVALID_PARAMS, ERR_METHOD_NOT_FOUND, RpcId, RpcNotification,
    RpcRequest, RpcResponse, SkillForwardedEventParams, SkillStatusChangedParams,
    SupervisorEntityCommandParams, SupervisorGetLogsParams, SupervisorInvokeSkillParams,
    SupervisorSkillIdParams, SupervisorStartSkillParams,
};
use crate::supervisor::process::{SupervisorEvent, SupervisorManager};

type CallbackMap = Arc<StdMutex<HashMap<RpcId, oneshot::Sender<Result<Value, String>>>>>;

struct ActiveConn {
    conn_id: u64,
    tx: mpsc::UnboundedSender<String>,
}

/// Bind a Unix Domain Socket at `socket_path` and serve bidirectional JSON-RPC 2.0 between
/// `xiaohe core` and `SupervisorManager` until `shutdown_rx` fires.
pub async fn run_uds_server(
    socket_path: impl AsRef<Path>,
    manager: Arc<AsyncMutex<SupervisorManager>>,
    mut events_rx: mpsc::UnboundedReceiver<SupervisorEvent>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> std::io::Result<()> {
    let socket_path = socket_path.as_ref().to_path_buf();
    if let Some(parent) = socket_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    if socket_path.exists() {
        let _ = fs::remove_file(&socket_path);
    }

    let listener = UnixListener::bind(&socket_path)?;
    info!(socket = %socket_path.display(), "supervisor UDS RPC server listening");

    let active_conn: Arc<StdMutex<Option<ActiveConn>>> = Arc::new(StdMutex::new(None));
    let pending_callbacks: CallbackMap = Arc::new(StdMutex::new(HashMap::new()));
    let next_conn_id = Arc::new(AtomicU64::new(1));
    let next_cb_id = Arc::new(AtomicU64::new(1));

    let events_conn = Arc::clone(&active_conn);
    let events_cbs = Arc::clone(&pending_callbacks);
    let events_cb_seq = Arc::clone(&next_cb_id);
    let mut events_shutdown = shutdown_rx.clone();

    let forwarder = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                changed = events_shutdown.changed() => {
                    if changed.is_err() || *events_shutdown.borrow() {
                        break;
                    }
                }
                maybe_ev = events_rx.recv() => {
                    let Some(ev) = maybe_ev else { break; };
                    forward_supervisor_event(ev, &events_conn, &events_cbs, &events_cb_seq);
                }
            }
        }
    });

    loop {
        tokio::select! {
            biased;
            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() {
                    break;
                }
            }
            accept_res = listener.accept() => {
                match accept_res {
                    Ok((stream, _addr)) => {
                        let conn_id = next_conn_id.fetch_add(1, Ordering::Relaxed);
                        let mgr = Arc::clone(&manager);
                        let active = Arc::clone(&active_conn);
                        let cbs = Arc::clone(&pending_callbacks);
                        let conn_shutdown = shutdown_rx.clone();
                        tokio::spawn(async move {
                            handle_core_connection(conn_id, stream, mgr, active, cbs, conn_shutdown).await;
                        });
                    }
                    Err(err) => {
                        warn!(error = %err, "failed to accept UDS connection");
                    }
                }
            }
        }
    }

    forwarder.abort();
    // Stop all running child skill processes cleanly before exiting.
    manager.lock().await.stop_all().await;
    cleanup_socket(&socket_path);
    info!(socket = %socket_path.display(), "supervisor UDS RPC server stopped");
    Ok(())
}

fn cleanup_socket(path: &PathBuf) {
    if path.exists() {
        let _ = fs::remove_file(path);
    }
}

fn forward_supervisor_event(
    event: SupervisorEvent,
    active_conn: &Arc<StdMutex<Option<ActiveConn>>>,
    pending_callbacks: &CallbackMap,
    next_cb_id: &AtomicU64,
) {
    match event {
        SupervisorEvent::StatusChanged {
            skill_id,
            status,
            restart_count,
            last_error,
        } => {
            let params = SkillStatusChangedParams {
                skill_id,
                status,
                restart_count,
                last_error,
            };
            let notif =
                RpcNotification::new("skill.status_changed", serde_json::to_value(params).ok());
            send_to_active_core(active_conn, &notif);
        }
        SupervisorEvent::SkillNotification {
            skill_id,
            method,
            params,
        } => {
            let forwarded = SkillForwardedEventParams {
                skill_id,
                method,
                params,
            };
            let notif = RpcNotification::new("skill.event", serde_json::to_value(forwarded).ok());
            send_to_active_core(active_conn, &notif);
        }
        SupervisorEvent::CoreCallback {
            skill_id: _,
            method,
            params,
            reply_tx,
        } => {
            let seq = next_cb_id.fetch_add(1, Ordering::Relaxed);
            let id = RpcId::String(format!("cb-{seq}"));
            let req = RpcRequest::new(id.clone(), method, params);
            let Ok(line) = serde_json::to_string(&req) else {
                let _ = reply_tx.send(Err("failed to serialize core callback".into()));
                return;
            };

            let tx = active_conn
                .lock()
                .expect("active_conn mutex poisoned")
                .as_ref()
                .map(|c| c.tx.clone());

            let Some(tx) = tx else {
                let _ = reply_tx.send(Err("core is not connected".into()));
                return;
            };

            pending_callbacks
                .lock()
                .expect("pending_callbacks mutex poisoned")
                .insert(id.clone(), reply_tx);

            if tx.send(line).is_err()
                && let Some(cb) = pending_callbacks
                    .lock()
                    .expect("pending_callbacks mutex poisoned")
                    .remove(&id)
            {
                let _ = cb.send(Err("core connection closed".into()));
            }
        }
    }
}

fn send_to_active_core<T: serde::Serialize>(
    active_conn: &Arc<StdMutex<Option<ActiveConn>>>,
    payload: &T,
) {
    let Ok(line) = serde_json::to_string(payload) else {
        return;
    };
    let tx = active_conn
        .lock()
        .expect("active_conn mutex poisoned")
        .as_ref()
        .map(|c| c.tx.clone());
    if let Some(tx) = tx {
        let _ = tx.send(line);
    }
}

async fn handle_core_connection(
    conn_id: u64,
    stream: UnixStream,
    manager: Arc<AsyncMutex<SupervisorManager>>,
    active_conn: Arc<StdMutex<Option<ActiveConn>>>,
    pending_callbacks: CallbackMap,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    info!(conn_id, "xiaohe core connected over UDS");
    let (read_half, mut write_half) = stream.into_split();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();

    {
        let mut guard = active_conn.lock().expect("active_conn mutex poisoned");
        *guard = Some(ActiveConn {
            conn_id,
            tx: out_tx.clone(),
        });
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
                        handle_core_line(trimmed, &manager, &out_tx, &pending_callbacks);
                    }
                    Ok(None) => {
                        debug!(conn_id, "xiaohe core UDS stream closed");
                        break;
                    }
                    Err(err) => {
                        warn!(conn_id, error = %err, "error reading from xiaohe core UDS");
                        break;
                    }
                }
            }
        }
    }

    // Clear active connection if it still belongs to us, and fail orphaned callbacks.
    {
        let mut guard = active_conn.lock().expect("active_conn mutex poisoned");
        if guard.as_ref().is_some_and(|c| c.conn_id == conn_id) {
            *guard = None;
            let mut cbs = pending_callbacks
                .lock()
                .expect("pending_callbacks mutex poisoned");
            for (_, tx) in cbs.drain() {
                let _ = tx.send(Err("core disconnected".into()));
            }
        }
    }

    drop(out_tx);
    let _ = writer_task.await;
    info!(conn_id, "xiaohe core UDS session ended");
}

fn handle_core_line(
    line: &str,
    manager: &Arc<AsyncMutex<SupervisorManager>>,
    out_tx: &mpsc::UnboundedSender<String>,
    pending_callbacks: &CallbackMap,
) {
    let val: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(err) => {
            warn!(error = %err, "ignoring invalid JSON line from core");
            return;
        }
    };

    // Case 1: Response to a Supervisor -> Core callback (core.get_entity / core.call_entity).
    if val.get("method").is_none() {
        let Ok(resp) = serde_json::from_value::<RpcResponse>(val) else {
            return;
        };
        if let Some(id) = resp.id
            && let Some(reply_tx) = pending_callbacks
                .lock()
                .expect("pending_callbacks mutex poisoned")
                .remove(&id)
        {
            let outcome = match (resp.result, resp.error) {
                (_, Some(err)) => Err(format!("code {}: {}", err.code, err.message)),
                (Some(res), None) => Ok(res),
                (None, None) => Ok(Value::Null),
            };
            let _ = reply_tx.send(outcome);
        }
        return;
    }

    // Case 2: Request from Core to Supervisor.
    let Ok(req) = serde_json::from_value::<RpcRequest>(val) else {
        return;
    };

    let mgr = Arc::clone(manager);
    let tx = out_tx.clone();
    tokio::spawn(async move {
        let resp = dispatch_core_request(req, mgr).await;
        if let Ok(encoded) = serde_json::to_string(&resp) {
            let _ = tx.send(encoded);
        }
    });
}

async fn dispatch_core_request(
    req: RpcRequest,
    manager: Arc<AsyncMutex<SupervisorManager>>,
) -> RpcResponse {
    let id = req.id;
    let params = req.params.unwrap_or(Value::Null);

    match req.method.as_str() {
        "supervisor.list_skills" => {
            let summaries = manager.lock().await.list_summaries();
            match serde_json::to_value(summaries) {
                Ok(v) => RpcResponse::ok(id, v),
                Err(e) => RpcResponse::err(Some(id), ERR_INTERNAL_ERROR, e.to_string()),
            }
        }
        "supervisor.rescan" => {
            let mut guard = manager.lock().await;
            let errs: Vec<String> = guard.scan().into_iter().map(|e| e.to_string()).collect();
            let summaries = guard.list_summaries();
            RpcResponse::ok(id, json!({ "skills": summaries, "errors": errs }))
        }
        "supervisor.start_skill" => {
            let parsed: SupervisorStartSkillParams = match serde_json::from_value(params) {
                Ok(p) => p,
                Err(e) => return RpcResponse::err(Some(id), ERR_INVALID_PARAMS, e.to_string()),
            };
            let res = manager
                .lock()
                .await
                .start_device_skill(&parsed.skill_id, parsed.config, parsed.secrets)
                .await;
            match res {
                Ok(()) => RpcResponse::ok(id, json!({ "ok": true })),
                Err(e) => RpcResponse::err(Some(id), ERR_INTERNAL_ERROR, e.to_string()),
            }
        }
        "supervisor.stop_skill" => {
            let parsed: SupervisorSkillIdParams = match serde_json::from_value(params) {
                Ok(p) => p,
                Err(e) => return RpcResponse::err(Some(id), ERR_INVALID_PARAMS, e.to_string()),
            };
            let res = manager.lock().await.stop_skill(&parsed.skill_id).await;
            match res {
                Ok(()) => RpcResponse::ok(id, json!({ "ok": true })),
                Err(e) => RpcResponse::err(Some(id), ERR_INTERNAL_ERROR, e.to_string()),
            }
        }
        "supervisor.reinstall_skill" => {
            let parsed: SupervisorSkillIdParams = match serde_json::from_value(params) {
                Ok(p) => p,
                Err(e) => return RpcResponse::err(Some(id), ERR_INVALID_PARAMS, e.to_string()),
            };
            let prepared = manager.lock().await.prepare_reinstall(&parsed.skill_id);
            match prepared {
                Ok(task) => match task.execute().await {
                    Ok(()) => RpcResponse::ok(id, json!({ "ok": true })),
                    Err(e) => RpcResponse::err(Some(id), ERR_INTERNAL_ERROR, e.to_string()),
                },
                Err(e) => RpcResponse::err(Some(id), ERR_INTERNAL_ERROR, e.to_string()),
            }
        }
        "supervisor.get_logs" => {
            let parsed: SupervisorGetLogsParams = match serde_json::from_value(params) {
                Ok(p) => p,
                Err(e) => return RpcResponse::err(Some(id), ERR_INVALID_PARAMS, e.to_string()),
            };
            let res = manager.lock().await.get_logs(&parsed.skill_id, parsed.tail);
            match res {
                Ok(logs) => match serde_json::to_value(logs) {
                    Ok(v) => RpcResponse::ok(id, v),
                    Err(e) => RpcResponse::err(Some(id), ERR_INTERNAL_ERROR, e.to_string()),
                },
                Err(e) => RpcResponse::err(Some(id), ERR_INTERNAL_ERROR, e.to_string()),
            }
        }
        "supervisor.entity_command" => {
            let parsed: SupervisorEntityCommandParams = match serde_json::from_value(params) {
                Ok(p) => p,
                Err(e) => return RpcResponse::err(Some(id), ERR_INVALID_PARAMS, e.to_string()),
            };
            let prepared = manager
                .lock()
                .await
                .prepare_entity_command(&parsed.skill_id);
            match prepared {
                Ok(cmd) => match cmd.execute(parsed.command).await {
                    Ok(res) => RpcResponse::ok(id, res),
                    Err(e) => RpcResponse::err(Some(id), ERR_INTERNAL_ERROR, e.to_string()),
                },
                Err(e) => RpcResponse::err(Some(id), ERR_INTERNAL_ERROR, e.to_string()),
            }
        }
        "supervisor.invoke_skill" => {
            let parsed: SupervisorInvokeSkillParams = match serde_json::from_value(params) {
                Ok(p) => p,
                Err(e) => return RpcResponse::err(Some(id), ERR_INVALID_PARAMS, e.to_string()),
            };
            let prepared = manager.lock().await.prepare_action_invoke(&parsed.skill_id);
            match prepared {
                Ok(task) => match task
                    .execute(parsed.config, parsed.secrets, parsed.invoke)
                    .await
                {
                    Ok(res) => RpcResponse::ok(id, res),
                    Err(e) => RpcResponse::err(Some(id), ERR_INTERNAL_ERROR, e.to_string()),
                },
                Err(e) => RpcResponse::err(Some(id), ERR_INTERNAL_ERROR, e.to_string()),
            }
        }
        other => RpcResponse::err(
            Some(id),
            ERR_METHOD_NOT_FOUND,
            format!("unknown supervisor method '{other}'"),
        ),
    }
}
