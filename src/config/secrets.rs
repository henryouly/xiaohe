use std::collections::BTreeMap;
use std::env;
use std::fmt;

use serde::{Deserialize, Serialize};

/// Schema for `config/secrets.toml`.
///
/// Stored on disk with `0600` permissions (Unix) and ignored by Git.
/// Values formatted as `${ENV_VAR}` are resolved from the process environment at read time.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretsConfig {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    secrets: BTreeMap<String, String>,
}

impl fmt::Debug for SecretsConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let keys: Vec<&str> = self.secrets.keys().map(String::as_str).collect();
        f.debug_struct("SecretsConfig")
            .field("keys", &keys)
            .finish()
    }
}

impl SecretsConfig {
    /// Resolve a secret by key, expanding `${ENV_VAR}` references from the process environment.
    pub fn get(&self, key: &str) -> Option<String> {
        self.get_with_env(key, |name| env::var(name).ok())
    }

    pub(crate) fn get_with_env<F>(&self, key: &str, get_env: F) -> Option<String>
    where
        F: Fn(&str) -> Option<String>,
    {
        let raw = self.secrets.get(key)?;
        if let Some(var_name) = parse_env_ref(raw) {
            get_env(var_name)
        } else {
            Some(raw.clone())
        }
    }

    /// Insert or update a secret value.
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.secrets.insert(key.into(), value.into());
    }

    /// Remove a secret by key.
    pub fn remove(&mut self, key: &str) -> Option<String> {
        self.secrets.remove(key)
    }

    /// Check whether a secret key is configured (without exposing its value).
    pub fn contains_key(&self, key: &str) -> bool {
        self.secrets.contains_key(key)
    }

    /// List configured secret keys (safe for masked Admin UI display).
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.secrets.keys().map(String::as_str)
    }

    /// Number of configured secrets.
    pub fn len(&self) -> usize {
        self.secrets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.secrets.is_empty()
    }
}

fn parse_env_ref(value: &str) -> Option<&str> {
    let inner = value.strip_prefix("${")?.strip_suffix('}')?;
    if !inner.is_empty()
        && inner
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        Some(inner)
    } else {
        None
    }
}
