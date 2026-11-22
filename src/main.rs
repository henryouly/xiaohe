use std::process::ExitCode;

use clap::{Parser, Subcommand};
use tracing::error;
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
async fn main() -> ExitCode {
    init_tracing();

    let cli = Cli::parse();
    match cli.command {
        Commands::Core => match xiaohe::core::run().await {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                error!(error = %err, "xiaohe core failed to start");
                ExitCode::FAILURE
            }
        },
        Commands::Supervisor => {
            xiaohe::supervisor::run().await;
            ExitCode::SUCCESS
        }
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
