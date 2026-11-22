use std::env;

use tracing::info;

use crate::config::{ConfigError, ConfigStore};
use crate::protocol::rpc::PROTOCOL_VERSION;

pub const DEFAULT_CONFIG_DIR: &str = "/config";

/// Entrypoint for `xiaohe core`.
pub async fn run() -> Result<(), ConfigError> {
    let config_dir = env::var("CONFIG_DIR").unwrap_or_else(|_| DEFAULT_CONFIG_DIR.to_owned());
    let store = ConfigStore::new(&config_dir);

    let mut system = store.load_system()?;
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

    Ok(())
}
