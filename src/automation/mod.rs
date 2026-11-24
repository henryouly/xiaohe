pub mod schedule;
pub mod schema;
pub mod validator;

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
}
