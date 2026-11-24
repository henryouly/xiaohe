use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::protocol::skill::{RestartPolicy, RuntimeType, SkillMode};

pub const MANIFEST_FILE: &str = "skill.toml";

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("I/O error reading {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("invalid manifest in {path}: {reason}")]
    Validation { path: PathBuf, reason: String },
}

/// Machine-readable manifest parsed from `/skills/<skill_id>/skill.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillManifest {
    pub skill: SkillMeta,
    pub runtime: SkillRuntime,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub config: BTreeMap<String, ConfigFieldSpec>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub secrets: BTreeMap<String, SecretFieldSpec>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub actions: BTreeMap<String, ActionSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillMeta {
    pub id: String,
    pub name: String,
    pub version: String,
    pub mode: SkillMode,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillRuntime {
    #[serde(rename = "type")]
    pub runtime_type: RuntimeType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub python: Option<String>,
    pub entrypoint: Vec<String>,
    #[serde(default = "default_heartbeat_secs")]
    pub heartbeat_interval_secs: u64,
    #[serde(default = "default_invoke_timeout_secs")]
    pub invoke_timeout_secs: u64,
    #[serde(default)]
    pub restart_policy: RestartPolicy,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigFieldSpec {
    #[serde(rename = "type")]
    pub field_type: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretFieldSpec {
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionSpec {
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, ConfigFieldSpec>,
}

/// Discovered skill directory and its validated manifest.
#[derive(Debug, Clone, PartialEq)]
pub struct DiscoveredSkill {
    pub dir: PathBuf,
    pub manifest: SkillManifest,
}

impl SkillManifest {
    /// Load and validate a `skill.toml` file from disk.
    pub fn load(path: &Path) -> Result<Self, ManifestError> {
        let raw = fs::read_to_string(path).map_err(|source| ManifestError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let manifest: Self = toml::from_str(&raw).map_err(|source| ManifestError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
        manifest.validate(path)?;
        Ok(manifest)
    }

    pub fn validate(&self, path: &Path) -> Result<(), ManifestError> {
        let err = |reason: &str| ManifestError::Validation {
            path: path.to_path_buf(),
            reason: reason.to_owned(),
        };

        if self.skill.id.is_empty()
            || !self
                .skill
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        {
            return Err(err("skill.id must be non-empty lowercase ASCII [a-z0-9_-]"));
        }
        if self.skill.name.trim().is_empty() {
            return Err(err("skill.name must not be empty"));
        }
        if self.skill.version.trim().is_empty() {
            return Err(err("skill.version must not be empty"));
        }
        if self.runtime.entrypoint.is_empty() || self.runtime.entrypoint[0].trim().is_empty() {
            return Err(err("runtime.entrypoint must contain at least one element"));
        }
        if self.runtime.heartbeat_interval_secs == 0 {
            return Err(err("runtime.heartbeat_interval_secs must be >= 1"));
        }
        if self.runtime.invoke_timeout_secs == 0 {
            return Err(err("runtime.invoke_timeout_secs must be >= 1"));
        }
        if self.skill.mode == SkillMode::Action && self.actions.is_empty() {
            return Err(err(
                "action-mode skills must declare at least one [actions.<name>]",
            ));
        }
        Ok(())
    }

    /// Return the list of secret keys declared in `[secrets.*]` for scoped injection.
    pub fn declared_secret_keys(&self) -> Vec<String> {
        self.secrets.keys().cloned().collect()
    }
}

/// Scan `skills_dir` for `<skills_dir>/*/skill.toml` manifests.
///
/// Returns `(valid_skills, errors)` so one broken skill manifest never blocks healthy skills.
pub fn scan_skills_dir(skills_dir: &Path) -> (Vec<DiscoveredSkill>, Vec<ManifestError>) {
    let mut discovered = Vec::new();
    let mut errors = Vec::new();

    let entries = match fs::read_dir(skills_dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return (discovered, errors),
        Err(source) => {
            errors.push(ManifestError::Io {
                path: skills_dir.to_path_buf(),
                source,
            });
            return (discovered, errors);
        }
    };

    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();

    let mut seen_ids = std::collections::HashSet::new();
    for dir in dirs {
        let manifest_path = dir.join(MANIFEST_FILE);
        if !manifest_path.exists() {
            continue;
        }
        match SkillManifest::load(&manifest_path) {
            Ok(manifest) => {
                if !seen_ids.insert(manifest.skill.id.clone()) {
                    errors.push(ManifestError::Validation {
                        path: manifest_path,
                        reason: format!("duplicate skill.id '{}'", manifest.skill.id),
                    });
                    continue;
                }
                discovered.push(DiscoveredSkill { dir, manifest });
            }
            Err(err) => errors.push(err),
        }
    }

    (discovered, errors)
}

fn default_heartbeat_secs() -> u64 {
    30
}

fn default_invoke_timeout_secs() -> u64 {
    15
}
