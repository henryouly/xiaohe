pub mod engine;
pub mod schedule;
pub mod schema;
pub mod validator;

pub use engine::{
    ActionExecutor, AutomationEngine, CoreActionExecutor, evaluate_conditions,
    extract_numeric_value,
};
pub use schedule::{CronSchedule, compute_sun_time_utc, parse_wall_time, time_in_window};
pub use schema::{
    ActionSpec, AutomationConfig, ConditionSpec, ExecutionMode, SunEvent, TriggerSpec, Weekday,
};
pub use validator::{
    AutomationError, LoadedAutomation, ValidationContext, ValidationWarning, load_automation_file,
    load_automations_dir, validate_automation,
};

/// Generate a time-ordered UUIDv7 identifier for an automation run.
pub fn new_run_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use chrono::{NaiveDate, NaiveTime, TimeZone, Timelike, Utc};
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn parses_and_validates_complete_delegation_automation_from_spec() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("smart_climate.toml");
        fs::write(
            &file,
            r#"
id = "smart_living_climate"
name = "Smart Living Room Climate"
description = "Delegate heat-index cooling curve to climate-optimizer"
enabled = true
mode = "single"

[[triggers]]
type = "numeric_state"
entity_id = "sensor.living_temp"
above = 26.0
for_secs = 180

[[triggers]]
type = "webhook"
webhook_id = "manual_boost"

[[conditions]]
type = "state"
entity_id = "binary_sensor.living_window"
equals = "off"

[[conditions]]
type = "or"
conditions = [
  { type = "time", after = "22:00:00", before = "06:30:00", weekdays = ["mon", "fri"] },
  { type = "sun", after = "sunset", after_offset_minutes = -15 },
]

[[actions]]
type = "skill.invoke"
skill_id = "climate-optimizer"
action = "evaluate_comfort"
timeout_secs = 15
context_entities = ["sensor.living_temp", "sensor.living_humidity"]
[actions.params]
area_id = "living_room"

[[actions]]
type = "delay"
secs = 10

[[actions]]
type = "parallel"
steps = [
  { type = "entity.call", entity_id = "light.living_ceiling", action = "turn_off" },
  { type = "entity.call", entity_id = "switch.purifier_power", action = "turn_on" },
]
"#,
        )
        .unwrap();

        let mut ctx = ValidationContext::default();
        ctx.known_entities.insert("sensor.living_temp".into());
        ctx.known_skills.insert("climate-optimizer".into());

        let (loaded, errors) = load_automations_dir(dir.path(), Some(&ctx));
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].config.id, "smart_living_climate");
        assert_eq!(loaded[0].config.mode, ExecutionMode::Single);
        assert_eq!(loaded[0].config.triggers.len(), 2);
        assert_eq!(loaded[0].config.conditions.len(), 2);
        assert_eq!(loaded[0].config.actions.len(), 3);
        // Unregistered entities produce non-blocking warnings rather than hard errors.
        assert!(
            loaded[0]
                .warnings
                .iter()
                .any(|w| w.message.contains("binary_sensor.living_window"))
        );
    }

    #[test]
    fn rejects_invalid_schemas_and_duplicate_ids_without_blocking_healthy_rules() {
        let dir = TempDir::new().unwrap();

        // Healthy rule
        fs::write(
            dir.path().join("01_good.toml"),
            r#"
id = "good_rule"
name = "Good Rule"

[[triggers]]
type = "time"
cron = "0 */15 * * * *"

[[actions]]
type = "entity.call"
entity_id = "switch.hall"
action = "turn_on"
"#,
        )
        .unwrap();

        // Duplicate id
        fs::write(
            dir.path().join("02_dup.toml"),
            r#"
id = "good_rule"
name = "Duplicate Rule"

[[triggers]]
type = "webhook"
webhook_id = "dup"

[[actions]]
type = "delay"
secs = 1
"#,
        )
        .unwrap();

        // Invalid time + invalid entity_id + impossible numeric_state condition
        fs::write(
            dir.path().join("03_bad.toml"),
            r#"
id = "BAD ID!"
name = ""

[[triggers]]
type = "time"
at = "25:99:00"

[[conditions]]
type = "numeric_state"
entity_id = "not_an_entity"
above = 30.0
below = 10.0

[[actions]]
type = "delay"
secs = 0
"#,
        )
        .unwrap();

        let (loaded, errors) = load_automations_dir(dir.path(), None);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].config.id, "good_rule");
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn cron_window_and_solar_schedule_helpers_behave_deterministically() {
        let sched = CronSchedule::parse("0 */15 8-18 * * 1-5").expect("valid cron");
        // Monday 2026-11-23 09:30:00 UTC -> matches
        let mon_match = Utc.with_ymd_and_hms(2026, 11, 23, 9, 30, 0).unwrap();
        assert!(sched.matches(&mon_match));
        // Monday 2026-11-23 09:30:05 UTC -> second != 0
        let mon_bad_sec = Utc.with_ymd_and_hms(2026, 11, 23, 9, 30, 5).unwrap();
        assert!(!sched.matches(&mon_bad_sec));
        // Sunday 2026-11-22 09:30:00 UTC -> dow = 0 not in 1-5
        let sun = Utc.with_ymd_and_hms(2026, 11, 22, 9, 30, 0).unwrap();
        assert!(!sched.matches(&sun));

        // Midnight wrap-around window: [22:00:00, 06:30:00)
        let after = parse_wall_time("22:00:00").unwrap();
        let before = parse_wall_time("06:30:00").unwrap();
        assert!(time_in_window(
            NaiveTime::from_hms_opt(23, 15, 0).unwrap(),
            Some(after),
            Some(before)
        ));
        assert!(time_in_window(
            NaiveTime::from_hms_opt(2, 0, 0).unwrap(),
            Some(after),
            Some(before)
        ));
        assert!(!time_in_window(
            NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
            Some(after),
            Some(before)
        ));

        // Equator equinox sunrise ~06:00 UTC, sunset ~18:00 UTC at lon=0
        let equinox = NaiveDate::from_ymd_opt(2026, 3, 21).unwrap();
        let sr = compute_sun_time_utc(equinox, 0.0, 0.0, SunEvent::Sunrise).unwrap();
        let ss = compute_sun_time_utc(equinox, 0.0, 0.0, SunEvent::Sunset).unwrap();
        assert!(sr.hour() == 5 || sr.hour() == 6, "unexpected sunrise: {sr}");
        assert!(
            ss.hour() == 17 || ss.hour() == 18,
            "unexpected sunset: {ss}"
        );
    }

    type SkillInvokeLog = (String, String, serde_json::Map<String, serde_json::Value>);

    struct RecordingExecutor {
        calls: std::sync::Mutex<Vec<(String, String)>>,
        skill_invokes: std::sync::Mutex<Vec<SkillInvokeLog>>,
        call_delay: std::time::Duration,
    }

    impl RecordingExecutor {
        fn new(call_delay: std::time::Duration) -> Self {
            Self {
                calls: std::sync::Mutex::new(Vec::new()),
                skill_invokes: std::sync::Mutex::new(Vec::new()),
                call_delay,
            }
        }

        fn calls(&self) -> Vec<(String, String)> {
            self.calls.lock().unwrap().clone()
        }

        fn skill_invokes(&self) -> Vec<SkillInvokeLog> {
            self.skill_invokes.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl ActionExecutor for RecordingExecutor {
        async fn call_entity(
            &self,
            entity_id: &str,
            action: &str,
            _params: serde_json::Map<String, serde_json::Value>,
        ) -> Result<serde_json::Value, String> {
            if !self.call_delay.is_zero() {
                tokio::time::sleep(self.call_delay).await;
            }
            self.calls
                .lock()
                .unwrap()
                .push((entity_id.to_owned(), action.to_owned()));
            Ok(serde_json::json!({ "ok": true }))
        }

        async fn invoke_skill(
            &self,
            skill_id: &str,
            action: &str,
            _params: serde_json::Map<String, serde_json::Value>,
            context: serde_json::Map<String, serde_json::Value>,
            _timeout: std::time::Duration,
        ) -> Result<serde_json::Value, String> {
            self.skill_invokes.lock().unwrap().push((
                skill_id.to_owned(),
                action.to_owned(),
                context,
            ));
            Ok(serde_json::json!({ "comfort": "hot" }))
        }
    }

    fn make_binary_sensor(entity_id: &str, active: bool) -> crate::protocol::domain::EntityRecord {
        crate::protocol::domain::EntityRecord {
            entity_id: entity_id.into(),
            device_id: Some("dev".into()),
            area_id: Some("living_room".into()),
            name: entity_id.into(),
            source: crate::protocol::domain::EntitySource::EspHome {
                node_id: "test_node".into(),
            },
            availability: crate::protocol::domain::Availability::Online,
            state: crate::protocol::domain::DomainState::BinarySensor {
                active,
                device_class: Some("motion".into()),
            },
            capabilities: vec![],
            extra: serde_json::Map::new(),
            custom_actions: vec![],
            last_changed_at: 0,
            last_updated_at: 0,
        }
    }

    fn make_numeric_sensor(entity_id: &str, value: f64) -> crate::protocol::domain::EntityRecord {
        crate::protocol::domain::EntityRecord {
            entity_id: entity_id.into(),
            device_id: Some("dev".into()),
            area_id: Some("living_room".into()),
            name: entity_id.into(),
            source: crate::protocol::domain::EntitySource::EspHome {
                node_id: "test_node".into(),
            },
            availability: crate::protocol::domain::Availability::Online,
            state: crate::protocol::domain::DomainState::Sensor {
                value: crate::protocol::domain::SensorValue::Number(value),
                unit: Some("°C".into()),
                device_class: Some("temperature".into()),
            },
            capabilities: vec![],
            extra: serde_json::Map::new(),
            custom_actions: vec![],
            last_changed_at: 0,
            last_updated_at: 0,
        }
    }

    #[tokio::test]
    async fn debounce_state_trigger_cancels_on_bounce_and_fires_when_held() {
        use crate::storage::{AutomationOutcome, HistoryStore, SqliteStore};
        use std::sync::Arc;
        use std::time::Duration;

        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("motion.toml"),
            r#"
id = "motion_debounce"
name = "Motion Debounce"

[[triggers]]
type = "state"
entity_id = "binary_sensor.motion"
from = "off"
to = "on"
for_secs = 1

[[actions]]
type = "entity.call"
entity_id = "light.hall"
action = "turn_on"
"#,
        )
        .unwrap();

        let history = Arc::new(SqliteStore::open(":memory:").await.unwrap());
        let events = crate::state::EventBus::new();
        let state = crate::state::StateStore::new(events.clone());
        state.upsert_entity(make_binary_sensor("binary_sensor.motion", false));

        let exec = Arc::new(RecordingExecutor::new(Duration::ZERO));
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let (engine, errs) = AutomationEngine::start(
            dir.path(),
            crate::config::LocationConfig::default(),
            state.clone(),
            events,
            Arc::clone(&history) as Arc<dyn HistoryStore>,
            Arc::clone(&exec) as Arc<dyn ActionExecutor>,
            shutdown_rx,
        );
        assert!(errs.is_empty());

        // Bounce: off -> on -> off within 200ms (must cancel debounce timer!)
        state.update_state(
            "binary_sensor.motion",
            crate::protocol::domain::DomainState::BinarySensor {
                active: true,
                device_class: Some("motion".into()),
            },
            None,
            None,
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
        state.update_state(
            "binary_sensor.motion",
            crate::protocol::domain::DomainState::BinarySensor {
                active: false,
                device_class: Some("motion".into()),
            },
            None,
            None,
        );
        tokio::time::sleep(Duration::from_millis(1100)).await;
        assert!(
            exec.calls().is_empty(),
            "bounced motion should not fire debounced trigger"
        );

        // Hold: off -> on for > 1s (must fire once and record Succeeded in SQLite)
        state.update_state(
            "binary_sensor.motion",
            crate::protocol::domain::DomainState::BinarySensor {
                active: true,
                device_class: Some("motion".into()),
            },
            None,
            None,
        );
        tokio::time::sleep(Duration::from_millis(1300)).await;
        assert_eq!(
            exec.calls(),
            vec![("light.hall".to_owned(), "turn_on".to_owned())]
        );

        let runs = history
            .query_automation_runs(Some("motion_debounce"), 10, 0)
            .await
            .unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].outcome, AutomationOutcome::Succeeded);

        let _ = shutdown_tx.send(true);
        engine.shutdown().await;
    }

    #[tokio::test]
    async fn numeric_state_trigger_is_edge_triggered_and_evaluates_conditions() {
        use crate::storage::{AutomationOutcome, HistoryStore, SqliteStore};
        use std::sync::Arc;
        use std::time::Duration;

        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("temp_guard.toml"),
            r#"
id = "temp_guard"
name = "Temp Guard"

[[triggers]]
type = "numeric_state"
entity_id = "sensor.temp"
above = 26.0

[[conditions]]
type = "state"
entity_id = "binary_sensor.window"
equals = "off"

[[actions]]
type = "parallel"
steps = [
  { type = "entity.call", entity_id = "switch.fan", action = "turn_on" },
  { type = "skill.invoke", skill_id = "climate-optimizer", action = "evaluate_comfort", context_entities = ["sensor.temp"] },
]
"#,
        )
        .unwrap();

        let history = Arc::new(SqliteStore::open(":memory:").await.unwrap());
        let events = crate::state::EventBus::new();
        let state = crate::state::StateStore::new(events.clone());
        state.upsert_entity(make_numeric_sensor("sensor.temp", 24.0));
        // Window is initially OPEN (on) -> condition should fail
        state.upsert_entity(make_binary_sensor("binary_sensor.window", true));

        let exec = Arc::new(RecordingExecutor::new(Duration::ZERO));
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let (engine, errs) = AutomationEngine::start(
            dir.path(),
            crate::config::LocationConfig::default(),
            state.clone(),
            events,
            Arc::clone(&history) as Arc<dyn HistoryStore>,
            Arc::clone(&exec) as Arc<dyn ActionExecutor>,
            shutdown_rx,
        );
        assert!(errs.is_empty());

        // Cross 24.0 -> 27.0 while window is open -> ConditionsNotMet
        state.update_state(
            "sensor.temp",
            crate::protocol::domain::DomainState::Sensor {
                value: crate::protocol::domain::SensorValue::Number(27.0),
                unit: Some("°C".into()),
                device_class: Some("temperature".into()),
            },
            None,
            None,
        );
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(exec.calls().is_empty());

        // Stay above threshold (27.0 -> 28.0) -> MUST NOT re-fire (edge-triggered!)
        state.update_state(
            "sensor.temp",
            crate::protocol::domain::DomainState::Sensor {
                value: crate::protocol::domain::SensorValue::Number(28.0),
                unit: Some("°C".into()),
                device_class: Some("temperature".into()),
            },
            None,
            None,
        );
        tokio::time::sleep(Duration::from_millis(150)).await;
        let runs_after_stay = history
            .query_automation_runs(Some("temp_guard"), 10, 0)
            .await
            .unwrap();
        assert_eq!(runs_after_stay.len(), 1);
        assert_eq!(
            runs_after_stay[0].outcome,
            AutomationOutcome::ConditionsNotMet
        );

        // Close window, drop below 26.0, then cross above again -> Succeeded!
        state.update_state(
            "binary_sensor.window",
            crate::protocol::domain::DomainState::BinarySensor {
                active: false,
                device_class: Some("window".into()),
            },
            None,
            None,
        );
        state.update_state(
            "sensor.temp",
            crate::protocol::domain::DomainState::Sensor {
                value: crate::protocol::domain::SensorValue::Number(25.0),
                unit: Some("°C".into()),
                device_class: Some("temperature".into()),
            },
            None,
            None,
        );
        state.update_state(
            "sensor.temp",
            crate::protocol::domain::DomainState::Sensor {
                value: crate::protocol::domain::SensorValue::Number(26.5),
                unit: Some("°C".into()),
                device_class: Some("temperature".into()),
            },
            None,
            None,
        );
        tokio::time::sleep(Duration::from_millis(200)).await;

        assert_eq!(
            exec.calls(),
            vec![("switch.fan".to_owned(), "turn_on".to_owned())]
        );
        let invokes = exec.skill_invokes();
        assert_eq!(invokes.len(), 1);
        assert_eq!(invokes[0].0, "climate-optimizer");
        assert_eq!(
            invokes[0].2["entities"]["sensor.temp"]["value"],
            serde_json::json!(26.5)
        );

        let runs = history
            .query_automation_runs(Some("temp_guard"), 10, 0)
            .await
            .unwrap();
        assert_eq!(runs.len(), 2);
        assert!(
            runs.iter()
                .any(|r| r.outcome == AutomationOutcome::Succeeded)
        );

        let _ = shutdown_tx.send(true);
        engine.shutdown().await;
    }

    #[tokio::test]
    async fn concurrency_modes_single_restart_queued_and_parallel() {
        use crate::storage::{AutomationOutcome, HistoryStore, SqliteStore};
        use std::sync::Arc;
        use std::time::Duration;

        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("01_single.toml"),
            r#"
id = "mode_single"
name = "Single Mode"
mode = "single"

[[triggers]]
type = "webhook"
webhook_id = "w_single"

[[actions]]
type = "delay"
secs = 1

[[actions]]
type = "entity.call"
entity_id = "switch.single"
action = "turn_on"
"#,
        )
        .unwrap();

        fs::write(
            dir.path().join("02_restart.toml"),
            r#"
id = "mode_restart"
name = "Restart Mode"
mode = "restart"

[[triggers]]
type = "webhook"
webhook_id = "w_restart"

[[actions]]
type = "delay"
secs = 1

[[actions]]
type = "entity.call"
entity_id = "switch.restart"
action = "turn_on"
"#,
        )
        .unwrap();

        fs::write(
            dir.path().join("03_queued.toml"),
            r#"
id = "mode_queued"
name = "Queued Mode"
mode = "queued"

[[triggers]]
type = "webhook"
webhook_id = "w_queued"

[[actions]]
type = "entity.call"
entity_id = "switch.queued"
action = "turn_on"
"#,
        )
        .unwrap();

        fs::write(
            dir.path().join("04_parallel.toml"),
            r#"
id = "mode_parallel"
name = "Parallel Mode"
mode = "parallel"
max_parallel = 3

[[triggers]]
type = "webhook"
webhook_id = "w_parallel"

[[actions]]
type = "entity.call"
entity_id = "switch.parallel"
action = "turn_on"
"#,
        )
        .unwrap();

        let history = Arc::new(SqliteStore::open(":memory:").await.unwrap());
        let events = crate::state::EventBus::new();
        let state = crate::state::StateStore::new(events.clone());
        let exec = Arc::new(RecordingExecutor::new(Duration::from_millis(80)));
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        let (engine, errs) = AutomationEngine::start(
            dir.path(),
            crate::config::LocationConfig::default(),
            state,
            events,
            Arc::clone(&history) as Arc<dyn HistoryStore>,
            Arc::clone(&exec) as Arc<dyn ActionExecutor>,
            shutdown_rx,
        );
        assert!(errs.is_empty());

        // 1. Single: trigger twice quickly -> 1 SkippedSingle + 1 Succeeded
        engine.trigger_manual("mode_single").await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        engine.trigger_manual("mode_single").await.unwrap();

        // 2. Restart: trigger twice quickly -> 1 Cancelled + 1 Succeeded
        engine.trigger_manual("mode_restart").await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        engine.trigger_manual("mode_restart").await.unwrap();

        // 3. Queued: trigger twice -> 2 Succeeded sequentially
        engine.trigger_manual("mode_queued").await.unwrap();
        engine.trigger_manual("mode_queued").await.unwrap();

        // 4. Parallel: trigger 3 times -> 3 Succeeded concurrently
        engine.trigger_manual("mode_parallel").await.unwrap();
        engine.trigger_manual("mode_parallel").await.unwrap();
        engine.trigger_manual("mode_parallel").await.unwrap();

        tokio::time::sleep(Duration::from_millis(1400)).await;

        let single_runs = history
            .query_automation_runs(Some("mode_single"), 10, 0)
            .await
            .unwrap();
        assert_eq!(single_runs.len(), 2);
        assert!(
            single_runs
                .iter()
                .any(|r| r.outcome == AutomationOutcome::SkippedSingle)
        );
        assert!(
            single_runs
                .iter()
                .any(|r| r.outcome == AutomationOutcome::Succeeded)
        );

        let restart_runs = history
            .query_automation_runs(Some("mode_restart"), 10, 0)
            .await
            .unwrap();
        assert_eq!(restart_runs.len(), 2);
        assert!(
            restart_runs
                .iter()
                .any(|r| r.outcome == AutomationOutcome::Cancelled)
        );
        assert!(
            restart_runs
                .iter()
                .any(|r| r.outcome == AutomationOutcome::Succeeded)
        );

        let queued_runs = history
            .query_automation_runs(Some("mode_queued"), 10, 0)
            .await
            .unwrap();
        assert_eq!(queued_runs.len(), 2);
        assert!(
            queued_runs
                .iter()
                .all(|r| r.outcome == AutomationOutcome::Succeeded)
        );

        let parallel_runs = history
            .query_automation_runs(Some("mode_parallel"), 10, 0)
            .await
            .unwrap();
        assert_eq!(parallel_runs.len(), 3);
        assert!(
            parallel_runs
                .iter()
                .all(|r| r.outcome == AutomationOutcome::Succeeded)
        );

        let _ = shutdown_tx.send(true);
        engine.shutdown().await;
    }
}
