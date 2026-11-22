pub mod areas;
pub mod devices;
pub mod secrets;
pub mod system;
pub mod users;

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;
use tempfile::NamedTempFile;
use thiserror::Error;

pub use areas::{AreaConfig, AreasConfig};
pub use devices::{DevicesConfig, EspHomeConfig, EspHomeNodeConfig, SkillUserConfig};
pub use secrets::SecretsConfig;
pub use system::{
    InfluxDbConfig, LocationConfig, RuntimeConfig, ServerConfig, SqliteConfig, StorageBackend,
    StorageConfig, SystemConfig, WebhookConfig,
};
pub use users::{AllowedAreas, ApiTokenConfig, UserConfig, UserRole, UsersConfig};

pub const SYSTEM_FILE: &str = "system.toml";
pub const AREAS_FILE: &str = "areas.toml";
pub const USERS_FILE: &str = "users.toml";
pub const DEVICES_FILE: &str = "devices.toml";
pub const SECRETS_FILE: &str = "secrets.toml";

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("I/O error for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse TOML in {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("failed to serialize TOML for {path}: {source}")]
    Serialize {
        path: PathBuf,
        #[source]
        source: toml::ser::Error,
    },
    #[error("failed to atomically persist {path}: {source}")]
    Persist {
        path: PathBuf,
        #[source]
        source: tempfile::PersistError,
    },
}

/// Manages reading and atomically writing Xiaohe's TOML configuration files.
#[derive(Debug, Clone)]
pub struct ConfigStore {
    config_dir: PathBuf,
}

impl ConfigStore {
    pub fn new(config_dir: impl Into<PathBuf>) -> Self {
        Self {
            config_dir: config_dir.into(),
        }
    }

    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    pub fn load_system(&self) -> Result<SystemConfig, ConfigError> {
        self.load_or_default(SYSTEM_FILE)
    }

    pub fn save_system(&self, cfg: &SystemConfig) -> Result<(), ConfigError> {
        self.save_toml(SYSTEM_FILE, cfg, false)
    }

    pub fn load_areas(&self) -> Result<AreasConfig, ConfigError> {
        self.load_or_default(AREAS_FILE)
    }

    pub fn save_areas(&self, cfg: &AreasConfig) -> Result<(), ConfigError> {
        self.save_toml(AREAS_FILE, cfg, false)
    }

    pub fn load_users(&self) -> Result<UsersConfig, ConfigError> {
        self.load_or_default(USERS_FILE)
    }

    pub fn save_users(&self, cfg: &UsersConfig) -> Result<(), ConfigError> {
        self.save_toml(USERS_FILE, cfg, true)
    }

    pub fn load_devices(&self) -> Result<DevicesConfig, ConfigError> {
        self.load_or_default(DEVICES_FILE)
    }

    pub fn save_devices(&self, cfg: &DevicesConfig) -> Result<(), ConfigError> {
        self.save_toml(DEVICES_FILE, cfg, false)
    }

    pub fn load_secrets(&self) -> Result<SecretsConfig, ConfigError> {
        self.load_or_default(SECRETS_FILE)
    }

    pub fn save_secrets(&self, cfg: &SecretsConfig) -> Result<(), ConfigError> {
        self.save_toml(SECRETS_FILE, cfg, true)
    }

    fn load_or_default<T: DeserializeOwned + Default>(
        &self,
        filename: &str,
    ) -> Result<T, ConfigError> {
        let path = self.config_dir.join(filename);
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(T::default()),
            Err(source) => return Err(ConfigError::Io { path, source }),
        };

        toml::from_str(&content).map_err(|source| ConfigError::Parse { path, source })
    }

    fn save_toml<T: Serialize>(
        &self,
        filename: &str,
        value: &T,
        restrict_mode: bool,
    ) -> Result<(), ConfigError> {
        let path = self.config_dir.join(filename);
        let serialized =
            toml::to_string_pretty(value).map_err(|source| ConfigError::Serialize {
                path: path.clone(),
                source,
            })?;
        atomic_write(&path, &serialized, restrict_mode)
    }
}

/// Atomically writes `contents` to `path` via a same-directory temp file + `sync_all` + `rename`.
///
/// When `restrict_mode` is `true` on Unix, the file is created with `0600` permissions before
/// replacing the destination.
pub fn atomic_write(path: &Path, contents: &str, restrict_mode: bool) -> Result<(), ConfigError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| ConfigError::Io {
        path: parent.to_path_buf(),
        source,
    })?;

    let mut temp = NamedTempFile::new_in(parent).map_err(|source| ConfigError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    #[cfg(unix)]
    if restrict_mode {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o600);
        temp.as_file()
            .set_permissions(perms)
            .map_err(|source| ConfigError::Io {
                path: path.to_path_buf(),
                source,
            })?;
    }

    #[cfg(not(unix))]
    let _ = restrict_mode;

    temp.write_all(contents.as_bytes())
        .map_err(|source| ConfigError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    temp.as_file()
        .sync_all()
        .map_err(|source| ConfigError::Io {
            path: path.to_path_buf(),
            source,
        })?;

    temp.persist(path).map_err(|source| ConfigError::Persist {
        path: path.to_path_buf(),
        source,
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn missing_files_return_defaults() {
        let dir = TempDir::new().expect("tempdir");
        let store = ConfigStore::new(dir.path());

        let system = store.load_system().expect("default system");
        assert_eq!(system.server.host, "0.0.0.0");
        assert_eq!(system.server.port, 4000);
        assert_eq!(system.storage.backend, StorageBackend::Sqlite);
        assert_eq!(system.storage.sqlite.path, "/data/xiaohe.db");
        assert_eq!(system.runtime.socket_path, "/run/xiaohe/rpc.sock");

        assert!(store.load_areas().expect("default areas").areas.is_empty());
        assert!(store.load_users().expect("default users").users.is_empty());
        assert!(
            store
                .load_devices()
                .expect("default devices")
                .esphome
                .nodes
                .is_empty()
        );
        assert!(store.load_secrets().expect("default secrets").is_empty());
    }

    #[test]
    fn all_config_files_round_trip_atomically() {
        let dir = TempDir::new().expect("tempdir");
        let store = ConfigStore::new(dir.path());

        // 1. system.toml
        let system = SystemConfig {
            server: ServerConfig {
                host: "127.0.0.1".into(),
                port: 8080,
                public_url: Some("https://home.example.com".into()),
            },
            location: LocationConfig {
                timezone: "America/Los_Angeles".into(),
                latitude: Some(37.7749),
                longitude: Some(-122.4194),
            },
            storage: StorageConfig {
                backend: StorageBackend::Sqlite,
                sqlite: SqliteConfig {
                    path: "/data/custom.db".into(),
                    retention_days: 30,
                },
                influxdb: None,
            },
            runtime: RuntimeConfig {
                socket_path: "/run/xiaohe/custom.sock".into(),
            },
            webhooks: vec![WebhookConfig {
                id: "ios_arriving_home".into(),
                name: Some("iOS Shortcuts".into()),
                secret_hash: "sha256:abcdef".into(),
                enabled: true,
            }],
        };
        store.save_system(&system).expect("save system");
        assert_eq!(store.load_system().expect("load system"), system);

        // 2. areas.toml
        let areas = AreasConfig {
            areas: vec![
                AreaConfig {
                    area_id: "living_room".into(),
                    name: "Living Room".into(),
                    icon: Some("sofa".into()),
                },
                AreaConfig {
                    area_id: "kitchen".into(),
                    name: "Kitchen".into(),
                    icon: None,
                },
            ],
        };
        store.save_areas(&areas).expect("save areas");
        assert_eq!(store.load_areas().expect("load areas"), areas);

        // 3. users.toml (verifies both "*" and list AllowedAreas)
        let users = UsersConfig {
            users: vec![
                UserConfig {
                    id: "u_admin".into(),
                    username: "admin".into(),
                    display_name: "Admin".into(),
                    password_hash: "$argon2id$v=19$m=19456,t=2,p=1$salt$hash".into(),
                    role: UserRole::Admin,
                    allowed_areas: AllowedAreas::All,
                    allow_guest_locks: false,
                    api_tokens: vec![ApiTokenConfig {
                        id: "tok_1".into(),
                        name: "CLI".into(),
                        token_hash: "sha256:123456".into(),
                        created_at: 1_700_000_000_000,
                    }],
                },
                UserConfig {
                    id: "u_guest".into(),
                    username: "guest".into(),
                    display_name: "Guest".into(),
                    password_hash: "$argon2id$...".into(),
                    role: UserRole::Guest,
                    allowed_areas: AllowedAreas::List(vec!["living_room".into()]),
                    allow_guest_locks: false,
                    api_tokens: vec![],
                },
            ],
        };
        store.save_users(&users).expect("save users");
        assert_eq!(store.load_users().expect("load users"), users);

        // 4. devices.toml
        let mut skill_cfg = BTreeMap::new();
        skill_cfg.insert("host".into(), toml::Value::String("192.168.1.42".into()));
        skill_cfg.insert("poll_interval_secs".into(), toml::Value::Integer(10));
        let mut skills = BTreeMap::new();
        skills.insert(
            "miio-airpurifier".into(),
            SkillUserConfig {
                enabled: true,
                config: skill_cfg,
            },
        );
        let devices = DevicesConfig {
            esphome: EspHomeConfig {
                nodes: vec![EspHomeNodeConfig {
                    id: "esp32_living".into(),
                    host: "192.168.1.50".into(),
                    port: 6053,
                    noise_psk_secret: Some("esphome_living_psk".into()),
                    area_id: Some("living_room".into()),
                    enabled: true,
                }],
            },
            skills,
        };
        store.save_devices(&devices).expect("save devices");
        assert_eq!(store.load_devices().expect("load devices"), devices);

        // 5. secrets.toml
        let mut secrets = SecretsConfig::default();
        secrets.set("miio_token", "0123456789abcdef0123456789abcdef");
        secrets.set("influx_token", "${TEST_INFLUX_TOKEN}");
        store.save_secrets(&secrets).expect("save secrets");
        let loaded_secrets = store.load_secrets().expect("load secrets");
        assert_eq!(loaded_secrets, secrets);
        assert_eq!(
            loaded_secrets.get("miio_token").as_deref(),
            Some("0123456789abcdef0123456789abcdef")
        );
        assert_eq!(
            loaded_secrets
                .get_with_env("influx_token", |k| {
                    (k == "TEST_INFLUX_TOKEN").then(|| "env-secret-value".to_owned())
                })
                .as_deref(),
            Some("env-secret-value")
        );

        // Ensure no stray temporary files were left behind in config_dir.
        let mut filenames: Vec<String> = fs::read_dir(dir.path())
            .expect("read_dir")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        filenames.sort();
        assert_eq!(
            filenames,
            vec![
                "areas.toml",
                "devices.toml",
                "secrets.toml",
                "system.toml",
                "users.toml"
            ]
        );
    }

    #[test]
    fn secrets_debug_is_redacted() {
        let mut secrets = SecretsConfig::default();
        secrets.set("miio_token", "super-secret-token-value");
        let debug_output = format!("{secrets:?}");
        assert!(debug_output.contains("miio_token"));
        assert!(!debug_output.contains("super-secret-token-value"));
    }

    #[cfg(unix)]
    #[test]
    fn secrets_and_users_files_have_0600_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().expect("tempdir");
        let store = ConfigStore::new(dir.path());

        let mut secrets = SecretsConfig::default();
        secrets.set("api_key", "secret");
        store.save_secrets(&secrets).expect("save secrets");
        store
            .save_users(&UsersConfig::default())
            .expect("save users");

        let secrets_mode = fs::metadata(dir.path().join(SECRETS_FILE))
            .expect("secrets meta")
            .permissions()
            .mode()
            & 0o777;
        let users_mode = fs::metadata(dir.path().join(USERS_FILE))
            .expect("users meta")
            .permissions()
            .mode()
            & 0o777;

        assert_eq!(secrets_mode, 0o600);
        assert_eq!(users_mode, 0o600);
    }

    #[test]
    fn malformed_toml_returns_parse_error() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(dir.path().join(SYSTEM_FILE), "not valid = [toml").expect("write bad toml");
        let store = ConfigStore::new(dir.path());

        let err = store.load_system().expect_err("expected parse error");
        assert!(matches!(err, ConfigError::Parse { .. }));
    }

    #[test]
    fn system_env_overrides_apply_cleanly() {
        let mut cfg = SystemConfig::default();
        cfg.apply_env_overrides_with(|key| match key {
            "HOST" => Some("127.0.0.1".into()),
            "PORT" => Some("4567".into()),
            "STORAGE_BACKEND" => Some("influxdb".into()),
            "SQLITE_PATH" => Some("/tmp/test.db".into()),
            "RUNTIME_SOCKET" => Some("/tmp/rpc.sock".into()),
            _ => None,
        });

        assert_eq!(cfg.server.host, "127.0.0.1");
        assert_eq!(cfg.server.port, 4567);
        assert_eq!(cfg.storage.backend, StorageBackend::InfluxDb);
        assert_eq!(cfg.storage.sqlite.path, "/tmp/test.db");
        assert_eq!(cfg.runtime.socket_path, "/tmp/rpc.sock");
    }
}
