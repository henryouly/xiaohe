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

    /// Register entity metadata discovered from a protocol (e.g., ESPHome `ListEntities` or Skill
    /// `entity.register`) without emitting a synthetic `EntityStateChanged` for placeholder state.
    ///
    /// If the entity already exists (e.g., on reconnect), its metadata (`name`, `device_id`,
    /// `area_id`, `capabilities`) is refreshed while preserving its last known `state`.
    pub fn register_entity(&self, mut record: EntityRecord) -> EntityRecord {
        let mut map = self.entities.write().expect("state store rwlock poisoned");
        let now = now_unix_ms();

        if let Some(existing) = map.get_mut(&record.entity_id) {
            let avail_changed = existing.availability != record.availability;
            existing.device_id = record.device_id;
            existing.area_id = record.area_id;
            existing.name = record.name;
            existing.source = record.source;
            existing.capabilities = record.capabilities;
            existing.custom_actions = record.custom_actions;
            existing.availability = record.availability;
            existing.last_updated_at = now;
            let stored = existing.clone();
            if avail_changed {
                self.events.publish(CoreEvent::EntityAvailabilityChanged {
                    entity_id: stored.entity_id.clone(),
                    area_id: stored.area_id.clone(),
                    availability: stored.availability,
                    at: now,
                });
            }
            return stored;
        }

        record.last_updated_at = now;
        record.last_changed_at = now;
        let stored = record.clone();
        map.insert(record.entity_id.clone(), record);

        self.events.publish(CoreEvent::EntityAvailabilityChanged {
            entity_id: stored.entity_id.clone(),
            area_id: stored.area_id.clone(),
            availability: stored.availability,
            at: now,
        });

        stored
    }

    /// Insert a new entity or update an existing one, stamping server-issued Unix ms timestamps
    /// and emitting `EntityStateChanged` / `EntityAvailabilityChanged` events in mutation order.
    pub fn upsert_entity(&self, mut record: EntityRecord) -> EntityRecord {
        let mut map = self.entities.write().expect("state store rwlock poisoned");
        let now = now_unix_ms();

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

        if avail_changed {
            self.events.publish(CoreEvent::EntityAvailabilityChanged {
                entity_id: stored.entity_id.clone(),
                area_id: stored.area_id.clone(),
                availability: stored.availability,
                at: now,
            });
        }
        if state_changed || is_numeric_sensor {
            self.events.publish(CoreEvent::EntityStateChanged {
                entity_id: stored.entity_id.clone(),
                area_id: stored.area_id.clone(),
                old_state,
                new_state: stored.state.clone(),
                extra: stored.extra.clone(),
                at: now,
            });
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
        let mut map = self.entities.write().expect("state store rwlock poisoned");
        let entry = map.get_mut(entity_id)?;
        let now = now_unix_ms();

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

        if avail_changed {
            self.events.publish(CoreEvent::EntityAvailabilityChanged {
                entity_id: stored.entity_id.clone(),
                area_id: stored.area_id.clone(),
                availability: stored.availability,
                at: now,
            });
        }
        if state_changed || is_numeric_sensor {
            self.events.publish(CoreEvent::EntityStateChanged {
                entity_id: stored.entity_id.clone(),
                area_id: stored.area_id.clone(),
                old_state: Some(old_state),
                new_state: stored.state.clone(),
                extra: stored.extra.clone(),
                at: now,
            });
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
        let mut map = self.entities.write().expect("state store rwlock poisoned");
        let now = now_unix_ms();
        let mut count = 0;

        for record in map.values_mut() {
            if &record.source == source && record.availability != availability {
                record.availability = availability;
                record.last_updated_at = now;
                self.events.publish(CoreEvent::EntityAvailabilityChanged {
                    entity_id: record.entity_id.clone(),
                    area_id: record.area_id.clone(),
                    availability,
                    at: now,
                });
                count += 1;
            }
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
