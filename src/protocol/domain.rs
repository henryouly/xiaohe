use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Strongly typed domain state for all smart home entities.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "domain", rename_all = "snake_case")]
pub enum DomainState {
    Light {
        on: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        brightness: Option<u8>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color_temp_k: Option<u16>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rgb: Option<[u8; 3]>,
    },
    Switch {
        on: bool,
    },
    Climate {
        mode: ClimateMode,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_temp_c: Option<f32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        current_temp_c: Option<f32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fan_mode: Option<String>,
    },
    Sensor {
        value: SensorValue,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        device_class: Option<String>,
    },
    BinarySensor {
        active: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        device_class: Option<String>,
    },
    Cover {
        state: CoverState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        position: Option<u8>,
    },
    Lock {
        state: LockState,
    },
    MediaPlayer {
        state: MediaState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        volume: Option<u8>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        muted: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
    Button {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last_pressed_at: Option<i64>,
    },
    Custom {
        state: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClimateMode {
    Off,
    Heat,
    Cool,
    Auto,
    Dry,
    FanOnly,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SensorValue {
    Number(f64),
    Text(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverState {
    Open,
    Opening,
    Closed,
    Closing,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LockState {
    Locked,
    Unlocked,
    Jammed,
    Locking,
    Unlocking,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaState {
    Idle,
    Playing,
    Paused,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    Online,
    Offline,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EntitySource {
    #[serde(rename = "esphome")]
    EspHome {
        node_id: String,
    },
    Skill {
        skill_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomActionDef {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub params: Map<String, Value>,
}

/// Complete entity state record stored in `StateStore` and served over REST/SSE.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityRecord {
    pub entity_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area_id: Option<String>,
    pub name: String,
    pub source: EntitySource,
    pub availability: Availability,
    pub state: DomainState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_actions: Vec<CustomActionDef>,
    pub last_changed_at: i64,
    pub last_updated_at: i64,
}
