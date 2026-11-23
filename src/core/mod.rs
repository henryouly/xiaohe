use std::env;

use thiserror::Error;
use tokio::net::TcpListener;
use tracing::info;

use crate::config::{ConfigError, ConfigStore};
use crate::http::{self, AppState};
use crate::protocol::rpc::PROTOCOL_VERSION;
use crate::storage::{StorageError, build_history_store};

pub const DEFAULT_CONFIG_DIR: &str = "/config";

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("configuration error: {0}")]
    Config(#[from] ConfigError),
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),
    #[error("failed to bind HTTP listener on {addr}: {source}")]
    Bind {
        addr: String,
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

    let bind_addr = format!("{}:{}", system.server.host, system.server.port);
    let listener = TcpListener::bind(&bind_addr)
        .await
        .map_err(|source| CoreError::Bind {
            addr: bind_addr.clone(),
            source,
        })?;

    info!(
        version = env!("CARGO_PKG_VERSION"),
        protocol_version = PROTOCOL_VERSION,
        config_dir = %config_dir,
        bind = %bind_addr,
        storage_backend = ?system.storage.backend,
        runtime_socket = %system.runtime.socket_path,
        "xiaohe core listening"
    );

    let router = http::build_router(AppState { history });
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    info!("xiaohe core shut down cleanly");
    Ok(())
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
