use std::path::{Path, PathBuf};

use thiserror::Error;
use tokio::process::Command;
use tracing::{debug, info};

use crate::protocol::skill::RuntimeType;
use crate::supervisor::manifest::DiscoveredSkill;

#[derive(Debug, Error)]
pub enum EnvError {
    #[error("failed to execute uv for skill '{skill_id}': {source}")]
    Spawn {
        skill_id: String,
        #[source]
        source: std::io::Error,
    },
    #[error("uv provisioning failed for skill '{skill_id}' (exit {code:?}): {stderr}")]
    ProvisionFailed {
        skill_id: String,
        code: Option<i32>,
        stderr: String,
    },
    #[error("entrypoint '{path}' not found for skill '{skill_id}'")]
    MissingEntrypoint { skill_id: String, path: PathBuf },
}

/// Resolved command and arguments ready to spawn for a skill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub working_dir: PathBuf,
    pub venv_dir: Option<PathBuf>,
}

/// Provision (or verify) the isolated runtime environment for `skill` and return the command to
/// execute its entrypoint.
///
/// For `RuntimeType::PythonUv`:
/// - If `pyproject.toml` exists, runs `uv sync --frozen` (when `uv.lock` exists) or `uv sync`.
/// - Otherwise runs `uv venv` to create an isolated `.venv`.
/// - Resolves `python` / `python3` in `runtime.entrypoint[0]` to `<skill_dir>/.venv/bin/python`.
///
/// For `RuntimeType::Executable`:
/// - Resolves relative entrypoint paths against `skill.dir` (or uses `PATH` for bare commands).
pub async fn provision_skill_env(
    skill: &DiscoveredSkill,
    force_reinstall: bool,
) -> Result<ResolvedCommand, EnvError> {
    let skill_id = &skill.manifest.skill.id;

    match skill.manifest.runtime.runtime_type {
        RuntimeType::PythonUv => {
            let venv_dir = skill.dir.join(".venv");
            let venv_python = venv_dir.join("bin").join("python");
            let pyproject = skill.dir.join("pyproject.toml");
            let uv_lock = skill.dir.join("uv.lock");

            if force_reinstall || !venv_python.exists() {
                if pyproject.exists() {
                    let mut cmd = Command::new("uv");
                    cmd.arg("sync").current_dir(&skill.dir);
                    if uv_lock.exists() {
                        cmd.arg("--frozen");
                    }
                    if force_reinstall {
                        cmd.arg("--reinstall");
                    }
                    info!(skill_id = %skill_id, frozen = uv_lock.exists(), "running uv sync for skill");
                    run_uv(cmd, skill_id).await?;
                } else {
                    let mut cmd = Command::new("uv");
                    cmd.arg("venv").arg(&venv_dir).current_dir(&skill.dir);
                    if let Some(py) = &skill.manifest.runtime.python {
                        debug!(skill_id = %skill_id, python = %py, "creating isolated uv venv");
                    }
                    info!(skill_id = %skill_id, "running uv venv for skill");
                    run_uv(cmd, skill_id).await?;
                }
            }

            let entry = &skill.manifest.runtime.entrypoint;
            let program = if entry[0] == "python" || entry[0] == "python3" {
                venv_python
            } else {
                let candidate = venv_dir.join("bin").join(&entry[0]);
                if candidate.exists() {
                    candidate
                } else {
                    PathBuf::from(&entry[0])
                }
            };

            Ok(ResolvedCommand {
                program,
                args: entry[1..].to_vec(),
                working_dir: skill.dir.clone(),
                venv_dir: Some(venv_dir),
            })
        }
        RuntimeType::Executable => {
            let entry = &skill.manifest.runtime.entrypoint;
            let first = Path::new(&entry[0]);
            let program = if first.is_absolute() {
                first.to_path_buf()
            } else if entry[0].contains('/') {
                let rel = skill.dir.join(first);
                if !rel.exists() {
                    return Err(EnvError::MissingEntrypoint {
                        skill_id: skill_id.clone(),
                        path: rel,
                    });
                }
                rel
            } else {
                first.to_path_buf()
            };

            Ok(ResolvedCommand {
                program,
                args: entry[1..].to_vec(),
                working_dir: skill.dir.clone(),
                venv_dir: None,
            })
        }
    }
}

async fn run_uv(mut cmd: Command, skill_id: &str) -> Result<(), EnvError> {
    let output = cmd.output().await.map_err(|source| EnvError::Spawn {
        skill_id: skill_id.to_owned(),
        source,
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(EnvError::ProvisionFailed {
            skill_id: skill_id.to_owned(),
            code: output.status.code(),
            stderr,
        });
    }
    Ok(())
}
