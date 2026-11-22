use tracing::info;

use crate::protocol::rpc::PROTOCOL_VERSION;

/// Entrypoint for `xiaohe core`.
pub async fn run() {
    info!(
        version = env!("CARGO_PKG_VERSION"),
        protocol_version = PROTOCOL_VERSION,
        "xiaohe core initialized (scaffold)"
    );
}
