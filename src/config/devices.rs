use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Schema for `config/devices.toml`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DevicesConfig {
    #[serde(default)]
    pub esphome: EspHomeConfig,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub skills: BTreeMap<String, SkillUserConfig>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EspHomeConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<EspHomeNodeConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EspHomeNodeConfig {
    pub id: String,
    pub host: String,
    #[serde(default = "default_esphome_port")]
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub noise_psk_secret: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area_id: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillUserConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub config: BTreeMap<String, toml::Value>,
}

impl Default for SkillUserConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            config: BTreeMap::new(),
        }
    }
}

fn default_esphome_port() -> u16 {
    6053
}

fn default_true() -> bool {
    true
}
