use std::collections::BTreeMap;

use serde_json::{Map, Value};
use tokio::sync::broadcast;

use crate::protocol::domain::{Availability, DomainState};
use crate::protocol::skill::SkillStatus;
use crate::storage::AutomationOutcome;

pub const DEFAULT_EVENT_BUS_CAPACITY: usize = 2048;

/// Typed in-process event published over `EventBus`.
#[derive(Debug, Clone, PartialEq)]
pub enum CoreEvent {
    EntityStateChanged {
        entity_id: String,
        area_id: Option<String>,
        old_state: Option<DomainState>,
        new_state: DomainState,
        extra: Map<String, Value>,
        at: i64,
    },
    EntityAvailabilityChanged {
        entity_id: String,
        area_id: Option<String>,
        availability: Availability,
        at: i64,
    },
    WebhookReceived {
        webhook_id: String,
        payload: Value,
        headers: BTreeMap<String, String>,
        at: i64,
    },
    SkillStatusChanged {
        skill_id: String,
        status: SkillStatus,
        message: Option<String>,
        at: i64,
    },
    AutomationRunRecorded {
        run_id: String,
        automation_id: String,
        outcome: AutomationOutcome,
        at: i64,
    },
}

/// Bounded broadcast event bus driving history persistence, automation evaluation, and SSE fan-out.
#[derive(Debug, Clone)]
pub struct EventBus {
    tx: broadcast::Sender<CoreEvent>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_EVENT_BUS_CAPACITY)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity.max(1));
        Self { tx }
    }

    /// Publish an event to all active subscribers. Returns the number of receivers notified.
    pub fn publish(&self, event: CoreEvent) -> usize {
        self.tx.send(event).unwrap_or(0)
    }

    /// Subscribe to future events on the bus.
    pub fn subscribe(&self) -> broadcast::Receiver<CoreEvent> {
        self.tx.subscribe()
    }

    pub fn receiver_count(&self) -> usize {
        self.tx.receiver_count()
    }
}
