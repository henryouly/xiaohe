use std::path::PathBuf;
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
    /// Validate TOML configuration files, automations, and optional Skill manifests offline.
    Validate {
        /// Path to the config directory (defaults to CONFIG_DIR or /config).
        #[arg(long)]
        config_dir: Option<PathBuf>,
        /// Optional path to the skills directory (defaults to SKILLS_DIR if set).
        #[arg(long)]
        skills_dir: Option<PathBuf>,
        /// Emit machine-readable JSON output.
        #[arg(long)]
        json: bool,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Commands::Core => {
            init_tracing();
            match xiaohe::core::run().await {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    error!(error = %err, "xiaohe core failed to start");
                    ExitCode::FAILURE
                }
            }
        }
        Commands::Supervisor => {
            init_tracing();
            match xiaohe::supervisor::run().await {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    error!(error = %err, "xiaohe supervisor failed to start");
                    ExitCode::FAILURE
                }
            }
        }
        Commands::Validate {
            config_dir,
            skills_dir,
            json,
        } => {
            let (cfg_dir, sk_dir) = xiaohe::validate::resolve_dirs(config_dir, skills_dir);
            let report = xiaohe::validate::validate_workspace(&cfg_dir, sk_dir.as_deref());
            print!("{}", xiaohe::validate::format_report(&report, json));
            if report.is_valid() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
