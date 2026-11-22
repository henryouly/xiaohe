use std::env;

use tracing::info;

use crate::protocol::rpc::PROTOCOL_VERSION;

pub const DEFAULT_RUNTIME_SOCKET: &str = "/run/xiaohe/rpc.sock";
pub const DEFAULT_SKILLS_DIR: &str = "/skills";

/// Entrypoint for `xiaohe supervisor`.
pub async fn run() {
    let socket = env::var("RUNTIME_SOCKET").unwrap_or_else(|_| DEFAULT_RUNTIME_SOCKET.to_owned());
    let skills_dir = env::var("SKILLS_DIR").unwrap_or_else(|_| DEFAULT_SKILLS_DIR.to_owned());

    info!(
        version = env!("CARGO_PKG_VERSION"),
        protocol_version = PROTOCOL_VERSION,
        socket = %socket,
        skills_dir = %skills_dir,
        "xiaohe supervisor initialized (scaffold)"
    );
}
