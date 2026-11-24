pub mod client;
pub mod proto;
pub mod transport;

use std::collections::HashMap;

use serde_json::{Map, Value};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use crate::config::{EspHomeNodeConfig, SecretsConfig};
use crate::esphome::client::{NodeActorConfig, NodeCommand, run_node_actor};
use crate::esphome::transport::decode_psk;
use crate::state::StateStore;

pub use transport::EspHomeError;

const COMMAND_CHANNEL_CAPACITY: usize = 32;

/// Manages all configured ESPHome Native API node connections.
pub struct EspHomeManager {
    senders: HashMap<String, mpsc::Sender<NodeCommand>>,
    handles: std::sync::Mutex<Vec<JoinHandle<()>>>,
}

impl EspHomeManager {
    /// Start one background Tokio actor per enabled ESPHome node using production timings.
    pub fn start(
        nodes: &[EspHomeNodeConfig],
        secrets: &SecretsConfig,
        state: StateStore,
        shutdown_rx: watch::Receiver<bool>,
    ) -> Self {
        Self::start_with_config(
            nodes,
            secrets,
            state,
            shutdown_rx,
            NodeActorConfig::default(),
        )
    }

    /// Start node actors with custom timings (useful for fast deterministic tests).
    pub fn start_with_config(
        nodes: &[EspHomeNodeConfig],
        secrets: &SecretsConfig,
        state: StateStore,
        shutdown_rx: watch::Receiver<bool>,
        actor_cfg: NodeActorConfig,
    ) -> Self {
        let mut senders = HashMap::new();
        let mut handles = Vec::new();

        for node in nodes {
            if !node.enabled {
                info!(node_id = %node.id, "skipping disabled ESPHome node");
                continue;
            }

            let psk = match &node.noise_psk_secret {
                Some(secret_key) => match secrets.get(secret_key) {
                    Some(encoded) => match decode_psk(&encoded) {
                        Ok(key) => Some(key),
                        Err(err) => {
                            error!(
                                node_id = %node.id,
                                secret_key = %secret_key,
                                error = %err,
                                "invalid ESPHome Noise PSK; refusing to start node in plaintext"
                            );
                            continue;
                        }
                    },
                    None => {
                        error!(
                            node_id = %node.id,
                            secret_key = %secret_key,
                            "missing ESPHome Noise PSK in secrets.toml; refusing to start node in plaintext"
                        );
                        continue;
                    }
                },
                None => None,
            };

            let (tx, rx) = mpsc::channel(COMMAND_CHANNEL_CAPACITY);
            let node_clone = node.clone();
            let state_clone = state.clone();
            let shutdown_clone = shutdown_rx.clone();

            let handle = tokio::spawn(async move {
                run_node_actor(node_clone, psk, state_clone, rx, shutdown_clone, actor_cfg).await;
            });

            senders.insert(node.id.clone(), tx);
            handles.push(handle);
        }

        Self {
            senders,
            handles: std::sync::Mutex::new(handles),
        }
    }

    /// Dispatch a domain command to a specific ESPHome node and await write acknowledgment.
    pub async fn dispatch_command(
        &self,
        node_id: &str,
        entity_id: &str,
        action: &str,
        params: Map<String, Value>,
    ) -> Result<(), EspHomeError> {
        let tx = self
            .senders
            .get(node_id)
            .ok_or_else(|| EspHomeError::NodeOffline(node_id.to_owned()))?;
        let (ack_tx, ack_rx) = oneshot::channel();
        tx.send(NodeCommand {
            entity_id: entity_id.to_owned(),
            action: action.to_owned(),
            params,
            ack: ack_tx,
        })
        .await
        .map_err(|_| EspHomeError::NodeOffline(node_id.to_owned()))?;

        ack_rx
            .await
            .map_err(|_| EspHomeError::NodeOffline(node_id.to_owned()))?
    }

    pub fn active_node_count(&self) -> usize {
        self.senders.len()
    }

    /// Wait for all ESPHome node actors to exit cleanly after shutdown is signaled.
    pub async fn shutdown(&self) {
        let handles: Vec<JoinHandle<()>> = {
            let mut guard = self.handles.lock().expect("esphome handles mutex poisoned");
            std::mem::take(&mut *guard)
        };
        for handle in handles {
            if let Err(err) = handle.await {
                warn!(error = %err, "ESPHome node actor exited abnormally");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use prost::Message;
    use serde_json::{Map, json};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::{mpsc, watch};

    use super::*;
    use crate::esphome::proto::*;
    use crate::esphome::transport::{INDICATOR_NOISE, INDICATOR_PLAINTEXT};
    use crate::protocol::domain::{Availability, DomainState, SensorValue};
    use crate::state::{EventBus, spawn_history_writer_with_config};
    use crate::storage::{EntityHistorySeries, HistoryStore, SqliteStore, TimeRange};

    enum MockTransport {
        Plaintext(TcpStream),
        Noise {
            stream: TcpStream,
            state: Box<snow::TransportState>,
        },
    }

    impl MockTransport {
        async fn accept(mut stream: TcpStream, psk: Option<[u8; 32]>) -> Result<Self, String> {
            match psk {
                None => Ok(Self::Plaintext(stream)),
                Some(key) => {
                    // 1. Read client empty hello [0x01, 0x00, 0x00]
                    let hello = read_noise_raw(&mut stream).await?;
                    if !hello.is_empty() {
                        return Err("expected empty client hello".into());
                    }
                    // 2. Send ServerHello [0x01, "mock\0", "AA:BB\0"]
                    write_noise_raw(&mut stream, b"\x01mock-node\0AA:BB:CC:DD:EE:FF\0").await?;
                    // 3. Read client Noise handshake msg 1 [0x00, ...]
                    let msg1 = read_noise_raw(&mut stream).await?;
                    if msg1.first().copied() != Some(0x00) {
                        return Err("missing 0x00 prefix on client handshake".into());
                    }
                    // Official ESPHome Noise_NNpsk0_25519_ChaChaPoly_SHA256 spec literals
                    // (independent of production constants so typos in prod constants are caught).
                    let params: snow::params::NoiseParams =
                        "Noise_NNpsk0_25519_ChaChaPoly_SHA256".parse().unwrap();
                    let mut responder = snow::Builder::new(params)
                        .prologue(b"NoiseAPIInit\0\0")
                        .psk(0, &key)
                        .build_responder()
                        .unwrap();
                    let mut out = vec![0u8; 1024];
                    if let Err(e) = responder.read_message(&msg1[1..], &mut out) {
                        let mut err_frame = vec![0x01];
                        err_frame.extend_from_slice(format!("bad psk: {e}").as_bytes());
                        let _ = write_noise_raw(&mut stream, &err_frame).await;
                        return Err(format!("handshake failed: {e}"));
                    }
                    let mut msg2 = vec![0u8; 1024];
                    let n2 = responder.write_message(&[], &mut msg2).unwrap();
                    let mut reply = Vec::with_capacity(1 + n2);
                    reply.push(0x00);
                    reply.extend_from_slice(&msg2[..n2]);
                    write_noise_raw(&mut stream, &reply).await?;
                    let transport = responder.into_transport_mode().unwrap();
                    Ok(Self::Noise {
                        stream,
                        state: Box::new(transport),
                    })
                }
            }
        }

        async fn send<M: Message>(&mut self, msg_type: u16, msg: &M) -> Result<(), String> {
            let mut payload = Vec::new();
            msg.encode(&mut payload).unwrap();
            match self {
                Self::Plaintext(s) => {
                    let mut frame = vec![INDICATOR_PLAINTEXT];
                    encode_varint(payload.len() as u32, &mut frame);
                    encode_varint(u32::from(msg_type), &mut frame);
                    frame.extend_from_slice(&payload);
                    s.write_all(&frame).await.map_err(|e| e.to_string())?;
                    s.flush().await.map_err(|e| e.to_string())
                }
                Self::Noise { stream, state } => {
                    let mut inner = Vec::with_capacity(4 + payload.len());
                    inner.extend_from_slice(&msg_type.to_be_bytes());
                    inner.extend_from_slice(&(payload.len() as u16).to_be_bytes());
                    inner.extend_from_slice(&payload);
                    let mut cipher = vec![0u8; inner.len() + 64];
                    let n = state.write_message(&inner, &mut cipher).unwrap();
                    write_noise_raw(stream, &cipher[..n]).await
                }
            }
        }

        async fn recv(&mut self) -> Result<(u16, Vec<u8>), String> {
            match self {
                Self::Plaintext(s) => {
                    let mut ind = [0u8; 1];
                    s.read_exact(&mut ind).await.map_err(|e| e.to_string())?;
                    let len = read_varint_mock(s).await? as usize;
                    let mt = read_varint_mock(s).await? as u16;
                    let mut buf = vec![0u8; len];
                    s.read_exact(&mut buf).await.map_err(|e| e.to_string())?;
                    Ok((mt, buf))
                }
                Self::Noise { stream, state } => {
                    let cipher = read_noise_raw(stream).await?;
                    let mut inner = vec![0u8; cipher.len() + 64];
                    let _n = state
                        .read_message(&cipher, &mut inner)
                        .map_err(|e| e.to_string())?;
                    let mt = u16::from_be_bytes([inner[0], inner[1]]);
                    let plen = u16::from_be_bytes([inner[2], inner[3]]) as usize;
                    Ok((mt, inner[4..4 + plen].to_vec()))
                }
            }
        }
    }

    async fn read_varint_mock(s: &mut TcpStream) -> Result<u32, String> {
        let mut res = 0u32;
        let mut shift = 0u32;
        for _ in 0..5 {
            let mut b = [0u8; 1];
            s.read_exact(&mut b).await.map_err(|e| e.to_string())?;
            res |= u32::from(b[0] & 0x7F) << shift;
            if (b[0] & 0x80) == 0 {
                return Ok(res);
            }
            shift += 7;
        }
        Err("varint overflow".into())
    }

    async fn write_noise_raw(s: &mut TcpStream, body: &[u8]) -> Result<(), String> {
        let mut hdr = [INDICATOR_NOISE, 0, 0];
        hdr[1..3].copy_from_slice(&(body.len() as u16).to_be_bytes());
        s.write_all(&hdr).await.map_err(|e| e.to_string())?;
        if !body.is_empty() {
            s.write_all(body).await.map_err(|e| e.to_string())?;
        }
        s.flush().await.map_err(|e| e.to_string())
    }

    async fn read_noise_raw(s: &mut TcpStream) -> Result<Vec<u8>, String> {
        let mut hdr = [0u8; 3];
        s.read_exact(&mut hdr).await.map_err(|e| e.to_string())?;
        if hdr[0] != INDICATOR_NOISE {
            return Err(format!("bad indicator {}", hdr[0]));
        }
        let len = u16::from_be_bytes([hdr[1], hdr[2]]) as usize;
        let mut buf = vec![0u8; len];
        if len > 0 {
            s.read_exact(&mut buf).await.map_err(|e| e.to_string())?;
        }
        Ok(buf)
    }

    fn fast_actor_cfg() -> NodeActorConfig {
        NodeActorConfig {
            connect_timeout: Duration::from_secs(2),
            ping_interval: Duration::from_secs(30),
            ping_timeout: Duration::from_secs(5),
            initial_backoff: Duration::from_millis(50),
            max_backoff: Duration::from_millis(200),
        }
    }

    async fn run_mock_esphome_session(
        stream: TcpStream,
        psk: Option<[u8; 32]>,
        cmd_tx: mpsc::Sender<LightCommandRequest>,
    ) {
        let Ok(mut tr) = MockTransport::accept(stream, psk).await else {
            return;
        };

        // Hello
        let (mt, _) = tr.recv().await.unwrap();
        assert_eq!(mt, MSG_HELLO_REQUEST);
        tr.send(
            MSG_HELLO_RESPONSE,
            &HelloResponse {
                api_version_major: 1,
                api_version_minor: 10,
                server_info: "esphome-mock".into(),
                name: "living_esp32".into(),
            },
        )
        .await
        .unwrap();

        // Connect
        let (mt, _) = tr.recv().await.unwrap();
        assert_eq!(mt, MSG_CONNECT_REQUEST);
        tr.send(
            MSG_CONNECT_RESPONSE,
            &ConnectResponse {
                invalid_password: false,
            },
        )
        .await
        .unwrap();

        // ListEntities
        let (mt, _) = tr.recv().await.unwrap();
        assert_eq!(mt, MSG_LIST_ENTITIES_REQUEST);
        tr.send(
            MSG_LIST_ENTITIES_LIGHT_RESPONSE,
            &ListEntitiesLightResponse {
                object_id: "ceiling".into(),
                key: 101,
                name: "Ceiling Light".into(),
                min_mireds: 153.0,
                max_mireds: 500.0,
                supported_color_modes: vec![2, 4],
            },
        )
        .await
        .unwrap();
        tr.send(
            MSG_LIST_ENTITIES_SENSOR_RESPONSE,
            &ListEntitiesSensorResponse {
                object_id: "temp".into(),
                key: 102,
                name: "Temperature".into(),
                unit_of_measurement: "°C".into(),
                device_class: "temperature".into(),
            },
        )
        .await
        .unwrap();
        tr.send(
            MSG_LIST_ENTITIES_DONE_RESPONSE,
            &ListEntitiesDoneResponse {},
        )
        .await
        .unwrap();

        // SubscribeStates -> push initial states
        let (mt, _) = tr.recv().await.unwrap();
        assert_eq!(mt, MSG_SUBSCRIBE_STATES_REQUEST);
        tr.send(
            MSG_LIGHT_STATE_RESPONSE,
            &LightStateResponse {
                key: 101,
                state: true,
                brightness: 0.75,
                red: 0.0,
                green: 0.0,
                blue: 0.0,
                color_temperature: 250.0, // 4000K
            },
        )
        .await
        .unwrap();
        tr.send(
            MSG_SENSOR_STATE_RESPONSE,
            &SensorStateResponse {
                key: 102,
                state: 23.5,
                missing_state: false,
            },
        )
        .await
        .unwrap();

        // Serve commands / pings until disconnect
        while let Ok((mt, payload)) = tr.recv().await {
            match mt {
                MSG_LIGHT_COMMAND_REQUEST => {
                    let req = LightCommandRequest::decode(payload.as_slice()).unwrap();
                    let _ = cmd_tx.send(req.clone()).await;
                    let _ = tr
                        .send(
                            MSG_LIGHT_STATE_RESPONSE,
                            &LightStateResponse {
                                key: req.key,
                                state: req.state,
                                brightness: req.brightness,
                                red: req.red,
                                green: req.green,
                                blue: req.blue,
                                color_temperature: req.color_temperature,
                            },
                        )
                        .await;
                }
                MSG_PING_REQUEST => {
                    let _ = tr.send(MSG_PING_RESPONSE, &PingResponse {}).await;
                }
                MSG_DISCONNECT_REQUEST => break,
                _ => {}
            }
        }
    }

    async fn verify_esphome_end_to_end(use_noise: bool) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let psk_bytes = [0x42_u8; 32];
        let psk_opt = use_noise.then_some(psk_bytes);
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);

        let server_task = tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await {
                run_mock_esphome_session(stream, psk_opt, cmd_tx).await;
            }
        });

        let history = Arc::new(SqliteStore::open(":memory:").await.unwrap());
        let bus = EventBus::new();
        let state = StateStore::new(bus.clone());
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let writer_handle = spawn_history_writer_with_config(
            Arc::clone(&history) as Arc<dyn HistoryStore>,
            &bus,
            shutdown_rx.clone(),
            64,
            Duration::from_millis(50),
        );

        let mut secrets = SecretsConfig::default();
        let psk_secret_name = if use_noise {
            secrets.set("living_psk", STANDARD.encode(psk_bytes));
            Some("living_psk".to_owned())
        } else {
            None
        };

        let nodes = vec![EspHomeNodeConfig {
            id: "esp32_living".into(),
            host: "127.0.0.1".into(),
            port,
            noise_psk_secret: psk_secret_name,
            area_id: Some("living_room".into()),
            enabled: true,
        }];

        let mgr = EspHomeManager::start_with_config(
            &nodes,
            &secrets,
            state.clone(),
            shutdown_rx,
            fast_actor_cfg(),
        );

        // Wait for entity discovery + initial state push.
        let mut ready = false;
        for _ in 0..40 {
            if let Some(temp) = state.get("sensor.esp32_living_temp")
                && matches!(
                    temp.state,
                    DomainState::Sensor {
                        value: SensorValue::Number(v),
                        ..
                    } if (v - 23.5).abs() < 0.01
                )
            {
                ready = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(ready, "expected ESPHome sensor state to arrive");

        let light = state
            .get("light.esp32_living_ceiling")
            .expect("light discovered");
        assert_eq!(light.area_id.as_deref(), Some("living_room"));
        assert_eq!(light.availability, Availability::Online);
        assert_eq!(
            light.state,
            DomainState::Light {
                on: true,
                brightness: Some(75),
                color_temp_k: Some(4000),
                rgb: None,
            }
        );

        // Dispatch a LightCommandRequest and verify mock server receives it.
        let mut params = Map::new();
        params.insert("brightness".into(), json!(50));
        params.insert("color_temp_k".into(), json!(3200));
        mgr.dispatch_command(
            "esp32_living",
            "light.esp32_living_ceiling",
            "turn_on",
            params,
        )
        .await
        .expect("dispatch command");

        let received = tokio::time::timeout(Duration::from_secs(2), cmd_rx.recv())
            .await
            .expect("command timeout")
            .expect("command received");
        assert_eq!(received.key, 101);
        assert!(received.has_state && received.state);
        assert!(received.has_brightness && (received.brightness - 0.5).abs() < 0.01);
        assert!(received.has_color_temperature);

        // Shut down cleanly and verify SQLite persisted sensor history.
        let _ = shutdown_tx.send(true);
        mgr.shutdown().await;
        writer_handle.await.unwrap();
        let _ = server_task.await;

        let series = history
            .query_entity_history(
                "sensor.esp32_living_temp",
                TimeRange::new(0, i64::MAX).unwrap(),
                None,
            )
            .await
            .expect("query history");
        match series {
            EntityHistorySeries::Numeric { unit, points, .. } => {
                assert_eq!(unit.as_deref(), Some("°C"));
                assert!(!points.is_empty());
                assert!((points[0].avg - 23.5).abs() < 0.01);
            }
            other => panic!("expected numeric history, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn esphome_plaintext_discovery_streaming_command_and_history() {
        verify_esphome_end_to_end(false).await;
    }

    #[tokio::test]
    async fn esphome_noise_nnpsk0_discovery_streaming_command_and_history() {
        verify_esphome_end_to_end(true).await;
    }

    #[tokio::test]
    async fn esphome_disconnect_marks_offline_and_reconnects_online() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (drop_tx, mut drop_rx) = mpsc::channel::<()>(1);

        let server_task = tokio::spawn(async move {
            // Session 1: complete discovery + push state, then drop TCP when signaled.
            if let Ok((stream, _)) = listener.accept().await {
                let (cmd_tx, _cmd_rx) = mpsc::channel(4);
                let s1 = tokio::spawn(run_mock_esphome_session(stream, None, cmd_tx));
                let _ = drop_rx.recv().await;
                s1.abort();
            }
            // Session 2: accept reconnect and keep alive.
            if let Ok((stream, _)) = listener.accept().await {
                let (cmd_tx, _cmd_rx) = mpsc::channel(4);
                run_mock_esphome_session(stream, None, cmd_tx).await;
            }
        });

        let bus = EventBus::new();
        let state = StateStore::new(bus);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let nodes = vec![EspHomeNodeConfig {
            id: "esp_reconn".into(),
            host: "127.0.0.1".into(),
            port,
            noise_psk_secret: None,
            area_id: None,
            enabled: true,
        }];

        let mgr = EspHomeManager::start_with_config(
            &nodes,
            &SecretsConfig::default(),
            state.clone(),
            shutdown_rx,
            fast_actor_cfg(),
        );

        // Wait for initial Online state
        for _ in 0..40 {
            if state
                .get("light.esp_reconn_ceiling")
                .is_some_and(|e| e.availability == Availability::Online)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert_eq!(
            state.get("light.esp_reconn_ceiling").unwrap().availability,
            Availability::Online
        );

        // Trigger disconnect and observe Offline -> reconnect -> Online
        let _ = drop_tx.send(()).await;
        let mut saw_offline = false;
        for _ in 0..40 {
            if state
                .get("light.esp_reconn_ceiling")
                .is_some_and(|e| e.availability == Availability::Offline)
            {
                saw_offline = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(saw_offline, "entity should transition to Offline on drop");

        let mut recovered = false;
        for _ in 0..60 {
            if state
                .get("light.esp_reconn_ceiling")
                .is_some_and(|e| e.availability == Availability::Online)
            {
                recovered = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(recovered, "entity should recover Online after reconnect");

        let _ = shutdown_tx.send(true);
        mgr.shutdown().await;
        server_task.abort();
    }

    #[test]
    fn missing_or_invalid_psk_refuses_plaintext_downgrade() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let bus = EventBus::new();
            let state = StateStore::new(bus);
            let (_tx, rx) = watch::channel(false);

            let mut secrets = SecretsConfig::default();
            secrets.set("bad_psk", "not-valid-base64!!!");

            let nodes = vec![
                EspHomeNodeConfig {
                    id: "missing_secret".into(),
                    host: "127.0.0.1".into(),
                    port: 6053,
                    noise_psk_secret: Some("nonexistent".into()),
                    area_id: None,
                    enabled: true,
                },
                EspHomeNodeConfig {
                    id: "bad_secret".into(),
                    host: "127.0.0.1".into(),
                    port: 6053,
                    noise_psk_secret: Some("bad_psk".into()),
                    area_id: None,
                    enabled: true,
                },
            ];

            let mgr = EspHomeManager::start(&nodes, &secrets, state, rx);
            assert_eq!(mgr.active_node_count(), 0);
        });
    }

    #[tokio::test]
    async fn recv_frame_is_cancel_safe_under_partial_tcp_reads() {
        use crate::esphome::transport::EspHomeTransport;

        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();

        let (part2_tx, part2_rx) = oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let msg = HelloResponse {
                api_version_major: 1,
                api_version_minor: 10,
                server_info: "split-frame-test".into(),
                name: "esp_split".into(),
            };
            let mut payload = Vec::new();
            msg.encode(&mut payload).unwrap();

            let mut frame = vec![INDICATOR_PLAINTEXT];
            encode_varint(payload.len() as u32, &mut frame);
            encode_varint(u32::from(MSG_HELLO_RESPONSE), &mut frame);
            frame.extend_from_slice(&payload);

            // Send only the first 5 bytes (indicator + len + type + partial payload).
            sock.write_all(&frame[..5]).await.unwrap();
            sock.flush().await.unwrap();

            // Wait until the client's first recv_frame future has been cancelled.
            let _ = part2_rx.await;
            sock.write_all(&frame[5..]).await.unwrap();
            sock.flush().await.unwrap();
        });

        let stream = TcpStream::connect(addr).await.unwrap();
        let mut transport = EspHomeTransport::plaintext(stream);

        // Cancel recv_frame mid-frame via select!
        tokio::select! {
            _ = transport.recv_frame() => panic!("frame should not be complete yet"),
            () = tokio::time::sleep(Duration::from_millis(30)) => {}
        }

        // Release remaining bytes and verify the full frame decodes cleanly.
        let _ = part2_tx.send(());
        let (mt, payload) = transport
            .recv_frame()
            .await
            .expect("recv_frame must survive cancellation");
        assert_eq!(mt, MSG_HELLO_RESPONSE);
        let decoded = HelloResponse::decode(payload.as_slice()).unwrap();
        assert_eq!(decoded.name, "esp_split");
        let _ = server.await;
    }

    #[tokio::test]
    async fn stalled_handshake_respects_shutdown() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();

        // Accept TCP and intentionally never reply (simulates a hung ESP32).
        let _server = tokio::spawn(async move {
            let (_sock, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(60)).await;
        });

        let bus = EventBus::new();
        let state = StateStore::new(bus);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let nodes = vec![EspHomeNodeConfig {
            id: "esp_hung".into(),
            host: "127.0.0.1".into(),
            port,
            noise_psk_secret: None,
            area_id: None,
            enabled: true,
        }];

        let mgr = EspHomeManager::start_with_config(
            &nodes,
            &SecretsConfig::default(),
            state,
            shutdown_rx,
            fast_actor_cfg(),
        );

        tokio::time::sleep(Duration::from_millis(50)).await;
        let _ = shutdown_tx.send(true);

        tokio::time::timeout(Duration::from_millis(500), mgr.shutdown())
            .await
            .expect("node actor should exit promptly on shutdown during stalled handshake");
    }

    #[tokio::test]
    async fn noise_send_message_rejects_oversized_payload_as_frame_too_large() {
        use crate::esphome::transport::EspHomeTransport;

        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let psk = [0x11_u8; 32];

        let server = tokio::spawn(async move {
            let (sock, _) = listener.accept().await.unwrap();
            let _tr = MockTransport::accept(sock, Some(psk)).await.unwrap();
            tokio::time::sleep(Duration::from_millis(200)).await;
        });

        let stream = TcpStream::connect(addr).await.unwrap();
        let mut transport = EspHomeTransport::connect_noise(stream, psk)
            .await
            .expect("noise connect");

        // 65_516 bytes exceeds Noise's 65_535 - 16 (tag) - 4 (header) = 65_515 payload limit.
        let oversized = HelloRequest {
            client_info: "x".repeat(65_516),
            api_version_major: 1,
            api_version_minor: 10,
        };
        let err = transport
            .send_message(MSG_HELLO_REQUEST, &oversized)
            .await
            .expect_err("oversized Noise payload must fail");
        assert!(
            matches!(err, EspHomeError::FrameTooLarge(_)),
            "expected FrameTooLarge, got {err:?}"
        );
        server.abort();
    }
}
