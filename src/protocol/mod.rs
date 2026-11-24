pub mod domain;
pub mod rpc;
pub mod skill;

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::domain::{
        Availability, ClimateMode, CoverState, DomainState, EntityRecord, EntitySource, LockState,
        MediaState, SensorValue,
    };
    use super::rpc::{
        ERR_METHOD_NOT_FOUND, EntityCommandParams, EntityRegisterParams, EntityStateChangedParams,
        JSONRPC_VERSION, OkResult, PROTOCOL_VERSION, RpcId, RpcNotification, RpcRequest,
        RpcResponse, SkillInitParams, SkillInvokeParams,
    };
    use super::skill::{RestartPolicy, RuntimeType, SkillMode, SkillStatus};

    #[test]
    fn domain_state_variants_match_documented_wire_json() {
        let cases: Vec<(DomainState, serde_json::Value)> = vec![
            (
                DomainState::Light {
                    on: true,
                    brightness: Some(80),
                    color_temp_k: Some(3200),
                    rgb: Some([255, 200, 150]),
                },
                json!({
                    "domain": "light",
                    "on": true,
                    "brightness": 80,
                    "color_temp_k": 3200,
                    "rgb": [255, 200, 150]
                }),
            ),
            (
                DomainState::Switch { on: false },
                json!({ "domain": "switch", "on": false }),
            ),
            (
                DomainState::Climate {
                    mode: ClimateMode::FanOnly,
                    target_temp_c: Some(23.5),
                    current_temp_c: None,
                    fan_mode: Some("auto".into()),
                },
                json!({
                    "domain": "climate",
                    "mode": "fan_only",
                    "target_temp_c": 23.5,
                    "fan_mode": "auto"
                }),
            ),
            (
                DomainState::Sensor {
                    value: SensorValue::Number(14.0),
                    unit: Some("µg/m³".into()),
                    device_class: Some("pm25".into()),
                },
                json!({
                    "domain": "sensor",
                    "value": 14.0,
                    "unit": "µg/m³",
                    "device_class": "pm25"
                }),
            ),
            (
                DomainState::Sensor {
                    value: SensorValue::Text("good".into()),
                    unit: None,
                    device_class: None,
                },
                json!({ "domain": "sensor", "value": "good" }),
            ),
            (
                DomainState::BinarySensor {
                    active: true,
                    device_class: Some("motion".into()),
                },
                json!({
                    "domain": "binary_sensor",
                    "active": true,
                    "device_class": "motion"
                }),
            ),
            (
                DomainState::Cover {
                    state: CoverState::Opening,
                    position: Some(50),
                },
                json!({ "domain": "cover", "state": "opening", "position": 50 }),
            ),
            (
                DomainState::Lock {
                    state: LockState::Locked,
                },
                json!({ "domain": "lock", "state": "locked" }),
            ),
            (
                DomainState::MediaPlayer {
                    state: MediaState::Playing,
                    volume: Some(45),
                    muted: Some(false),
                    title: Some("Track 1".into()),
                },
                json!({
                    "domain": "media_player",
                    "state": "playing",
                    "volume": 45,
                    "muted": false,
                    "title": "Track 1"
                }),
            ),
            (
                DomainState::Button {
                    last_pressed_at: Some(1_700_000_000_000),
                },
                json!({ "domain": "button", "last_pressed_at": 1_700_000_000_000_i64 }),
            ),
            (
                DomainState::Custom {
                    state: "favorite".into(),
                },
                json!({ "domain": "custom", "state": "favorite" }),
            ),
        ];

        for (variant, expected_json) in cases {
            let actual_json = serde_json::to_value(&variant).expect("serialize DomainState");
            assert_eq!(actual_json, expected_json);
            let parsed: DomainState =
                serde_json::from_value(expected_json).expect("deserialize DomainState");
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn entity_record_round_trip() {
        let record = EntityRecord {
            entity_id: "light.living_ceiling".into(),
            device_id: Some("esphome:esp32_living".into()),
            area_id: Some("living_room".into()),
            name: "Living Room Ceiling".into(),
            source: EntitySource::EspHome {
                node_id: "esp32_living".into(),
            },
            availability: Availability::Online,
            state: DomainState::Light {
                on: true,
                brightness: Some(75),
                color_temp_k: Some(3200),
                rgb: None,
            },
            capabilities: vec!["brightness".into(), "color_temp".into()],
            extra: Default::default(),
            custom_actions: vec![],
            last_changed_at: 1_700_000_000_000,
            last_updated_at: 1_700_000_000_500,
        };

        let json = serde_json::to_value(&record).expect("serialize EntityRecord");
        assert_eq!(json["source"]["type"], "esphome");
        assert_eq!(json["state"]["domain"], "light");

        let parsed: EntityRecord = serde_json::from_value(json).expect("deserialize EntityRecord");
        assert_eq!(record, parsed);
    }

    #[test]
    fn skill_spec_init_handshake_deserializes() {
        let raw = r#"{
            "jsonrpc": "2.0",
            "id": 1,
            "method": "skill.init",
            "params": {
                "protocol_version": "1.0",
                "skill_id": "miio-airpurifier",
                "config": { "host": "192.168.1.42", "poll_interval_secs": 10, "area_id": "living_room" },
                "secrets": { "miio_token": "0123456789abcdef0123456789abcdef" }
            }
        }"#;

        let req: RpcRequest = serde_json::from_str(raw).expect("parse RpcRequest");
        assert_eq!(req.jsonrpc, JSONRPC_VERSION);
        assert_eq!(req.id, RpcId::Number(1));
        assert_eq!(req.method, "skill.init");

        let params: SkillInitParams =
            serde_json::from_value(req.params.expect("params")).expect("parse SkillInitParams");
        assert_eq!(params.protocol_version, PROTOCOL_VERSION);
        assert_eq!(params.skill_id, "miio-airpurifier");
        assert_eq!(params.config["host"], "192.168.1.42");
        assert_eq!(
            params.secrets["miio_token"],
            "0123456789abcdef0123456789abcdef"
        );

        let resp = RpcResponse::ok(RpcId::Number(1), json!(OkResult { ok: true }));
        let resp_json = serde_json::to_value(&resp).expect("serialize RpcResponse");
        assert_eq!(resp_json["result"]["ok"], true);
        assert!(resp_json.get("error").is_none());
    }

    #[test]
    fn skill_spec_entity_register_and_state_changed_deserialize() {
        let reg_raw = r#"{
            "jsonrpc": "2.0",
            "method": "entity.register",
            "params": {
                "device": {
                    "device_id": "miio_living_purifier",
                    "name": "Living Room Air Purifier",
                    "manufacturer": "Xiaomi",
                    "model": "zhimi.airpurifier.mb3",
                    "area_id": "living_room"
                },
                "entities": [
                    {
                        "entity_id": "switch.living_purifier_power",
                        "name": "Purifier Power",
                        "domain": "switch",
                        "capabilities": []
                    },
                    {
                        "entity_id": "sensor.living_purifier_pm25",
                        "name": "Living Room PM2.5",
                        "domain": "sensor",
                        "unit": "µg/m³",
                        "device_class": "pm25"
                    }
                ]
            }
        }"#;

        let notif: RpcNotification = serde_json::from_str(reg_raw).expect("parse notification");
        assert_eq!(notif.method, "entity.register");
        let reg: EntityRegisterParams =
            serde_json::from_value(notif.params.expect("params")).expect("parse register params");
        assert_eq!(reg.device.device_id, "miio_living_purifier");
        assert_eq!(reg.entities.len(), 2);

        let state_raw = r#"{
            "jsonrpc": "2.0",
            "method": "entity.state_changed",
            "params": {
                "entity_id": "sensor.living_purifier_pm25",
                "availability": "online",
                "state": {
                    "domain": "sensor",
                    "value": 14.0,
                    "unit": "µg/m³",
                    "device_class": "pm25"
                },
                "extra": {
                    "filter_life_remaining": 78
                }
            }
        }"#;

        let state_notif: RpcNotification =
            serde_json::from_str(state_raw).expect("parse state_changed");
        let state_params: EntityStateChangedParams =
            serde_json::from_value(state_notif.params.expect("params"))
                .expect("parse EntityStateChangedParams");
        assert_eq!(state_params.entity_id, "sensor.living_purifier_pm25");
        assert_eq!(state_params.availability, Availability::Online);
        assert_eq!(state_params.extra["filter_life_remaining"], 78);
    }

    #[test]
    fn skill_spec_command_invoke_and_error_response() {
        let cmd_raw = r#"{
            "jsonrpc": "2.0",
            "id": 42,
            "method": "entity.command",
            "params": {
                "entity_id": "switch.living_purifier_power",
                "action": "turn_on",
                "params": {}
            }
        }"#;
        let req: RpcRequest = serde_json::from_str(cmd_raw).expect("parse entity.command");
        let cmd: EntityCommandParams =
            serde_json::from_value(req.params.expect("params")).expect("parse EntityCommandParams");
        assert_eq!(cmd.action, "turn_on");

        let invoke_raw = r#"{
            "jsonrpc": "2.0",
            "id": 99,
            "method": "skill.invoke",
            "params": {
                "action": "adjust_climate_comfort",
                "params": { "area_id": "living_room" },
                "context": {
                    "trigger_entity_id": "sensor.living_temp"
                }
            }
        }"#;
        let invoke_req: RpcRequest = serde_json::from_str(invoke_raw).expect("parse skill.invoke");
        let invoke: SkillInvokeParams = serde_json::from_value(invoke_req.params.expect("params"))
            .expect("parse SkillInvokeParams");
        assert_eq!(invoke.action, "adjust_climate_comfort");
        assert_eq!(invoke.context["trigger_entity_id"], "sensor.living_temp");

        let err = RpcResponse::err(
            Some(RpcId::String("req-1".into())),
            ERR_METHOD_NOT_FOUND,
            "method not found",
        );
        let err_json = serde_json::to_value(&err).expect("serialize error response");
        assert_eq!(err_json["error"]["code"], ERR_METHOD_NOT_FOUND);
        assert!(err_json.get("result").is_none());
    }

    #[test]
    fn skill_enums_serialize_as_documented() {
        assert_eq!(serde_json::to_value(SkillMode::Device).unwrap(), "device");
        assert_eq!(serde_json::to_value(SkillMode::Action).unwrap(), "action");
        assert_eq!(
            serde_json::to_value(SkillStatus::Running).unwrap(),
            "running"
        );
        assert_eq!(
            serde_json::to_value(RuntimeType::PythonUv).unwrap(),
            "python-uv"
        );
        assert_eq!(
            serde_json::to_value(RestartPolicy::Exponential).unwrap(),
            "exponential"
        );
    }
}
