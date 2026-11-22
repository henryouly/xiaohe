use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "xiaohe", version, about = "Xiaohe smart home control platform")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Run the Xiaohe core state engine, automation controller, and web server.
    Core,
    /// Run the Xiaohe Skill supervisor inside the xiaohe-runtime container.
    Supervisor,
}

#[tokio::main]
async fn main() {
    init_tracing();

    let cli = Cli::parse();
    match cli.command {
        Commands::Core => xiaohe::core::run().await,
        Commands::Supervisor => xiaohe::supervisor::run().await,
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
