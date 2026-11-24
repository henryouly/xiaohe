//! Wire-compatible ESPHome Native API Protobuf message definitions.
//!
//! Field tags and message type IDs match ESPHome's official `api.proto` specification.

use prost::Message;

// Control & handshake message type IDs.
pub const MSG_HELLO_REQUEST: u16 = 1;
pub const MSG_HELLO_RESPONSE: u16 = 2;
pub const MSG_CONNECT_REQUEST: u16 = 3;
pub const MSG_CONNECT_RESPONSE: u16 = 4;
pub const MSG_DISCONNECT_REQUEST: u16 = 5;
pub const MSG_DISCONNECT_RESPONSE: u16 = 6;
pub const MSG_PING_REQUEST: u16 = 7;
pub const MSG_PING_RESPONSE: u16 = 8;
pub const MSG_DEVICE_INFO_REQUEST: u16 = 9;
pub const MSG_DEVICE_INFO_RESPONSE: u16 = 10;
pub const MSG_LIST_ENTITIES_REQUEST: u16 = 11;
pub const MSG_LIST_ENTITIES_DONE_RESPONSE: u16 = 19;
pub const MSG_SUBSCRIBE_STATES_REQUEST: u16 = 20;

// Entity discovery message type IDs.
pub const MSG_LIST_ENTITIES_BINARY_SENSOR_RESPONSE: u16 = 12;
pub const MSG_LIST_ENTITIES_COVER_RESPONSE: u16 = 13;
pub const MSG_LIST_ENTITIES_LIGHT_RESPONSE: u16 = 15;
pub const MSG_LIST_ENTITIES_SENSOR_RESPONSE: u16 = 16;
pub const MSG_LIST_ENTITIES_SWITCH_RESPONSE: u16 = 17;
pub const MSG_LIST_ENTITIES_CLIMATE_RESPONSE: u16 = 46;
pub const MSG_LIST_ENTITIES_LOCK_RESPONSE: u16 = 58;
pub const MSG_LIST_ENTITIES_BUTTON_RESPONSE: u16 = 61;

// Entity state response message type IDs.
pub const MSG_BINARY_SENSOR_STATE_RESPONSE: u16 = 21;
pub const MSG_COVER_STATE_RESPONSE: u16 = 22;
pub const MSG_LIGHT_STATE_RESPONSE: u16 = 24;
pub const MSG_SENSOR_STATE_RESPONSE: u16 = 25;
pub const MSG_SWITCH_STATE_RESPONSE: u16 = 26;
pub const MSG_CLIMATE_STATE_RESPONSE: u16 = 47;
pub const MSG_LOCK_STATE_RESPONSE: u16 = 59;

// Entity command request message type IDs.
pub const MSG_COVER_COMMAND_REQUEST: u16 = 30;
pub const MSG_LIGHT_COMMAND_REQUEST: u16 = 32;
pub const MSG_SWITCH_COMMAND_REQUEST: u16 = 33;
pub const MSG_CLIMATE_COMMAND_REQUEST: u16 = 48;
pub const MSG_LOCK_COMMAND_REQUEST: u16 = 60;
pub const MSG_BUTTON_COMMAND_REQUEST: u16 = 62;

// ==================== Control Messages ====================

#[derive(Clone, PartialEq, Message)]
pub struct HelloRequest {
    #[prost(string, tag = "1")]
    pub client_info: String,
    #[prost(uint32, tag = "2")]
    pub api_version_major: u32,
    #[prost(uint32, tag = "3")]
    pub api_version_minor: u32,
}

#[derive(Clone, PartialEq, Message)]
pub struct HelloResponse {
    #[prost(uint32, tag = "1")]
    pub api_version_major: u32,
    #[prost(uint32, tag = "2")]
    pub api_version_minor: u32,
    #[prost(string, tag = "3")]
    pub server_info: String,
    #[prost(string, tag = "4")]
    pub name: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct ConnectRequest {
    #[prost(string, tag = "1")]
    pub password: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct ConnectResponse {
    #[prost(bool, tag = "1")]
    pub invalid_password: bool,
}

#[derive(Clone, PartialEq, Message)]
pub struct DisconnectRequest {}

#[derive(Clone, PartialEq, Message)]
pub struct DisconnectResponse {}

#[derive(Clone, PartialEq, Message)]
pub struct PingRequest {}

#[derive(Clone, PartialEq, Message)]
pub struct PingResponse {}

#[derive(Clone, PartialEq, Message)]
pub struct DeviceInfoRequest {}

#[derive(Clone, PartialEq, Message)]
pub struct DeviceInfoResponse {
    #[prost(bool, tag = "1")]
    pub uses_password: bool,
    #[prost(string, tag = "2")]
    pub name: String,
    #[prost(string, tag = "3")]
    pub mac_address: String,
    #[prost(string, tag = "4")]
    pub esphome_version: String,
    #[prost(string, tag = "6")]
    pub model: String,
    #[prost(string, tag = "12")]
    pub manufacturer: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct ListEntitiesRequest {}

#[derive(Clone, PartialEq, Message)]
pub struct ListEntitiesDoneResponse {}

#[derive(Clone, PartialEq, Message)]
pub struct SubscribeStatesRequest {}

// ==================== Entity Discovery ====================

#[derive(Clone, PartialEq, Message)]
pub struct ListEntitiesBinarySensorResponse {
    #[prost(string, tag = "1")]
    pub object_id: String,
    #[prost(fixed32, tag = "2")]
    pub key: u32,
    #[prost(string, tag = "3")]
    pub name: String,
    #[prost(string, tag = "5")]
    pub device_class: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct ListEntitiesCoverResponse {
    #[prost(string, tag = "1")]
    pub object_id: String,
    #[prost(fixed32, tag = "2")]
    pub key: u32,
    #[prost(string, tag = "3")]
    pub name: String,
    #[prost(bool, tag = "7")]
    pub supports_position: bool,
}

#[derive(Clone, PartialEq, Message)]
pub struct ListEntitiesLightResponse {
    #[prost(string, tag = "1")]
    pub object_id: String,
    #[prost(fixed32, tag = "2")]
    pub key: u32,
    #[prost(string, tag = "3")]
    pub name: String,
    #[prost(float, tag = "9")]
    pub min_mireds: f32,
    #[prost(float, tag = "10")]
    pub max_mireds: f32,
    #[prost(int32, repeated, tag = "12")]
    pub supported_color_modes: Vec<i32>,
}

#[derive(Clone, PartialEq, Message)]
pub struct ListEntitiesSensorResponse {
    #[prost(string, tag = "1")]
    pub object_id: String,
    #[prost(fixed32, tag = "2")]
    pub key: u32,
    #[prost(string, tag = "3")]
    pub name: String,
    #[prost(string, tag = "6")]
    pub unit_of_measurement: String,
    #[prost(string, tag = "9")]
    pub device_class: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct ListEntitiesSwitchResponse {
    #[prost(string, tag = "1")]
    pub object_id: String,
    #[prost(fixed32, tag = "2")]
    pub key: u32,
    #[prost(string, tag = "3")]
    pub name: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct ListEntitiesClimateResponse {
    #[prost(string, tag = "1")]
    pub object_id: String,
    #[prost(fixed32, tag = "2")]
    pub key: u32,
    #[prost(string, tag = "3")]
    pub name: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct ListEntitiesLockResponse {
    #[prost(string, tag = "1")]
    pub object_id: String,
    #[prost(fixed32, tag = "2")]
    pub key: u32,
    #[prost(string, tag = "3")]
    pub name: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct ListEntitiesButtonResponse {
    #[prost(string, tag = "1")]
    pub object_id: String,
    #[prost(fixed32, tag = "2")]
    pub key: u32,
    #[prost(string, tag = "3")]
    pub name: String,
}

// ==================== Entity States ====================

#[derive(Clone, PartialEq, Message)]
pub struct BinarySensorStateResponse {
    #[prost(fixed32, tag = "1")]
    pub key: u32,
    #[prost(bool, tag = "2")]
    pub state: bool,
    #[prost(bool, tag = "3")]
    pub missing_state: bool,
}

#[derive(Clone, PartialEq, Message)]
pub struct CoverStateResponse {
    #[prost(fixed32, tag = "1")]
    pub key: u32,
    #[prost(float, tag = "3")]
    pub position: f32,
    #[prost(int32, tag = "5")]
    pub current_operation: i32,
}

#[derive(Clone, PartialEq, Message)]
pub struct LightStateResponse {
    #[prost(fixed32, tag = "1")]
    pub key: u32,
    #[prost(bool, tag = "2")]
    pub state: bool,
    #[prost(float, tag = "3")]
    pub brightness: f32,
    #[prost(float, tag = "4")]
    pub red: f32,
    #[prost(float, tag = "5")]
    pub green: f32,
    #[prost(float, tag = "6")]
    pub blue: f32,
    #[prost(float, tag = "8")]
    pub color_temperature: f32,
}

#[derive(Clone, PartialEq, Message)]
pub struct SensorStateResponse {
    #[prost(fixed32, tag = "1")]
    pub key: u32,
    #[prost(float, tag = "2")]
    pub state: f32,
    #[prost(bool, tag = "3")]
    pub missing_state: bool,
}

#[derive(Clone, PartialEq, Message)]
pub struct SwitchStateResponse {
    #[prost(fixed32, tag = "1")]
    pub key: u32,
    #[prost(bool, tag = "2")]
    pub state: bool,
}

#[derive(Clone, PartialEq, Message)]
pub struct ClimateStateResponse {
    #[prost(fixed32, tag = "1")]
    pub key: u32,
    #[prost(int32, tag = "2")]
    pub mode: i32,
    #[prost(float, tag = "3")]
    pub current_temperature: f32,
    #[prost(float, tag = "4")]
    pub target_temperature: f32,
}

#[derive(Clone, PartialEq, Message)]
pub struct LockStateResponse {
    #[prost(fixed32, tag = "1")]
    pub key: u32,
    #[prost(int32, tag = "2")]
    pub state: i32,
}

// ==================== Entity Commands ====================

#[derive(Clone, PartialEq, Message)]
pub struct CoverCommandRequest {
    #[prost(fixed32, tag = "1")]
    pub key: u32,
    #[prost(bool, tag = "4")]
    pub has_position: bool,
    #[prost(float, tag = "5")]
    pub position: f32,
    #[prost(bool, tag = "8")]
    pub stop: bool,
}

#[derive(Clone, PartialEq, Message)]
pub struct LightCommandRequest {
    #[prost(fixed32, tag = "1")]
    pub key: u32,
    #[prost(bool, tag = "2")]
    pub has_state: bool,
    #[prost(bool, tag = "3")]
    pub state: bool,
    #[prost(bool, tag = "4")]
    pub has_brightness: bool,
    #[prost(float, tag = "5")]
    pub brightness: f32,
    #[prost(bool, tag = "6")]
    pub has_rgb: bool,
    #[prost(float, tag = "7")]
    pub red: f32,
    #[prost(float, tag = "8")]
    pub green: f32,
    #[prost(float, tag = "9")]
    pub blue: f32,
    #[prost(bool, tag = "12")]
    pub has_color_temperature: bool,
    #[prost(float, tag = "13")]
    pub color_temperature: f32,
}

#[derive(Clone, PartialEq, Message)]
pub struct SwitchCommandRequest {
    #[prost(fixed32, tag = "1")]
    pub key: u32,
    #[prost(bool, tag = "2")]
    pub state: bool,
}

#[derive(Clone, PartialEq, Message)]
pub struct ClimateCommandRequest {
    #[prost(fixed32, tag = "1")]
    pub key: u32,
    #[prost(bool, tag = "2")]
    pub has_mode: bool,
    #[prost(int32, tag = "3")]
    pub mode: i32,
    #[prost(bool, tag = "4")]
    pub has_target_temperature: bool,
    #[prost(float, tag = "5")]
    pub target_temperature: f32,
}

#[derive(Clone, PartialEq, Message)]
pub struct LockCommandRequest {
    #[prost(fixed32, tag = "1")]
    pub key: u32,
    #[prost(int32, tag = "2")]
    pub command: i32,
}

#[derive(Clone, PartialEq, Message)]
pub struct ButtonCommandRequest {
    #[prost(fixed32, tag = "1")]
    pub key: u32,
}

// ==================== Helpers ====================

/// Encode an unsigned 32-bit integer as a Protobuf varint.
pub fn encode_varint(mut value: u32, out: &mut Vec<u8>) {
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

/// Convert ESPHome mireds to Kelvin (returns `None` for non-positive/NaN values).
pub fn mireds_to_kelvin(mireds: f32) -> Option<u16> {
    if !mireds.is_finite() || mireds <= 0.0 {
        return None;
    }
    let k = (1_000_000.0 / mireds).round();
    if (1.0..=f32::from(u16::MAX)).contains(&k) {
        Some(k as u16)
    } else {
        None
    }
}

/// Convert Kelvin to ESPHome mireds.
pub fn kelvin_to_mireds(kelvin: u16) -> Option<f32> {
    if kelvin == 0 {
        return None;
    }
    Some(1_000_000.0 / f32::from(kelvin))
}
