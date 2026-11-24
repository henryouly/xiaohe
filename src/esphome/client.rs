use std::collections::HashMap;
use std::time::Duration;

use prost::Message;
use serde_json::{Map, Value};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, watch};
use tracing::{debug, info, warn};

use crate::config::EspHomeNodeConfig;
use crate::esphome::proto::*;
use crate::esphome::transport::{EspHomeError, EspHomeTransport};
use crate::protocol::domain::{
    Availability, ClimateMode, CoverState, DomainState, EntityRecord, EntitySource, LockState,
    SensorValue,
};
use crate::state::StateStore;

/// Tunable timings for an ESPHome node actor (shortened in tests).
#[derive(Debug, Clone, Copy)]
pub struct NodeActorConfig {
    pub connect_timeout: Duration,
    pub ping_interval: Duration,
    pub ping_timeout: Duration,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
}

impl Default for NodeActorConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            ping_interval: Duration::from_secs(20),
            ping_timeout: Duration::from_secs(10),
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(60),
        }
    }
}

/// Command sent from `EspHomeManager` to a node actor.
pub struct NodeCommand {
    pub entity_id: String,
    pub action: String,
    pub params: Map<String, Value>,
    pub ack: oneshot::Sender<Result<(), EspHomeError>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntityKind {
    Light,
    Switch,
    Sensor,
    BinarySensor,
    Climate,
    Cover,
    Lock,
    Button,
}

#[derive(Debug, Clone)]
struct DiscoveredEntity {
    entity_id: String,
    kind: EntityKind,
    unit: Option<String>,
    device_class: Option<String>,
}

/// Sanitize a node/object identifier segment into lowercase ASCII `[a-z0-9_]`.
pub fn sanitize_slug(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut prev_underscore = false;
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_underscore = false;
        } else if !prev_underscore && !out.is_empty() {
            out.push('_');
            prev_underscore = true;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() {
        "entity".to_owned()
    } else {
        out
    }
}

pub fn make_entity_id(domain: &str, node_id: &str, object_id: &str) -> String {
    format!(
        "{domain}.{}_{}",
        sanitize_slug(node_id),
        sanitize_slug(object_id)
    )
}

fn is_shutdown(rx: &watch::Receiver<bool>) -> bool {
    *rx.borrow() || rx.has_changed().is_err()
}

/// Long-running actor managing one ESPHome node's TCP/Noise session and reconnection loop.
pub async fn run_node_actor(
    node: EspHomeNodeConfig,
    psk: Option<[u8; 32]>,
    state: StateStore,
    mut cmd_rx: mpsc::Receiver<NodeCommand>,
    mut shutdown_rx: watch::Receiver<bool>,
    cfg: NodeActorConfig,
) {
    let source = EntitySource::EspHome {
        node_id: node.id.clone(),
    };
    let mut backoff = cfg.initial_backoff;

    loop {
        if is_shutdown(&shutdown_rx) {
            break;
        }

        // Reject any commands queued while disconnected so callers don't hang.
        drain_offline_commands(&node.id, &mut cmd_rx);

        info!(
            node_id = %node.id,
            host = %node.host,
            port = node.port,
            encrypted = psk.is_some(),
            "connecting to ESPHome node"
        );

        let session_res = run_single_session(
            &node,
            psk,
            &source,
            &state,
            &mut cmd_rx,
            &mut shutdown_rx,
            cfg,
            &mut backoff,
        )
        .await;

        // Mark all entities from this node Offline whenever a session ends.
        state.set_source_availability(&source, Availability::Offline);
        drain_offline_commands(&node.id, &mut cmd_rx);

        if is_shutdown(&shutdown_rx) {
            break;
        }

        match session_res {
            Ok(()) => {
                info!(node_id = %node.id, "ESPHome session ended; reconnecting");
            }
            Err(err) => {
                warn!(
                    node_id = %node.id,
                    error = %err,
                    retry_in_ms = backoff.as_millis() as u64,
                    "ESPHome node disconnected; scheduling reconnect"
                );
            }
        }

        tokio::select! {
            biased;
            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() {
                    break;
                }
            }
            () = tokio::time::sleep(backoff) => {}
        }

        backoff = (backoff.saturating_mul(2)).min(cfg.max_backoff);
    }

    debug!(node_id = %node.id, "ESPHome node actor stopped");
}

type EstablishedSession = (
    EspHomeTransport,
    HashMap<u32, DiscoveredEntity>,
    HashMap<String, (u32, EntityKind)>,
);

async fn establish_session(
    node: &EspHomeNodeConfig,
    psk: Option<[u8; 32]>,
    source: &EntitySource,
    state: &StateStore,
) -> Result<EstablishedSession, EspHomeError> {
    let stream = TcpStream::connect((node.host.as_str(), node.port)).await?;
    let _ = stream.set_nodelay(true);

    let mut transport = match psk {
        Some(key) => EspHomeTransport::connect_noise(stream, key).await?,
        None => EspHomeTransport::plaintext(stream),
    };

    // 1. HelloRequest -> HelloResponse
    transport
        .send_message(
            MSG_HELLO_REQUEST,
            &HelloRequest {
                client_info: format!("xiaohe {}", env!("CARGO_PKG_VERSION")),
                api_version_major: 1,
                api_version_minor: 10,
            },
        )
        .await?;
    let (msg_type, payload) = transport.recv_frame().await?;
    if msg_type != MSG_HELLO_RESPONSE {
        return Err(EspHomeError::Handshake(format!(
            "expected HelloResponse (2), got message type {msg_type}"
        )));
    }
    let hello = HelloResponse::decode(payload.as_slice())?;
    debug!(
        node_id = %node.id,
        server_info = %hello.server_info,
        name = %hello.name,
        api_version = %format!("{}.{}", hello.api_version_major, hello.api_version_minor),
        "ESPHome Hello completed"
    );

    // 2. ConnectRequest -> ConnectResponse
    transport
        .send_message(
            MSG_CONNECT_REQUEST,
            &ConnectRequest {
                password: String::new(),
            },
        )
        .await?;
    let (msg_type, payload) = transport.recv_frame().await?;
    if msg_type != MSG_CONNECT_RESPONSE {
        return Err(EspHomeError::Handshake(format!(
            "expected ConnectResponse (4), got message type {msg_type}"
        )));
    }
    let connect_resp = ConnectResponse::decode(payload.as_slice())?;
    if connect_resp.invalid_password {
        return Err(EspHomeError::Handshake(
            "ESPHome rejected ConnectRequest (invalid password)".into(),
        ));
    }

    // 3. ListEntitiesRequest -> read until ListEntitiesDoneResponse (19)
    transport
        .send_message(MSG_LIST_ENTITIES_REQUEST, &ListEntitiesRequest {})
        .await?;

    let mut key_map: HashMap<u32, DiscoveredEntity> = HashMap::new();
    let mut entity_map: HashMap<String, (u32, EntityKind)> = HashMap::new();

    loop {
        let (msg_type, payload) = transport.recv_frame().await?;
        match msg_type {
            MSG_LIST_ENTITIES_DONE_RESPONSE => break,
            MSG_PING_REQUEST => {
                transport
                    .send_message(MSG_PING_RESPONSE, &PingResponse {})
                    .await?;
            }
            _ => {
                if let Some((key, discovered, record)) =
                    decode_discovery_message(msg_type, &payload, node, source)?
                {
                    entity_map.insert(discovered.entity_id.clone(), (key, discovered.kind));
                    key_map.insert(key, discovered);
                    state.register_entity(record);
                }
            }
        }
    }

    info!(
        node_id = %node.id,
        entity_count = key_map.len(),
        "ESPHome entity discovery completed"
    );

    // 4. SubscribeStatesRequest
    transport
        .send_message(MSG_SUBSCRIBE_STATES_REQUEST, &SubscribeStatesRequest {})
        .await?;

    Ok((transport, key_map, entity_map))
}

#[allow(clippy::too_many_arguments)]
async fn run_single_session(
    node: &EspHomeNodeConfig,
    psk: Option<[u8; 32]>,
    source: &EntitySource,
    state: &StateStore,
    cmd_rx: &mut mpsc::Receiver<NodeCommand>,
    shutdown_rx: &mut watch::Receiver<bool>,
    cfg: NodeActorConfig,
    backoff: &mut Duration,
) -> Result<(), EspHomeError> {
    let (mut transport, key_map, entity_map) = tokio::select! {
        biased;
        changed = shutdown_rx.changed() => {
            if changed.is_err() || *shutdown_rx.borrow() {
                return Ok(());
            }
            return Ok(());
        }
        res = tokio::time::timeout(
            cfg.connect_timeout,
            establish_session(node, psk, source, state),
        ) => {
            res.map_err(|_| EspHomeError::Handshake("connect/discovery timed out".into()))??
        }
    };

    // Connection + discovery succeeded — reset reconnect backoff.
    *backoff = cfg.initial_backoff;

    // 5. Connected event loop: incoming frames, outbound commands, keep-alive pings, shutdown.
    let mut ping_interval = tokio::time::interval(cfg.ping_interval);
    ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Skip immediate first tick.
    ping_interval.tick().await;

    let mut awaiting_pong = false;
    let pong_deadline = tokio::time::sleep(Duration::from_secs(86_400));
    tokio::pin!(pong_deadline);

    loop {
        tokio::select! {
            biased;
            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() {
                    let _ = transport.send_message(MSG_DISCONNECT_REQUEST, &DisconnectRequest {}).await;
                    return Ok(());
                }
            }
            frame_res = transport.recv_frame() => {
                let (msg_type, payload) = frame_res?;
                match msg_type {
                    MSG_PING_REQUEST => {
                        transport.send_message(MSG_PING_RESPONSE, &PingResponse {}).await?;
                    }
                    MSG_PING_RESPONSE => {
                        awaiting_pong = false;
                        pong_deadline.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(86_400));
                    }
                    MSG_DISCONNECT_REQUEST => {
                        let _ = transport.send_message(MSG_DISCONNECT_RESPONSE, &DisconnectResponse {}).await;
                        return Ok(());
                    }
                    _ => {
                        apply_state_message(msg_type, &payload, &key_map, state)?;
                    }
                }
            }
            Some(cmd) = cmd_rx.recv() => {
                let res = send_entity_command(&mut transport, &entity_map, &cmd).await;
                let fatal = matches!(&res, Err(EspHomeError::Io(_) | EspHomeError::Noise(_) | EspHomeError::Disconnected));
                let _ = cmd.ack.send(res);
                if fatal {
                    return Err(EspHomeError::Disconnected);
                }
            }
            _ = ping_interval.tick() => {
                if !awaiting_pong {
                    transport.send_message(MSG_PING_REQUEST, &PingRequest {}).await?;
                    awaiting_pong = true;
                    pong_deadline.as_mut().reset(tokio::time::Instant::now() + cfg.ping_timeout);
                }
            }
            () = &mut pong_deadline, if awaiting_pong => {
                return Err(EspHomeError::Protocol("keep-alive ping timed out".into()));
            }
        }
    }
}

fn drain_offline_commands(node_id: &str, cmd_rx: &mut mpsc::Receiver<NodeCommand>) {
    while let Ok(cmd) = cmd_rx.try_recv() {
        let _ = cmd
            .ack
            .send(Err(EspHomeError::NodeOffline(node_id.to_owned())));
    }
}

fn decode_discovery_message(
    msg_type: u16,
    payload: &[u8],
    node: &EspHomeNodeConfig,
    source: &EntitySource,
) -> Result<Option<(u32, DiscoveredEntity, EntityRecord)>, EspHomeError> {
    let device_id = Some(format!("esphome:{}", node.id));
    let area_id = node.area_id.clone();

    let (key, entity_id, name, kind, state, capabilities, unit, device_class) = match msg_type {
        MSG_LIST_ENTITIES_LIGHT_RESPONSE => {
            let m = ListEntitiesLightResponse::decode(payload)?;
            let mut caps = Vec::new();
            // ESPHome color modes: 2=Brightness, 4=ColorTemp, 35=RGB, etc.
            if m.supported_color_modes.iter().any(|&c| c >= 2) {
                caps.push("brightness".to_owned());
            }
            if m.min_mireds > 0.0 && m.max_mireds > 0.0 {
                caps.push("color_temp".to_owned());
            }
            if m.supported_color_modes.iter().any(|&c| (c & 0x20) != 0) {
                caps.push("rgb".to_owned());
            }
            (
                m.key,
                make_entity_id("light", &node.id, &m.object_id),
                m.name,
                EntityKind::Light,
                DomainState::Light {
                    on: false,
                    brightness: None,
                    color_temp_k: None,
                    rgb: None,
                },
                caps,
                None,
                None,
            )
        }
        MSG_LIST_ENTITIES_SWITCH_RESPONSE => {
            let m = ListEntitiesSwitchResponse::decode(payload)?;
            (
                m.key,
                make_entity_id("switch", &node.id, &m.object_id),
                m.name,
                EntityKind::Switch,
                DomainState::Switch { on: false },
                vec![],
                None,
                None,
            )
        }
        MSG_LIST_ENTITIES_SENSOR_RESPONSE => {
            let m = ListEntitiesSensorResponse::decode(payload)?;
            let unit = (!m.unit_of_measurement.is_empty()).then_some(m.unit_of_measurement);
            let dc = (!m.device_class.is_empty()).then_some(m.device_class);
            (
                m.key,
                make_entity_id("sensor", &node.id, &m.object_id),
                m.name,
                EntityKind::Sensor,
                DomainState::Sensor {
                    value: SensorValue::Text("unknown".into()),
                    unit: unit.clone(),
                    device_class: dc.clone(),
                },
                vec![],
                unit,
                dc,
            )
        }
        MSG_LIST_ENTITIES_BINARY_SENSOR_RESPONSE => {
            let m = ListEntitiesBinarySensorResponse::decode(payload)?;
            let dc = (!m.device_class.is_empty()).then_some(m.device_class);
            (
                m.key,
                make_entity_id("binary_sensor", &node.id, &m.object_id),
                m.name,
                EntityKind::BinarySensor,
                DomainState::BinarySensor {
                    active: false,
                    device_class: dc.clone(),
                },
                vec![],
                None,
                dc,
            )
        }
        MSG_LIST_ENTITIES_CLIMATE_RESPONSE => {
            let m = ListEntitiesClimateResponse::decode(payload)?;
            (
                m.key,
                make_entity_id("climate", &node.id, &m.object_id),
                m.name,
                EntityKind::Climate,
                DomainState::Climate {
                    mode: ClimateMode::Off,
                    target_temp_c: None,
                    current_temp_c: None,
                    fan_mode: None,
                },
                vec![],
                None,
                None,
            )
        }
        MSG_LIST_ENTITIES_COVER_RESPONSE => {
            let m = ListEntitiesCoverResponse::decode(payload)?;
            let caps = if m.supports_position {
                vec!["position".to_owned()]
            } else {
                vec![]
            };
            (
                m.key,
                make_entity_id("cover", &node.id, &m.object_id),
                m.name,
                EntityKind::Cover,
                DomainState::Cover {
                    state: CoverState::Closed,
                    position: None,
                },
                caps,
                None,
                None,
            )
        }
        MSG_LIST_ENTITIES_LOCK_RESPONSE => {
            let m = ListEntitiesLockResponse::decode(payload)?;
            (
                m.key,
                make_entity_id("lock", &node.id, &m.object_id),
                m.name,
                EntityKind::Lock,
                DomainState::Lock {
                    state: LockState::Locked,
                },
                vec![],
                None,
                None,
            )
        }
        MSG_LIST_ENTITIES_BUTTON_RESPONSE => {
            let m = ListEntitiesButtonResponse::decode(payload)?;
            (
                m.key,
                make_entity_id("button", &node.id, &m.object_id),
                m.name,
                EntityKind::Button,
                DomainState::Button {
                    last_pressed_at: None,
                },
                vec![],
                None,
                None,
            )
        }
        _ => {
            debug!(msg_type, "ignoring unsupported ESPHome discovery message");
            return Ok(None);
        }
    };

    let discovered = DiscoveredEntity {
        entity_id: entity_id.clone(),
        kind,
        unit,
        device_class,
    };

    let record = EntityRecord {
        entity_id,
        device_id,
        area_id,
        name,
        source: source.clone(),
        availability: Availability::Online,
        state,
        capabilities,
        extra: Map::new(),
        custom_actions: vec![],
        last_changed_at: 0,
        last_updated_at: 0,
    };

    Ok(Some((key, discovered, record)))
}

fn apply_state_message(
    msg_type: u16,
    payload: &[u8],
    key_map: &HashMap<u32, DiscoveredEntity>,
    state: &StateStore,
) -> Result<(), EspHomeError> {
    match msg_type {
        MSG_LIGHT_STATE_RESPONSE => {
            let m = LightStateResponse::decode(payload)?;
            let Some(ent) = key_map.get(&m.key) else {
                return Ok(());
            };
            let brightness = if m.brightness.is_finite() {
                Some((m.brightness.clamp(0.0, 1.0) * 100.0).round() as u8)
            } else {
                None
            };
            let rgb = if m.red > 0.0 || m.green > 0.0 || m.blue > 0.0 {
                Some([
                    (m.red.clamp(0.0, 1.0) * 255.0).round() as u8,
                    (m.green.clamp(0.0, 1.0) * 255.0).round() as u8,
                    (m.blue.clamp(0.0, 1.0) * 255.0).round() as u8,
                ])
            } else {
                None
            };
            state.update_state(
                &ent.entity_id,
                DomainState::Light {
                    on: m.state,
                    brightness,
                    color_temp_k: mireds_to_kelvin(m.color_temperature),
                    rgb,
                },
                None,
                Some(Availability::Online),
            );
        }
        MSG_SWITCH_STATE_RESPONSE => {
            let m = SwitchStateResponse::decode(payload)?;
            if let Some(ent) = key_map.get(&m.key) {
                state.update_state(
                    &ent.entity_id,
                    DomainState::Switch { on: m.state },
                    None,
                    Some(Availability::Online),
                );
            }
        }
        MSG_SENSOR_STATE_RESPONSE => {
            let m = SensorStateResponse::decode(payload)?;
            if m.missing_state || !m.state.is_finite() {
                return Ok(());
            }
            if let Some(ent) = key_map.get(&m.key) {
                state.update_state(
                    &ent.entity_id,
                    DomainState::Sensor {
                        value: SensorValue::Number(f64::from(m.state)),
                        unit: ent.unit.clone(),
                        device_class: ent.device_class.clone(),
                    },
                    None,
                    Some(Availability::Online),
                );
            }
        }
        MSG_BINARY_SENSOR_STATE_RESPONSE => {
            let m = BinarySensorStateResponse::decode(payload)?;
            if m.missing_state {
                return Ok(());
            }
            if let Some(ent) = key_map.get(&m.key) {
                state.update_state(
                    &ent.entity_id,
                    DomainState::BinarySensor {
                        active: m.state,
                        device_class: ent.device_class.clone(),
                    },
                    None,
                    Some(Availability::Online),
                );
            }
        }
        MSG_CLIMATE_STATE_RESPONSE => {
            let m = ClimateStateResponse::decode(payload)?;
            if let Some(ent) = key_map.get(&m.key) {
                let mode = match m.mode {
                    0 => ClimateMode::Off,
                    1 | 6 => ClimateMode::Auto,
                    2 => ClimateMode::Cool,
                    3 => ClimateMode::Heat,
                    4 => ClimateMode::FanOnly,
                    5 => ClimateMode::Dry,
                    _ => ClimateMode::Off,
                };
                let current_temp_c = m
                    .current_temperature
                    .is_finite()
                    .then_some(m.current_temperature);
                let target_temp_c = m
                    .target_temperature
                    .is_finite()
                    .then_some(m.target_temperature);
                state.update_state(
                    &ent.entity_id,
                    DomainState::Climate {
                        mode,
                        target_temp_c,
                        current_temp_c,
                        fan_mode: None,
                    },
                    None,
                    Some(Availability::Online),
                );
            }
        }
        MSG_COVER_STATE_RESPONSE => {
            let m = CoverStateResponse::decode(payload)?;
            if let Some(ent) = key_map.get(&m.key) {
                let pos = if m.position.is_finite() {
                    Some((m.position.clamp(0.0, 1.0) * 100.0).round() as u8)
                } else {
                    None
                };
                let cover_state = match m.current_operation {
                    1 => CoverState::Opening,
                    2 => CoverState::Closing,
                    _ => match pos {
                        Some(0) => CoverState::Closed,
                        Some(_) => CoverState::Open,
                        None => CoverState::Stopped,
                    },
                };
                state.update_state(
                    &ent.entity_id,
                    DomainState::Cover {
                        state: cover_state,
                        position: pos,
                    },
                    None,
                    Some(Availability::Online),
                );
            }
        }
        MSG_LOCK_STATE_RESPONSE => {
            let m = LockStateResponse::decode(payload)?;
            if let Some(ent) = key_map.get(&m.key) {
                let lock_state = match m.state {
                    1 => LockState::Locked,
                    2 => LockState::Unlocked,
                    3 => LockState::Jammed,
                    4 => LockState::Locking,
                    5 => LockState::Unlocking,
                    _ => LockState::Unlocked,
                };
                state.update_state(
                    &ent.entity_id,
                    DomainState::Lock { state: lock_state },
                    None,
                    Some(Availability::Online),
                );
            }
        }
        _ => {
            debug!(msg_type, "ignoring unhandled ESPHome message");
        }
    }
    Ok(())
}

async fn send_entity_command(
    transport: &mut EspHomeTransport,
    entity_map: &HashMap<String, (u32, EntityKind)>,
    cmd: &NodeCommand,
) -> Result<(), EspHomeError> {
    let &(key, kind) = entity_map
        .get(&cmd.entity_id)
        .ok_or_else(|| EspHomeError::UnknownEntity(cmd.entity_id.clone()))?;

    match kind {
        EntityKind::Light => {
            let mut req = LightCommandRequest {
                key,
                ..Default::default()
            };
            match cmd.action.as_str() {
                "turn_on" => {
                    req.has_state = true;
                    req.state = true;
                }
                "turn_off" => {
                    req.has_state = true;
                    req.state = false;
                }
                other => {
                    return Err(EspHomeError::InvalidCommand(format!(
                        "unsupported light action '{other}'"
                    )));
                }
            }
            if let Some(b) = cmd.params.get("brightness").and_then(Value::as_u64) {
                req.has_brightness = true;
                req.brightness = (b.min(100) as f32) / 100.0;
            }
            if let Some(k) = cmd.params.get("color_temp_k").and_then(Value::as_u64)
                && let Ok(k16) = u16::try_from(k)
                && let Some(mireds) = kelvin_to_mireds(k16)
            {
                req.has_color_temperature = true;
                req.color_temperature = mireds;
            }
            if let Some(arr) = cmd.params.get("rgb").and_then(Value::as_array)
                && arr.len() == 3
                && let (Some(r), Some(g), Some(b)) =
                    (arr[0].as_u64(), arr[1].as_u64(), arr[2].as_u64())
            {
                req.has_rgb = true;
                req.red = (r.min(255) as f32) / 255.0;
                req.green = (g.min(255) as f32) / 255.0;
                req.blue = (b.min(255) as f32) / 255.0;
            }
            transport
                .send_message(MSG_LIGHT_COMMAND_REQUEST, &req)
                .await
        }
        EntityKind::Switch => {
            let state = match cmd.action.as_str() {
                "turn_on" => true,
                "turn_off" => false,
                other => {
                    return Err(EspHomeError::InvalidCommand(format!(
                        "unsupported switch action '{other}'"
                    )));
                }
            };
            transport
                .send_message(
                    MSG_SWITCH_COMMAND_REQUEST,
                    &SwitchCommandRequest { key, state },
                )
                .await
        }
        EntityKind::Climate => {
            let mut req = ClimateCommandRequest {
                key,
                ..Default::default()
            };
            match cmd.action.as_str() {
                "set_temperature" => {
                    let temp = cmd
                        .params
                        .get("target_temp_c")
                        .and_then(Value::as_f64)
                        .ok_or_else(|| {
                            EspHomeError::InvalidCommand("missing target_temp_c".into())
                        })?;
                    req.has_target_temperature = true;
                    req.target_temperature = temp as f32;
                }
                "set_mode" => {
                    let mode_str = cmd
                        .params
                        .get("mode")
                        .and_then(Value::as_str)
                        .ok_or_else(|| EspHomeError::InvalidCommand("missing mode".into()))?;
                    req.has_mode = true;
                    req.mode = match mode_str {
                        "off" => 0,
                        "auto" => 1,
                        "cool" => 2,
                        "heat" => 3,
                        "fan_only" => 4,
                        "dry" => 5,
                        other => {
                            return Err(EspHomeError::InvalidCommand(format!(
                                "invalid climate mode '{other}'"
                            )));
                        }
                    };
                }
                other => {
                    return Err(EspHomeError::InvalidCommand(format!(
                        "unsupported climate action '{other}'"
                    )));
                }
            }
            transport
                .send_message(MSG_CLIMATE_COMMAND_REQUEST, &req)
                .await
        }
        EntityKind::Cover => {
            let mut req = CoverCommandRequest {
                key,
                ..Default::default()
            };
            match cmd.action.as_str() {
                "open" => {
                    req.has_position = true;
                    req.position = 1.0;
                }
                "close" => {
                    req.has_position = true;
                    req.position = 0.0;
                }
                "stop" => {
                    req.stop = true;
                }
                "set_position" => {
                    let pos = cmd
                        .params
                        .get("position")
                        .and_then(Value::as_u64)
                        .ok_or_else(|| EspHomeError::InvalidCommand("missing position".into()))?;
                    req.has_position = true;
                    req.position = (pos.min(100) as f32) / 100.0;
                }
                other => {
                    return Err(EspHomeError::InvalidCommand(format!(
                        "unsupported cover action '{other}'"
                    )));
                }
            }
            transport
                .send_message(MSG_COVER_COMMAND_REQUEST, &req)
                .await
        }
        EntityKind::Lock => {
            let command = match cmd.action.as_str() {
                "unlock" => 0,
                "lock" => 1,
                "open" => 2,
                other => {
                    return Err(EspHomeError::InvalidCommand(format!(
                        "unsupported lock action '{other}'"
                    )));
                }
            };
            transport
                .send_message(
                    MSG_LOCK_COMMAND_REQUEST,
                    &LockCommandRequest { key, command },
                )
                .await
        }
        EntityKind::Button => {
            if cmd.action != "press" {
                return Err(EspHomeError::InvalidCommand(format!(
                    "unsupported button action '{}'",
                    cmd.action
                )));
            }
            transport
                .send_message(MSG_BUTTON_COMMAND_REQUEST, &ButtonCommandRequest { key })
                .await
        }
        EntityKind::Sensor | EntityKind::BinarySensor => Err(EspHomeError::InvalidCommand(
            format!("entity '{}' is read-only", cmd.entity_id),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::EventBus;

    #[test]
    fn climate_mode_auto_enum_value_6_maps_to_auto() {
        let bus = EventBus::new();
        let state = StateStore::new(bus);
        let source = EntitySource::EspHome {
            node_id: "esp_ac".into(),
        };
        let node = EspHomeNodeConfig {
            id: "esp_ac".into(),
            host: "127.0.0.1".into(),
            port: 6053,
            noise_psk_secret: None,
            area_id: None,
            enabled: true,
        };

        let mut disc_payload = Vec::new();
        ListEntitiesClimateResponse {
            object_id: "study_ac".into(),
            key: 55,
            name: "Study AC".into(),
        }
        .encode(&mut disc_payload)
        .unwrap();
        let (key, discovered, record) = decode_discovery_message(
            MSG_LIST_ENTITIES_CLIMATE_RESPONSE,
            &disc_payload,
            &node,
            &source,
        )
        .unwrap()
        .unwrap();
        state.register_entity(record);
        let mut key_map = HashMap::new();
        key_map.insert(key, discovered);

        let mut state_payload = Vec::new();
        ClimateStateResponse {
            key: 55,
            mode: 6, // CLIMATE_MODE_AUTO in official ESPHome api.proto
            current_temperature: 25.0,
            target_temperature: 23.0,
        }
        .encode(&mut state_payload)
        .unwrap();
        apply_state_message(MSG_CLIMATE_STATE_RESPONSE, &state_payload, &key_map, &state).unwrap();

        let rec = state.get("climate.esp_ac_study_ac").unwrap();
        assert!(
            matches!(
                rec.state,
                DomainState::Climate {
                    mode: ClimateMode::Auto,
                    ..
                }
            ),
            "expected ClimateMode::Auto for ESPHome enum value 6, got {:?}",
            rec.state
        );
    }
}
