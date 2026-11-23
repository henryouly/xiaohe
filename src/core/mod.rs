use std::env;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use thiserror::Error;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::config::{ConfigError, ConfigStore, StorageBackend};
use crate::http::{self, AppState};
use crate::protocol::rpc::PROTOCOL_VERSION;
use crate::storage::{HistoryStore, StorageError, build_history_store};

pub const DEFAULT_CONFIG_DIR: &str = "/config";
const PRUNE_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const MS_PER_DAY: i64 = 86_400_000;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("configuration error: {0}")]
    Config(#[from] ConfigError),
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),
    #[error("failed to bind HTTP listener on {host}:{port}: {source}")]
    Bind {
        host: String,
        port: u16,
        #[source]
        source: std::io::Error,
    },
    #[error("HTTP server error: {0}")]
    Serve(#[from] std::io::Error),
}

/// Entrypoint for `xiaohe core`.
pub async fn run() -> Result<(), CoreError> {
    let config_dir = env::var("CONFIG_DIR").unwrap_or_else(|_| DEFAULT_CONFIG_DIR.to_owned());
    let config_store = ConfigStore::new(&config_dir);

    let mut system = config_store.load_system()?;
    system.apply_env_overrides();

    let history = build_history_store(&system.storage).await?;

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let retention_days = match system.storage.backend {
        StorageBackend::Sqlite => system.storage.sqlite.retention_days,
        StorageBackend::InfluxDb => 0,
    };
    let pruner_handle = spawn_retention_pruner(Arc::clone(&history), retention_days, shutdown_rx);

    let listener = TcpListener::bind((system.server.host.as_str(), system.server.port))
        .await
        .map_err(|source| CoreError::Bind {
            host: system.server.host.clone(),
            port: system.server.port,
            source,
        })?;
    let local_addr = listener.local_addr().map_err(|source| CoreError::Bind {
        host: system.server.host.clone(),
        port: system.server.port,
        source,
    })?;

    info!(
        version = env!("CARGO_PKG_VERSION"),
        protocol_version = PROTOCOL_VERSION,
        config_dir = %config_dir,
        bind = %local_addr,
        storage_backend = ?system.storage.backend,
        retention_days = retention_days,
        runtime_socket = %system.runtime.socket_path,
        "xiaohe core listening"
    );

    let router = http::build_router(AppState { history });
    let serve_result = axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            let _ = shutdown_tx.send(true);
        })
        .await;

    if let Some(handle) = pruner_handle
        && let Err(err) = handle.await
    {
        warn!(error = %err, "retention pruner task exited abnormally");
    }

    serve_result?;
    info!("xiaohe core shut down cleanly");
    Ok(())
}

fn spawn_retention_pruner(
    history: Arc<dyn HistoryStore>,
    retention_days: u32,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Option<JoinHandle<()>> {
    if retention_days == 0 {
        return None;
    }

    Some(tokio::spawn(async move {
        let mut interval = tokio::time::interval(PRUNE_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    prune_expired(&*history, retention_days).await;
                }
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        break;
                    }
                }
            }
        }
    }))
}

async fn prune_expired(history: &dyn HistoryStore, retention_days: u32) {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let cutoff_ms = now_ms.saturating_sub(i64::from(retention_days).saturating_mul(MS_PER_DAY));
    match history.prune_before(cutoff_ms).await {
        Ok(pruned) if pruned > 0 => {
            info!(
                pruned_rows = pruned,
                retention_days, "pruned expired history"
            );
        }
        Ok(_) => {}
        Err(err) => {
            warn!(error = %err, retention_days, "failed to prune expired history");
        }
    }
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
        () = ctrl_c => info!("received SIGINT, initiating graceful shutdown"),
        () = terminate => info!("received SIGTERM, initiating graceful shutdown"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{HistoryEntryKind, SqliteStore, StateHistoryEntry, TimeRange};

    #[tokio::test]
    async fn retention_pruner_removes_expired_rows_and_shuts_down_cleanly() {
        let store = Arc::new(SqliteStore::open(":memory:").await.expect("sqlite"));
        store
            .record_state_batch(&[StateHistoryEntry {
                entity_id: "sensor.old".into(),
                ts: 1_000, // epoch + 1s (far older than 1 day)
                kind: HistoryEntryKind::Numeric {
                    value: 10.0,
                    unit: Some("°C".into()),
                },
            }])
            .await
            .expect("record old sample");

        let (tx, rx) = watch::channel(false);
        let handle = spawn_retention_pruner(Arc::clone(&store) as Arc<dyn HistoryStore>, 1, rx)
            .expect("pruner spawned");

        // Allow initial tick to run, then signal graceful shutdown.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let _ = tx.send(true);
        handle.await.expect("pruner joined cleanly");

        let series = store
            .query_entity_history("sensor.old", TimeRange::new(0, 10_000).unwrap(), None)
            .await
            .expect("query after prune");
        match series {
            crate::storage::EntityHistorySeries::Discrete { transitions, .. } => {
                assert!(transitions.is_empty());
            }
            crate::storage::EntityHistorySeries::Numeric { points, .. } => {
                assert!(points.is_empty());
            }
        }
    }
}
