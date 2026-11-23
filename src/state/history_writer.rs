use std::sync::Arc;
use std::time::Duration;

use serde_json::{Map, Value};
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::protocol::domain::{DomainState, SensorValue};
use crate::state::events::{CoreEvent, EventBus};
use crate::storage::{HistoryEntryKind, HistoryStore, StateHistoryEntry};

pub const DEFAULT_BATCH_SIZE: usize = 64;
pub const DEFAULT_FLUSH_INTERVAL: Duration = Duration::from_millis(500);
const MAX_RETRY_BUFFER_MULTIPLIER: usize = 16;

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
    let max_buffer_size = batch_size.saturating_mul(MAX_RETRY_BUFFER_MULTIPLIER);

    tokio::spawn(async move {
        let mut buffer = Vec::with_capacity(batch_size);
        let mut ticker = tokio::time::interval(flush_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                biased;
                changed_res = shutdown_rx.changed() => {
                    if changed_res.is_err() || *shutdown_rx.borrow() {
                        drain_channel(&mut rx, &mut buffer, max_buffer_size);
                        flush_buffer(&*history, &mut buffer, max_buffer_size).await;
                        break;
                    }
                }
                recv_res = rx.recv() => {
                    match recv_res {
                        Ok(ev) => {
                            if let Some(entry) = event_to_history_entry(ev) {
                                push_bounded(&mut buffer, entry, max_buffer_size);
                                if buffer.len() >= batch_size {
                                    flush_buffer(&*history, &mut buffer, max_buffer_size).await;
                                }
                            }
                        }
                        Err(RecvError::Lagged(skipped)) => {
                            warn!(skipped, "history writer lagged behind event bus");
                        }
                        Err(RecvError::Closed) => {
                            flush_buffer(&*history, &mut buffer, max_buffer_size).await;
                            break;
                        }
                    }
                }
                _ = ticker.tick() => {
                    flush_buffer(&*history, &mut buffer, max_buffer_size).await;
                }
            }
        }
    })
}

fn drain_channel(
    rx: &mut broadcast::Receiver<CoreEvent>,
    buffer: &mut Vec<StateHistoryEntry>,
    max_buffer_size: usize,
) {
    loop {
        match rx.try_recv() {
            Ok(ev) => {
                if let Some(entry) = event_to_history_entry(ev) {
                    push_bounded(buffer, entry, max_buffer_size);
                }
            }
            Err(TryRecvError::Lagged(skipped)) => {
                warn!(
                    skipped,
                    "history writer lagged behind event bus during shutdown drain"
                );
            }
            Err(TryRecvError::Empty | TryRecvError::Closed) => break,
        }
    }
}

fn push_bounded(
    buffer: &mut Vec<StateHistoryEntry>,
    entry: StateHistoryEntry,
    max_buffer_size: usize,
) {
    if buffer.len() >= max_buffer_size {
        let drop_count = buffer.len() - max_buffer_size + 1;
        buffer.drain(..drop_count);
        warn!(
            dropped = drop_count,
            max_buffer_size, "history writer retry buffer full; dropped oldest entries"
        );
    }
    buffer.push(entry);
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

async fn flush_buffer(
    history: &dyn HistoryStore,
    buffer: &mut Vec<StateHistoryEntry>,
    max_buffer_size: usize,
) {
    if buffer.is_empty() {
        return;
    }

    let count = buffer.len();
    match history.record_state_batch(buffer).await {
        Ok(()) => {
            debug!(count, "flushed state history batch");
            buffer.clear();
        }
        Err(err) => {
            warn!(
                error = %err,
                count,
                "failed to flush state history batch; retaining records for retry"
            );
            if buffer.len() > max_buffer_size {
                let drop_count = buffer.len() - max_buffer_size;
                buffer.drain(..drop_count);
            }
        }
    }
}
