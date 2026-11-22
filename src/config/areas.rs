use serde::{Deserialize, Serialize};

/// Schema for `config/areas.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AreasConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub areas: Vec<AreaConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AreaConfig {
    pub area_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}
