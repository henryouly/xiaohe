use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

use crate::protocol::domain::{Availability, DomainState, EntityRecord, EntitySource, SensorValue};
use crate::state::events::{CoreEvent, EventBus};

/// Optional filter criteria for listing entities from `StateStore`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EntityFilter {
    pub area_id: Option<String>,
    pub domain: Option<String>,
    pub source: Option<EntitySource>,
}

/// Thread-safe in-memory store for all live entity records.
#[derive(Debug, Clone)]
pub struct StateStore {
    entities: Arc<RwLock<BTreeMap<String, EntityRecord>>>,
    events: EventBus,
}

impl StateStore {
    pub fn new(events: EventBus) -> Self {
        Self {
            entities: Arc::new(RwLock::new(BTreeMap::new())),
            events,
        }
    }

    pub fn events(&self) -> &EventBus {
        &self.events
    }

    /// Insert a new entity or update an existing one, stamping server-issued Unix ms timestamps
    /// and emitting `EntityStateChanged` / `EntityAvailabilityChanged` events.
    pub fn upsert_entity(&self, mut record: EntityRecord) -> EntityRecord {
        let now = now_unix_ms();
        let (state_event, avail_event, stored) = {
            let mut map = self.entities.write().expect("state store rwlock poisoned");

            let previous = map.get(&record.entity_id).cloned();
            let is_numeric_sensor = matches!(
                record.state,
                DomainState::Sensor {
                    value: SensorValue::Number(_),
                    ..
                }
            );

            let (state_changed, avail_changed, old_state) = match &previous {
                Some(prev) => {
                    let sc = prev.state != record.state;
                    let ac = prev.availability != record.availability;
                    record.last_updated_at = now;
                    record.last_changed_at = if sc { now } else { prev.last_changed_at };
                    (sc, ac, Some(prev.state.clone()))
                }
                None => {
                    record.last_updated_at = now;
                    record.last_changed_at = now;
                    (true, true, None)
                }
            };

            let stored = record.clone();
            map.insert(record.entity_id.clone(), record);

            let state_event = if state_changed || is_numeric_sensor {
                Some(CoreEvent::EntityStateChanged {
                    entity_id: stored.entity_id.clone(),
                    area_id: stored.area_id.clone(),
                    old_state,
                    new_state: stored.state.clone(),
                    extra: stored.extra.clone(),
                    at: now,
                })
            } else {
                None
            };

            let avail_event = if avail_changed {
                Some(CoreEvent::EntityAvailabilityChanged {
                    entity_id: stored.entity_id.clone(),
                    area_id: stored.area_id.clone(),
                    availability: stored.availability,
                    at: now,
                })
            } else {
                None
            };

            (state_event, avail_event, stored)
        };

        if let Some(ev) = avail_event {
            self.events.publish(ev);
        }
        if let Some(ev) = state_event {
            self.events.publish(ev);
        }

        stored
    }

    /// Update state, optional `extra` attributes, and optional `availability` on an existing entity.
    /// Returns `None` if `entity_id` is not registered.
    pub fn update_state(
        &self,
        entity_id: &str,
        new_state: DomainState,
        extra: Option<Map<String, Value>>,
        availability: Option<Availability>,
    ) -> Option<EntityRecord> {
        let now = now_unix_ms();
        let (state_event, avail_event, stored) = {
            let mut map = self.entities.write().expect("state store rwlock poisoned");
            let entry = map.get_mut(entity_id)?;

            let old_state = entry.state.clone();
            let state_changed = old_state != new_state;
            let is_numeric_sensor = matches!(
                new_state,
                DomainState::Sensor {
                    value: SensorValue::Number(_),
                    ..
                }
            );

            entry.state = new_state;
            if let Some(extra_map) = extra {
                entry.extra = extra_map;
            }
            entry.last_updated_at = now;
            if state_changed {
                entry.last_changed_at = now;
            }

            let avail_changed = if let Some(avail) = availability {
                let changed = entry.availability != avail;
                entry.availability = avail;
                changed
            } else {
                false
            };

            let stored = entry.clone();

            let state_event = if state_changed || is_numeric_sensor {
                Some(CoreEvent::EntityStateChanged {
                    entity_id: stored.entity_id.clone(),
                    area_id: stored.area_id.clone(),
                    old_state: Some(old_state),
                    new_state: stored.state.clone(),
                    extra: stored.extra.clone(),
                    at: now,
                })
            } else {
                None
            };

            let avail_event = if avail_changed {
                Some(CoreEvent::EntityAvailabilityChanged {
                    entity_id: stored.entity_id.clone(),
                    area_id: stored.area_id.clone(),
                    availability: stored.availability,
                    at: now,
                })
            } else {
                None
            };

            (state_event, avail_event, stored)
        };

        if let Some(ev) = avail_event {
            self.events.publish(ev);
        }
        if let Some(ev) = state_event {
            self.events.publish(ev);
        }

        Some(stored)
    }

    /// Update availability for every entity belonging to `source` (e.g., an ESPHome node or Skill
    /// disconnecting). Returns the number of entities whose availability changed.
    pub fn set_source_availability(
        &self,
        source: &EntitySource,
        availability: Availability,
    ) -> usize {
        let now = now_unix_ms();
        let events_to_publish: Vec<CoreEvent> = {
            let mut map = self.entities.write().expect("state store rwlock poisoned");
            let mut pending = Vec::new();

            for record in map.values_mut() {
                if &record.source == source && record.availability != availability {
                    record.availability = availability;
                    record.last_updated_at = now;
                    pending.push(CoreEvent::EntityAvailabilityChanged {
                        entity_id: record.entity_id.clone(),
                        area_id: record.area_id.clone(),
                        availability,
                        at: now,
                    });
                }
            }
            pending
        };

        let count = events_to_publish.len();
        for ev in events_to_publish {
            self.events.publish(ev);
        }
        count
    }

    pub fn get(&self, entity_id: &str) -> Option<EntityRecord> {
        let map = self.entities.read().expect("state store rwlock poisoned");
        map.get(entity_id).cloned()
    }

    pub fn list(&self, filter: &EntityFilter) -> Vec<EntityRecord> {
        let map = self.entities.read().expect("state store rwlock poisoned");
        map.values()
            .filter(|rec| {
                if let Some(area) = &filter.area_id
                    && rec.area_id.as_deref() != Some(area.as_str())
                {
                    return false;
                }
                if let Some(domain) = &filter.domain
                    && rec.state.domain_name() != domain.as_str()
                {
                    return false;
                }
                if let Some(source) = &filter.source
                    && &rec.source != source
                {
                    return false;
                }
                true
            })
            .cloned()
            .collect()
    }

    pub fn len(&self) -> usize {
        let map = self.entities.read().expect("state store rwlock poisoned");
        map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

pub fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
