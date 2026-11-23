use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use prost::Message;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::esphome::proto::encode_varint;

pub const NOISE_PATTERN: &str = "Noise_NNpsk0_25519_ChaChaPoly_SHA256";
pub const NOISE_PROLOGUE: &[u8] = b"NoiseAPIInit\0\0";
pub const INDICATOR_PLAINTEXT: u8 = 0x00;
pub const INDICATOR_NOISE: u8 = 0x01;
pub const MAX_FRAME_SIZE: usize = 65_536;

#[derive(Debug, Error)]
pub enum EspHomeError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Noise protocol error: {0}")]
    Noise(String),
    #[error("handshake failed: {0}")]
    Handshake(String),
    #[error("protobuf decode error: {0}")]
    Decode(#[from] prost::DecodeError),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("invalid Noise PSK: {0}")]
    InvalidPsk(String),
    #[error("frame size {0} exceeds maximum {MAX_FRAME_SIZE}")]
    FrameTooLarge(usize),
    #[error("invalid command: {0}")]
    InvalidCommand(String),
    #[error("unknown entity '{0}'")]
    UnknownEntity(String),
    #[error("ESPHome node '{0}' is offline")]
    NodeOffline(String),
    #[error("connection closed by peer")]
    Disconnected,
}

/// Decode a base64-encoded 32-byte ESPHome Noise PSK.
pub fn decode_psk(encoded: &str) -> Result<[u8; 32], EspHomeError> {
    let bytes = STANDARD
        .decode(encoded.trim())
        .map_err(|e| EspHomeError::InvalidPsk(format!("base64 decode failed: {e}")))?;
    bytes.try_into().map_err(|v: Vec<u8>| {
        EspHomeError::InvalidPsk(format!("expected 32 bytes, got {}", v.len()))
    })
}

/// Framed TCP transport for ESPHome Native API (plaintext or Noise `NNpsk0`).
pub enum EspHomeTransport {
    Plaintext(TcpStream),
    Noise {
        stream: TcpStream,
        state: Box<snow::TransportState>,
    },
}

impl EspHomeTransport {
    pub fn plaintext(stream: TcpStream) -> Self {
        Self::Plaintext(stream)
    }

    /// Perform the official ESPHome `Noise_NNpsk0_25519_ChaChaPoly_SHA256` handshake.
    pub async fn connect_noise(mut stream: TcpStream, psk: [u8; 32]) -> Result<Self, EspHomeError> {
        // 1. Send empty ClientHello frame: [0x01, 0x00, 0x00]
        write_noise_frame(&mut stream, &[]).await?;

        // 2. Read ServerHello frame: [0x01 (protocol), <name>\0, <mac>\0]
        let server_hello = read_noise_frame(&mut stream).await?;
        if server_hello.first().copied() != Some(INDICATOR_NOISE) {
            return Err(EspHomeError::Handshake(format!(
                "unsupported Noise server hello protocol byte: {:?}",
                server_hello.first()
            )));
        }

        // 3. Build Noise initiator and send [0x00, <noise_handshake_msg_1>]
        let params: snow::params::NoiseParams = NOISE_PATTERN
            .parse()
            .map_err(|e| EspHomeError::Noise(format!("{e}")))?;
        let mut initiator = snow::Builder::new(params)
            .prologue(NOISE_PROLOGUE)
            .psk(0, &psk)
            .build_initiator()
            .map_err(|e| EspHomeError::Noise(format!("{e}")))?;

        let mut msg1 = vec![0u8; 1024];
        let n1 = initiator
            .write_message(&[], &mut msg1)
            .map_err(|e| EspHomeError::Noise(format!("{e}")))?;
        let mut frame1 = Vec::with_capacity(1 + n1);
        frame1.push(0x00);
        frame1.extend_from_slice(&msg1[..n1]);
        write_noise_frame(&mut stream, &frame1).await?;

        // 4. Read server handshake reply: [status, <noise_handshake_msg_2 or error_utf8>]
        let server_reply = read_noise_frame(&mut stream).await?;
        if server_reply.is_empty() {
            return Err(EspHomeError::Handshake(
                "empty Noise handshake reply".into(),
            ));
        }
        if server_reply[0] != 0x00 {
            let reason = String::from_utf8_lossy(&server_reply[1..]);
            return Err(EspHomeError::Handshake(format!(
                "device rejected Noise handshake: {reason}"
            )));
        }

        let mut out = vec![0u8; 1024];
        initiator
            .read_message(&server_reply[1..], &mut out)
            .map_err(|e| EspHomeError::Noise(format!("invalid PSK or handshake reply: {e}")))?;

        let transport = initiator
            .into_transport_mode()
            .map_err(|e| EspHomeError::Noise(format!("{e}")))?;

        Ok(Self::Noise {
            stream,
            state: Box::new(transport),
        })
    }

    /// Encode and send a single Protobuf message frame.
    pub async fn send_message<M: Message>(
        &mut self,
        msg_type: u16,
        msg: &M,
    ) -> Result<(), EspHomeError> {
        let mut payload = Vec::with_capacity(msg.encoded_len());
        msg.encode(&mut payload)
            .map_err(|e| EspHomeError::Protocol(format!("protobuf encode failed: {e}")))?;

        if payload.len() > MAX_FRAME_SIZE {
            return Err(EspHomeError::FrameTooLarge(payload.len()));
        }

        match self {
            Self::Plaintext(stream) => {
                let mut frame = Vec::with_capacity(1 + 10 + payload.len());
                frame.push(INDICATOR_PLAINTEXT);
                encode_varint(payload.len() as u32, &mut frame);
                encode_varint(u32::from(msg_type), &mut frame);
                frame.extend_from_slice(&payload);
                stream.write_all(&frame).await?;
                stream.flush().await?;
                Ok(())
            }
            Self::Noise { stream, state } => {
                let payload_len = u16::try_from(payload.len())
                    .map_err(|_| EspHomeError::FrameTooLarge(payload.len()))?;
                let mut inner = Vec::with_capacity(4 + payload.len());
                inner.extend_from_slice(&msg_type.to_be_bytes());
                inner.extend_from_slice(&payload_len.to_be_bytes());
                inner.extend_from_slice(&payload);

                let mut cipher = vec![0u8; inner.len() + 64];
                let cipher_len = state
                    .write_message(&inner, &mut cipher)
                    .map_err(|e| EspHomeError::Noise(format!("encrypt failed: {e}")))?;
                write_noise_frame(stream, &cipher[..cipher_len]).await
            }
        }
    }

    /// Receive the next `(msg_type, protobuf_payload)` frame from the device.
    pub async fn recv_frame(&mut self) -> Result<(u16, Vec<u8>), EspHomeError> {
        match self {
            Self::Plaintext(stream) => {
                let indicator = read_u8_or_eof(stream).await?;
                if indicator != INDICATOR_PLAINTEXT {
                    return Err(EspHomeError::Protocol(format!(
                        "expected plaintext indicator 0x00, got 0x{indicator:02x}"
                    )));
                }
                let payload_len = read_varint(stream).await? as usize;
                if payload_len > MAX_FRAME_SIZE {
                    return Err(EspHomeError::FrameTooLarge(payload_len));
                }
                let msg_type = read_varint(stream).await?;
                let msg_type = u16::try_from(msg_type).map_err(|_| {
                    EspHomeError::Protocol(format!("invalid message type {msg_type}"))
                })?;
                let mut payload = vec![0u8; payload_len];
                stream.read_exact(&mut payload).await?;
                Ok((msg_type, payload))
            }
            Self::Noise { stream, state } => {
                let cipher = read_noise_frame(stream).await?;
                let mut inner = vec![0u8; cipher.len() + 64];
                let inner_len = state
                    .read_message(&cipher, &mut inner)
                    .map_err(|e| EspHomeError::Noise(format!("decrypt failed: {e}")))?;
                if inner_len < 4 {
                    return Err(EspHomeError::Protocol(
                        "decrypted Noise frame shorter than 4-byte header".into(),
                    ));
                }
                let msg_type = u16::from_be_bytes([inner[0], inner[1]]);
                let payload_len = u16::from_be_bytes([inner[2], inner[3]]) as usize;
                if 4 + payload_len > inner_len {
                    return Err(EspHomeError::Protocol(format!(
                        "decrypted payload length {payload_len} exceeds frame body {}",
                        inner_len - 4
                    )));
                }
                Ok((msg_type, inner[4..4 + payload_len].to_vec()))
            }
        }
    }
}

async fn read_u8_or_eof(stream: &mut TcpStream) -> Result<u8, EspHomeError> {
    let mut b = [0u8; 1];
    match stream.read_exact(&mut b).await {
        Ok(_) => Ok(b[0]),
        Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => {
            Err(EspHomeError::Disconnected)
        }
        Err(err) => Err(EspHomeError::Io(err)),
    }
}

async fn read_varint(stream: &mut TcpStream) -> Result<u32, EspHomeError> {
    let mut result: u32 = 0;
    let mut shift: u32 = 0;
    for _ in 0..5 {
        let byte = read_u8_or_eof(stream).await?;
        result |= u32::from(byte & 0x7F) << shift;
        if (byte & 0x80) == 0 {
            return Ok(result);
        }
        shift += 7;
    }
    Err(EspHomeError::Protocol("varint overflow".into()))
}

async fn write_noise_frame(stream: &mut TcpStream, body: &[u8]) -> Result<(), EspHomeError> {
    let len = u16::try_from(body.len()).map_err(|_| EspHomeError::FrameTooLarge(body.len()))?;
    let mut header = [0u8; 3];
    header[0] = INDICATOR_NOISE;
    header[1..3].copy_from_slice(&len.to_be_bytes());
    stream.write_all(&header).await?;
    if !body.is_empty() {
        stream.write_all(body).await?;
    }
    stream.flush().await?;
    Ok(())
}

async fn read_noise_frame(stream: &mut TcpStream) -> Result<Vec<u8>, EspHomeError> {
    let indicator = read_u8_or_eof(stream).await?;
    if indicator != INDICATOR_NOISE {
        return Err(EspHomeError::Protocol(format!(
            "expected Noise indicator 0x01, got 0x{indicator:02x}"
        )));
    }
    let mut len_buf = [0u8; 2];
    stream.read_exact(&mut len_buf).await?;
    let len = u16::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME_SIZE {
        return Err(EspHomeError::FrameTooLarge(len));
    }
    let mut body = vec![0u8; len];
    if len > 0 {
        stream.read_exact(&mut body).await?;
    }
    Ok(body)
}
