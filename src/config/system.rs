use std::env;

use serde::{Deserialize, Serialize};

/// Top-level schema for `config/system.toml`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SystemConfig {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub location: LocationConfig,
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub runtime: RuntimeConfig,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub webhooks: Vec<WebhookConfig>,
}

impl SystemConfig {
    /// Apply environment variable overrides onto the loaded config.
    pub fn apply_env_overrides(&mut self) {
        self.apply_env_overrides_with(|key| env::var(key).ok());
    }

    pub(crate) fn apply_env_overrides_with<F>(&mut self, get_env: F)
    where
        F: Fn(&str) -> Option<String>,
    {
        if let Some(host) = get_env("HOST").filter(|v| !v.is_empty()) {
            self.server.host = host;
        }
        if let Some(port) = get_env("PORT").and_then(|v| v.parse::<u16>().ok()) {
            self.server.port = port;
        }
        if let Some(backend) = get_env("STORAGE_BACKEND") {
            match backend.to_ascii_lowercase().as_str() {
                "sqlite" => self.storage.backend = StorageBackend::Sqlite,
                "influxdb" => self.storage.backend = StorageBackend::InfluxDb,
                _ => {}
            }
        }
        if let Some(sqlite_path) = get_env("SQLITE_PATH").filter(|v| !v.is_empty()) {
            self.storage.sqlite.path = sqlite_path;
        }
        if let Some(socket) = get_env("RUNTIME_SOCKET").filter(|v| !v.is_empty()) {
            self.runtime.socket_path = socket;
        }
        let influx_url = get_env("INFLUXDB_URL").filter(|v| !v.is_empty());
        let influx_org = get_env("INFLUXDB_ORG").filter(|v| !v.is_empty());
        let influx_bucket = get_env("INFLUXDB_BUCKET").filter(|v| !v.is_empty());
        if influx_url.is_some() || influx_org.is_some() || influx_bucket.is_some() {
            let cfg = self.storage.influxdb.get_or_insert_with(|| InfluxDbConfig {
                url: "http://influxdb:8086".to_owned(),
                database: None,
                org: Some("xiaohe".to_owned()),
                bucket: Some("xiaohe".to_owned()),
                token_secret: None,
            });
            if let Some(url) = influx_url {
                cfg.url = url;
            }
            if let Some(org) = influx_org {
                cfg.org = Some(org);
            }
            if let Some(bucket) = influx_bucket {
                cfg.bucket = Some(bucket);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_url: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            public_url: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocationConfig {
    #[serde(default = "default_timezone")]
    pub timezone: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latitude: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub longitude: Option<f64>,
}

impl Default for LocationConfig {
    fn default() -> Self {
        Self {
            timezone: default_timezone(),
            latitude: None,
            longitude: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageBackend {
    #[default]
    Sqlite,
    #[serde(rename = "influxdb")]
    InfluxDb,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageConfig {
    #[serde(default)]
    pub backend: StorageBackend,
    #[serde(default)]
    pub sqlite: SqliteConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub influxdb: Option<InfluxDbConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SqliteConfig {
    #[serde(default = "default_sqlite_path")]
    pub path: String,
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
}

impl Default for SqliteConfig {
    fn default() -> Self {
        Self {
            path: default_sqlite_path(),
            retention_days: default_retention_days(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InfluxDbConfig {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bucket: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_secret: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeConfig {
    #[serde(default = "default_runtime_socket")]
    pub socket_path: String,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            socket_path: default_runtime_socket(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebhookConfig {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub secret_hash: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_host() -> String {
    "0.0.0.0".to_owned()
}

fn default_port() -> u16 {
    4000
}

fn default_timezone() -> String {
    "UTC".to_owned()
}

fn default_sqlite_path() -> String {
    "/data/xiaohe.db".to_owned()
}

fn default_retention_days() -> u32 {
    90
}

fn default_runtime_socket() -> String {
    "/run/xiaohe/rpc.sock".to_owned()
}

fn default_true() -> bool {
    true
}
