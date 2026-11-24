use std::collections::BTreeMap;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tracing::{debug, warn};

use crate::http::AppState;
use crate::state::{CoreEvent, now_unix_ms};

/// Maximum allowed webhook request body size (64 KB, per `docs/api.md`).
pub const MAX_WEBHOOK_BODY_BYTES: usize = 64 * 1024;

#[derive(Debug, Serialize)]
pub struct WebhookAcceptedResponse {
    pub accepted: bool,
    pub webhook_id: String,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: &'static str,
}

/// Compute `sha256:<lowercase-hex>` hash of a plaintext webhook secret.
pub fn hash_webhook_secret(secret: &str) -> String {
    let digest = Sha256::digest(secret.as_bytes());
    let mut out = String::with_capacity(7 + 64);
    out.push_str("sha256:");
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

/// Verify a plaintext secret against a stored `sha256:<hex>` (or raw `<hex>`) digest in constant time.
pub fn verify_webhook_secret(provided: &str, expected_hash: &str) -> bool {
    let Some(expected_bytes) = decode_sha256_hex(expected_hash) else {
        return false;
    };
    let actual_bytes: [u8; 32] = Sha256::digest(provided.as_bytes()).into();
    bool::from(actual_bytes.ct_eq(&expected_bytes))
}

fn decode_sha256_hex(raw: &str) -> Option<[u8; 32]> {
    let trimmed = raw.trim();
    let hex = trimmed
        .strip_prefix("sha256:")
        .or_else(|| trimmed.strip_prefix("SHA256:"))
        .unwrap_or(trimmed);
    if hex.len() != 64 || !hex.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    let bytes = hex.as_bytes();
    for i in 0..32 {
        let hi = hex_nibble(bytes[i * 2])?;
        let lo = hex_nibble(bytes[i * 2 + 1])?;
        out[i] = (hi << 4) | lo;
    }
    Some(out)
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn extract_secret(headers: &HeaderMap) -> Option<&str> {
    if let Some(auth) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        let mut parts = auth.splitn(2, ' ');
        if let (Some(scheme), Some(token)) = (parts.next(), parts.next())
            && scheme.eq_ignore_ascii_case("bearer")
        {
            let token = token.trim();
            if !token.is_empty() {
                return Some(token);
            }
        }
    }

    headers
        .get("x-webhook-secret")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn sanitize_headers(headers: &HeaderMap) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (name, value) in headers {
        let key = name.as_str().to_ascii_lowercase();
        if matches!(
            key.as_str(),
            "authorization" | "proxy-authorization" | "x-webhook-secret" | "cookie"
        ) {
            continue;
        }
        if let Ok(val_str) = value.to_str() {
            out.insert(key, val_str.to_owned());
        }
    }
    out
}

/// `POST /api/webhooks/{webhook_id}`
pub async fn handle_webhook(
    State(state): State<AppState>,
    Path(webhook_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<WebhookAcceptedResponse>), (StatusCode, Json<ErrorResponse>)> {
    let Some(webhook) = state
        .webhooks
        .iter()
        .find(|w| w.id == webhook_id && w.enabled)
    else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse { error: "not_found" }),
        ));
    };

    let provided = extract_secret(&headers).ok_or_else(|| {
        warn!(webhook_id = %webhook_id, "webhook request missing secret");
        (
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                error: "unauthorized",
            }),
        )
    })?;

    if !verify_webhook_secret(provided, &webhook.secret_hash) {
        warn!(webhook_id = %webhook_id, "webhook secret verification failed");
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                error: "unauthorized",
            }),
        ));
    }

    let payload = if body.iter().all(|b| b.is_ascii_whitespace()) {
        Value::Null
    } else {
        serde_json::from_slice::<Value>(&body).map_err(|err| {
            debug!(webhook_id = %webhook_id, error = %err, "invalid webhook JSON payload");
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "invalid_json",
                }),
            )
        })?
    };

    let safe_headers = sanitize_headers(&headers);
    state.events.publish(CoreEvent::WebhookReceived {
        webhook_id: webhook_id.clone(),
        payload,
        headers: safe_headers,
        at: now_unix_ms(),
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(WebhookAcceptedResponse {
            accepted: true,
            webhook_id,
        }),
    ))
}
