pub mod events;
pub mod history_writer;
pub mod store;

pub use events::{CoreEvent, DEFAULT_EVENT_BUS_CAPACITY, EventBus};
pub use history_writer::{
    DEFAULT_BATCH_SIZE, DEFAULT_FLUSH_INTERVAL, domain_state_to_history_kind, spawn_history_writer,
    spawn_history_writer_with_config,
};
pub use store::{EntityFilter, StateStore, now_unix_ms};

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use serde_json::{Map, json};
    use tokio::sync::watch;

    use super::*;
    use crate::protocol::domain::{
        Availability, DomainState, EntityRecord, EntitySource, SensorValue,
    };
    use crate::storage::{EntityHistorySeries, HistoryStore, SqliteStore, TimeRange};

    fn sample_light(entity_id: &str, area_id: &str, node_id: &str, on: bool) -> EntityRecord {
        EntityRecord {
            entity_id: entity_id.into(),
            device_id: Some(format!("esphome:{node_id}")),
            area_id: Some(area_id.into()),
            name: entity_id.into(),
            source: EntitySource::EspHome {
                node_id: node_id.into(),
            },
            availability: Availability::Online,
            state: DomainState::Light {
                on,
                brightness: Some(100),
                color_temp_k: None,
                rgb: None,
            },
            capabilities: vec!["brightness".into()],
            extra: Map::new(),
            custom_actions: vec![],
            last_changed_at: 0,
            last_updated_at: 0,
        }
    }

    fn sample_temp_sensor(entity_id: &str, area_id: &str, value: f64) -> EntityRecord {
        EntityRecord {
            entity_id: entity_id.into(),
            device_id: Some("esphome:esp32_living".into()),
            area_id: Some(area_id.into()),
            name: "Living Temp".into(),
            source: EntitySource::EspHome {
                node_id: "esp32_living".into(),
            },
            availability: Availability::Online,
            state: DomainState::Sensor {
                value: SensorValue::Number(value),
                unit: Some("°C".into()),
                device_class: Some("temperature".into()),
            },
            capabilities: vec![],
            extra: Map::new(),
            custom_actions: vec![],
            last_changed_at: 0,
            last_updated_at: 0,
        }
    }

    #[tokio::test]
    async fn state_store_upsert_update_filter_and_source_availability() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();
        let store = StateStore::new(bus);

        let inserted = store.upsert_entity(sample_light(
            "light.living_ceiling",
            "living_room",
            "esp32_living",
            true,
        ));
        assert!(inserted.last_changed_at > 0);
        assert_eq!(inserted.last_changed_at, inserted.last_updated_at);
        assert_eq!(store.len(), 1);

        // Expect availability + state events on initial insert
        let ev1 = rx.recv().await.expect("avail event");
        let ev2 = rx.recv().await.expect("state event");
        assert!(matches!(ev1, CoreEvent::EntityAvailabilityChanged { .. }));
        assert!(matches!(ev2, CoreEvent::EntityStateChanged { .. }));

        // Updating with identical discrete state preserves last_changed_at and emits no new event
        let unchanged = store
            .update_state(
                "light.living_ceiling",
                inserted.state.clone(),
                None,
                Some(Availability::Online),
            )
            .expect("entity exists");
        assert_eq!(unchanged.last_changed_at, inserted.last_changed_at);
        assert!(rx.try_recv().is_err());

        // Changing discrete state updates last_changed_at and emits EntityStateChanged
        let updated = store
            .update_state(
                "light.living_ceiling",
                DomainState::Light {
                    on: false,
                    brightness: Some(0),
                    color_temp_k: None,
                    rgb: None,
                },
                None,
                None,
            )
            .expect("entity exists");
        assert!(updated.last_changed_at >= inserted.last_changed_at);
        let ev3 = rx.recv().await.expect("state change event");
        assert!(matches!(
            ev3,
            CoreEvent::EntityStateChanged {
                ref entity_id,
                old_state: Some(DomainState::Light { on: true, .. }),
                new_state: DomainState::Light { on: false, .. },
                ..
            } if entity_id == "light.living_ceiling"
        ));

        // Add a second entity in another area / domain and test EntityFilter
        store.upsert_entity(sample_temp_sensor("sensor.kitchen_temp", "kitchen", 22.5));
        assert_eq!(store.len(), 2);

        let living = store.list(&EntityFilter {
            area_id: Some("living_room".into()),
            ..Default::default()
        });
        assert_eq!(living.len(), 1);
        assert_eq!(living[0].entity_id, "light.living_ceiling");

        let sensors = store.list(&EntityFilter {
            domain: Some("sensor".into()),
            ..Default::default()
        });
        assert_eq!(sensors.len(), 1);
        assert_eq!(sensors[0].entity_id, "sensor.kitchen_temp");

        // Mark whole ESPHome node offline
        let changed = store.set_source_availability(
            &EntitySource::EspHome {
                node_id: "esp32_living".into(),
            },
            Availability::Offline,
        );
        assert_eq!(changed, 2);
        assert_eq!(
            store.get("light.living_ceiling").unwrap().availability,
            Availability::Offline
        );
    }

    #[tokio::test]
    async fn history_writer_batches_and_flushes_to_sqlite_on_shutdown() {
        let history = Arc::new(SqliteStore::open(":memory:").await.expect("sqlite"));
        let bus = EventBus::new();
        let store = StateStore::new(bus.clone());

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        // Large interval so we specifically verify shutdown-triggered flush!
        let handle = spawn_history_writer_with_config(
            Arc::clone(&history) as Arc<dyn HistoryStore>,
            &bus,
            shutdown_rx,
            64,
            Duration::from_secs(60),
        );

        let mut extra = Map::new();
        extra.insert("source_detail".into(), json!("manual"));

        store.upsert_entity(sample_temp_sensor(
            "sensor.living_temp",
            "living_room",
            21.5,
        ));
        store.update_state(
            "sensor.living_temp",
            DomainState::Sensor {
                value: SensorValue::Number(22.0),
                unit: Some("°C".into()),
                device_class: Some("temperature".into()),
            },
            None,
            None,
        );
        store.upsert_entity(sample_light(
            "light.living_ceiling",
            "living_room",
            "esp32_living",
            true,
        ));
        store.update_state(
            "light.living_ceiling",
            DomainState::Light {
                on: false,
                brightness: None,
                color_temp_k: None,
                rgb: None,
            },
            Some(extra),
            None,
        );

        // Trigger graceful shutdown and wait for final flush.
        let _ = shutdown_tx.send(true);
        handle.await.expect("history writer joined cleanly");

        let range = TimeRange::new(0, i64::MAX).expect("range");
        let temp_series = history
            .query_entity_history("sensor.living_temp", range, None)
            .await
            .expect("query temp");
        match temp_series {
            EntityHistorySeries::Numeric { unit, points, .. } => {
                assert_eq!(unit.as_deref(), Some("°C"));
                assert_eq!(points.len(), 2);
                assert_eq!(points[0].avg, 21.5);
                assert_eq!(points[1].avg, 22.0);
            }
            other => panic!("expected numeric series, got {other:?}"),
        }

        let light_series = history
            .query_entity_history("light.living_ceiling", range, None)
            .await
            .expect("query light");
        match light_series {
            EntityHistorySeries::Discrete { transitions, .. } => {
                assert_eq!(transitions.len(), 2);
                assert_eq!(transitions[0].state, "on");
                assert_eq!(transitions[1].state, "off");
            }
            other => panic!("expected discrete series, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn history_writer_drains_retained_events_after_broadcast_lag_on_shutdown() {
        let history = Arc::new(SqliteStore::open(":memory:").await.expect("sqlite"));
        let bus = EventBus::with_capacity(2);
        let store = StateStore::new(bus.clone());

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let handle = spawn_history_writer_with_config(
            Arc::clone(&history) as Arc<dyn HistoryStore>,
            &bus,
            shutdown_rx,
            64,
            Duration::from_secs(60),
        );

        // Publish 4 sensor samples synchronously before yielding so the capacity-2 receiver lags
        // by 2 events while 2 newer events remain retained in the channel, then signal shutdown.
        store.upsert_entity(sample_temp_sensor("sensor.lag_test", "living_room", 10.0));
        for val in [20.0, 30.0, 40.0] {
            store.update_state(
                "sensor.lag_test",
                DomainState::Sensor {
                    value: SensorValue::Number(val),
                    unit: Some("°C".into()),
                    device_class: Some("temperature".into()),
                },
                None,
                None,
            );
        }
        let _ = shutdown_tx.send(true);
        handle.await.expect("history writer joined");

        let series = history
            .query_entity_history(
                "sensor.lag_test",
                TimeRange::new(0, i64::MAX).unwrap(),
                None,
            )
            .await
            .expect("query lag_test");
        match series {
            EntityHistorySeries::Numeric { points, .. } => {
                assert_eq!(points.len(), 2, "expected 2 retained events after lag");
                assert_eq!(points[0].avg, 30.0);
                assert_eq!(points[1].avg, 40.0);
            }
            other => panic!("expected numeric series with retained events, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn history_writer_exits_and_flushes_when_shutdown_sender_is_dropped() {
        let history = Arc::new(SqliteStore::open(":memory:").await.expect("sqlite"));
        let bus = EventBus::new();
        let store = StateStore::new(bus.clone());

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let handle = spawn_history_writer_with_config(
            Arc::clone(&history) as Arc<dyn HistoryStore>,
            &bus,
            shutdown_rx,
            64,
            Duration::from_secs(60),
        );

        store.upsert_entity(sample_temp_sensor("sensor.drop_test", "living_room", 25.0));
        // Drop sender without sending `true` (e.g. startup error before serve).
        drop(shutdown_tx);

        tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .expect("history writer should exit instead of spinning on closed watch channel")
            .expect("history writer task should not panic");

        let series = history
            .query_entity_history(
                "sensor.drop_test",
                TimeRange::new(0, i64::MAX).unwrap(),
                None,
            )
            .await
            .expect("query drop_test");
        match series {
            EntityHistorySeries::Numeric { points, .. } => {
                assert_eq!(points.len(), 1);
                assert_eq!(points[0].avg, 25.0);
            }
            other => panic!("expected numeric series flushed on sender drop, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn history_writer_retains_and_retries_batch_after_transient_write_failure() {
        use std::sync::Mutex;
        use std::sync::atomic::{AtomicUsize, Ordering};

        use async_trait::async_trait;

        use crate::storage::{AutomationRunRecord, StateHistoryEntry, StorageError};

        struct FlakyStore {
            attempts: AtomicUsize,
            saved: Mutex<Vec<StateHistoryEntry>>,
        }

        #[async_trait]
        impl HistoryStore for FlakyStore {
            async fn health_check(&self) -> Result<(), StorageError> {
                Ok(())
            }

            async fn record_state_batch(
                &self,
                records: &[StateHistoryEntry],
            ) -> Result<(), StorageError> {
                let attempt = self.attempts.fetch_add(1, Ordering::SeqCst);
                if attempt == 0 {
                    return Err(StorageError::Query("transient sqlite busy".into()));
                }
                self.saved.lock().unwrap().extend_from_slice(records);
                Ok(())
            }

            async fn record_automation_run(
                &self,
                _run: &AutomationRunRecord,
            ) -> Result<(), StorageError> {
                unimplemented!()
            }

            async fn query_entity_history(
                &self,
                _entity_id: &str,
                _range: TimeRange,
                _bucket_seconds: Option<u32>,
            ) -> Result<EntityHistorySeries, StorageError> {
                unimplemented!()
            }

            async fn query_automation_runs(
                &self,
                _automation_id: Option<&str>,
                _limit: u32,
                _offset: u32,
            ) -> Result<Vec<AutomationRunRecord>, StorageError> {
                unimplemented!()
            }

            async fn prune_before(&self, _cutoff_unix_ms: i64) -> Result<u64, StorageError> {
                unimplemented!()
            }
        }

        let flaky = Arc::new(FlakyStore {
            attempts: AtomicUsize::new(0),
            saved: Mutex::new(Vec::new()),
        });
        let bus = EventBus::new();
        let store = StateStore::new(bus.clone());

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let handle = spawn_history_writer_with_config(
            Arc::clone(&flaky) as Arc<dyn HistoryStore>,
            &bus,
            shutdown_rx,
            1, // Flush immediately on 1st event (fails), then retry on 20ms tick
            Duration::from_millis(20),
        );

        store.upsert_entity(sample_temp_sensor("sensor.flaky", "living_room", 23.0));
        tokio::time::sleep(Duration::from_millis(80)).await;
        let _ = shutdown_tx.send(true);
        handle.await.expect("history writer joined");

        let saved = flaky.saved.lock().unwrap();
        assert_eq!(
            saved.len(),
            1,
            "failed batch should be retained and persisted on retry"
        );
        assert_eq!(saved[0].entity_id, "sensor.flaky");
    }

    #[test]
    fn state_store_publishes_events_in_strict_mutation_order() {
        use std::sync::Barrier;

        let bus = EventBus::with_capacity(8192);
        let mut rx = bus.subscribe();
        let store = StateStore::new(bus);

        store.upsert_entity(sample_light(
            "light.race",
            "living_room",
            "esp32_living",
            false,
        ));
        while rx.try_recv().is_ok() {}

        let num_threads = 8;
        let barrier = Arc::new(Barrier::new(num_threads));
        let mut threads = Vec::new();
        for t in 0..num_threads as u8 {
            let store = store.clone();
            let barrier = Arc::clone(&barrier);
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                for i in 0..500_u16 {
                    let brightness = t.wrapping_mul(31).wrapping_add((i % 31) as u8);
                    store.update_state(
                        "light.race",
                        DomainState::Light {
                            on: (i % 2) == 0,
                            brightness: Some(brightness),
                            color_temp_k: Some(2700 + i),
                            rgb: None,
                        },
                        None,
                        None,
                    );
                }
            }));
        }
        for th in threads {
            th.join().expect("thread joined");
        }

        // Verify every published EntityStateChanged forms a contiguous causal chain:
        // each event's old_state must equal the previous event's new_state, and the final
        // event's new_state must equal the current state stored in StateStore.
        let mut prev_new_state: Option<DomainState> = None;
        let mut count = 0usize;
        while let Ok(ev) = rx.try_recv() {
            if let CoreEvent::EntityStateChanged {
                old_state,
                new_state,
                ..
            } = ev
            {
                if let Some(expected_old) = &prev_new_state {
                    assert_eq!(
                        old_state.as_ref(),
                        Some(expected_old),
                        "out-of-order state event published after lock release"
                    );
                }
                prev_new_state = Some(new_state);
                count += 1;
            }
        }
        assert!(count > 0);
        let final_stored = store.get("light.race").unwrap().state;
        assert_eq!(prev_new_state, Some(final_stored));
    }
}
