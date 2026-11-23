use std::sync::Arc;
use std::time::Duration;

use serde_json::{Map, Value};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::protocol::domain::{DomainState, SensorValue};
use crate::state::events::{CoreEvent, EventBus};
use crate::storage::{HistoryEntryKind, HistoryStore, StateHistoryEntry};

pub const DEFAULT_BATCH_SIZE: usize = 64;
pub const DEFAULT_FLUSH_INTERVAL: Duration = Duration::from_millis(500);

/// Convert a `DomainState` + `extra` attributes into a `HistoryEntryKind`.
pub fn domain_state_to_history_kind(
    state: &DomainState,
    extra: &Map<String, Value>,
) -> HistoryEntryKind {
    match state {
        DomainState::Sensor {
            value: SensorValue::Number(v),
            unit,
            ..
        } => HistoryEntryKind::Numeric {
            value: *v,
            unit: unit.clone(),
        },
        other => {
            let attributes = if extra.is_empty() {
                None
            } else {
                Some(Value::Object(extra.clone()))
            };
            HistoryEntryKind::Discrete {
                state: other.canonical_state_label(),
                attributes,
            }
        }
    }
}

/// Spawn the background batched history writer using default batch size (64) and flush interval (500ms).
pub fn spawn_history_writer(
    history: Arc<dyn HistoryStore>,
    events: &EventBus,
    shutdown_rx: watch::Receiver<bool>,
) -> JoinHandle<()> {
    spawn_history_writer_with_config(
        history,
        events,
        shutdown_rx,
        DEFAULT_BATCH_SIZE,
        DEFAULT_FLUSH_INTERVAL,
    )
}

/// Spawn the background batched history writer with custom batch size and flush interval.
pub fn spawn_history_writer_with_config(
    history: Arc<dyn HistoryStore>,
    events: &EventBus,
    mut shutdown_rx: watch::Receiver<bool>,
    batch_size: usize,
    flush_interval: Duration,
) -> JoinHandle<()> {
    let mut rx = events.subscribe();
    let batch_size = batch_size.max(1);

    tokio::spawn(async move {
        let mut buffer = Vec::with_capacity(batch_size);
        let mut ticker = tokio::time::interval(flush_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                biased;
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        // Drain any already-queued events before final flush.
                        while let Ok(ev) = rx.try_recv() {
                            if let Some(entry) = event_to_history_entry(ev) {
                                buffer.push(entry);
                            }
                        }
                        flush_buffer(&*history, &mut buffer).await;
                        break;
                    }
                }
                recv_res = rx.recv() => {
                    match recv_res {
                        Ok(ev) => {
                            if let Some(entry) = event_to_history_entry(ev) {
                                buffer.push(entry);
                                if buffer.len() >= batch_size {
                                    flush_buffer(&*history, &mut buffer).await;
                                }
                            }
                        }
                        Err(RecvError::Lagged(skipped)) => {
                            warn!(skipped, "history writer lagged behind event bus");
                        }
                        Err(RecvError::Closed) => {
                            flush_buffer(&*history, &mut buffer).await;
                            break;
                        }
                    }
                }
                _ = ticker.tick() => {
                    flush_buffer(&*history, &mut buffer).await;
                }
            }
        }
    })
}

fn event_to_history_entry(event: CoreEvent) -> Option<StateHistoryEntry> {
    match event {
        CoreEvent::EntityStateChanged {
            entity_id,
            new_state,
            extra,
            at,
            ..
        } => Some(StateHistoryEntry {
            entity_id,
            ts: at,
            kind: domain_state_to_history_kind(&new_state, &extra),
        }),
        _ => None,
    }
}

async fn flush_buffer(history: &dyn HistoryStore, buffer: &mut Vec<StateHistoryEntry>) {
    if buffer.is_empty() {
        return;
    }

    let count = buffer.len();
    if let Err(err) = history.record_state_batch(buffer).await {
        warn!(error = %err, count, "failed to flush state history batch");
    } else {
        debug!(count, "flushed state history batch");
    }
    buffer.clear();
}
