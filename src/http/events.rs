use std::convert::Infallible;
use std::time::Duration;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use serde_json::json;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;

use crate::http::AppState;
use crate::http::auth::AuthUser;
use crate::state::CoreEvent;

const SSE_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);

/// `GET /api/events` — Authenticated Server-Sent Events stream with 15s keep-alive,
/// `resync` lag notifications, and strict server-side RBAC filtering BEFORE serialization.
pub async fn sse_events(
    AuthUser(caller): AuthUser,
    State(state): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.events.subscribe();
    let state_store = state.state.clone();

    let stream = BroadcastStream::new(rx).filter_map(move |item| {
        match item {
            Ok(CoreEvent::EntityStateChanged {
                entity_id,
                area_id,
                new_state,
                extra,
                at,
                ..
            }) => {
                // Enforce area-scoped RBAC before serialization.
                if !caller.can_access_area(area_id.as_deref()) {
                    return None;
                }
                let snap = state_store.get(&entity_id);
                let payload = json!({
                    "entity_id": entity_id,
                    "area_id": area_id,
                    "availability": snap.as_ref().map(|e| e.availability),
                    "state": new_state,
                    "extra": extra,
                    "last_changed_at": snap.as_ref().map(|e| e.last_changed_at).unwrap_or(at),
                    "last_updated_at": snap.as_ref().map(|e| e.last_updated_at).unwrap_or(at),
                });
                Some(Ok(Event::default()
                    .event("entity_state")
                    .data(payload.to_string())))
            }
            Ok(CoreEvent::EntityAvailabilityChanged {
                entity_id,
                area_id,
                availability,
                at,
            }) => {
                if !caller.can_access_area(area_id.as_deref()) {
                    return None;
                }
                let snap = state_store.get(&entity_id);
                let payload = json!({
                    "entity_id": entity_id,
                    "area_id": area_id,
                    "availability": availability,
                    "state": snap.as_ref().map(|e| &e.state),
                    "extra": snap.as_ref().map(|e| &e.extra),
                    "last_changed_at": snap.as_ref().map(|e| e.last_changed_at).unwrap_or(at),
                    "last_updated_at": at,
                });
                Some(Ok(Event::default()
                    .event("entity_state")
                    .data(payload.to_string())))
            }
            Ok(CoreEvent::SkillStatusChanged {
                skill_id,
                status,
                message,
                at,
            }) => {
                if !caller.is_admin() {
                    return None;
                }
                let payload = json!({
                    "skill_id": skill_id,
                    "status": status,
                    "last_error": message,
                    "at": at,
                });
                Some(Ok(Event::default()
                    .event("skill_status")
                    .data(payload.to_string())))
            }
            Ok(CoreEvent::AutomationRunRecorded {
                run_id,
                automation_id,
                outcome,
                at,
            }) => {
                if !caller.is_admin() {
                    return None;
                }
                let payload = json!({
                    "run_id": run_id,
                    "automation_id": automation_id,
                    "outcome": outcome,
                    "finished_at": at,
                });
                Some(Ok(Event::default()
                    .event("automation_run")
                    .data(payload.to_string())))
            }
            Ok(CoreEvent::WebhookReceived { .. }) => None,
            Err(BroadcastStreamRecvError::Lagged(_)) => Some(Ok(Event::default()
                .event("resync")
                .data(json!({ "reason": "lagged" }).to_string()))),
        }
    });

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(SSE_KEEPALIVE_INTERVAL)
            .text("keep-alive"),
    )
}
