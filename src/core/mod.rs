use std::env;

use tracing::{error, info};

use crate::config::ConfigStore;
use crate::protocol::rpc::PROTOCOL_VERSION;

pub const DEFAULT_CONFIG_DIR: &str = "/config";

/// Entrypoint for `xiaohe core`.
pub async fn run() {
    let config_dir = env::var("CONFIG_DIR").unwrap_or_else(|_| DEFAULT_CONFIG_DIR.to_owned());
    let store = ConfigStore::new(&config_dir);

    let mut system = match store.load_system() {
        Ok(cfg) => cfg,
        Err(err) => {
            error!(config_dir = %config_dir, error = %err, "failed to load system.toml");
            return;
        }
    };
    system.apply_env_overrides();

    info!(
        version = env!("CARGO_PKG_VERSION"),
        protocol_version = PROTOCOL_VERSION,
        config_dir = %config_dir,
        bind = %format!("{}:{}", system.server.host, system.server.port),
        storage_backend = ?system.storage.backend,
        runtime_socket = %system.runtime.socket_path,
        "xiaohe core initialized (scaffold)"
    );
}
