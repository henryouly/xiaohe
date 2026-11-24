use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::automation::schedule::{CronSchedule, parse_wall_time};
use crate::automation::schema::{ActionSpec, AutomationConfig, ConditionSpec, TriggerSpec};

/// Error produced when reading, parsing, or statically validating an automation file.
#[derive(Debug, Error)]
pub enum AutomationError {
    #[error("I/O error reading {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse automation TOML {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("validation failed for {path}: {reasons:?}")]
    Validation { path: PathBuf, reasons: Vec<String> },
}

/// Non-blocking diagnostic warning (e.g. referenced entity or skill is currently offline/unregistered).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationWarning {
    pub automation_id: String,
    pub message: String,
}

/// Optional live household inventory used to emit non-blocking warnings during validation.
#[derive(Debug, Default)]
pub struct ValidationContext {
    pub known_entities: HashSet<String>,
    pub known_skills: HashSet<String>,
}

/// A parsed and statically validated automation together with its source path and warnings.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedAutomation {
    pub path: PathBuf,
    pub config: AutomationConfig,
    pub warnings: Vec<ValidationWarning>,
}

/// Load and statically validate all `*.toml` files under `automations_dir`.
///
/// Isolates broken files and duplicate `id`s so healthy automations still load cleanly.
pub fn load_automations_dir(
    automations_dir: &Path,
    ctx: Option<&ValidationContext>,
) -> (Vec<LoadedAutomation>, Vec<AutomationError>) {
    let mut loaded = Vec::new();
    let mut errors = Vec::new();

    let entries = match fs::read_dir(automations_dir) {
        Ok(e) => e,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return (loaded, errors),
        Err(source) => {
            errors.push(AutomationError::Io {
                path: automations_dir.to_path_buf(),
                source,
            });
            return (loaded, errors);
        }
    };

    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|ext| ext == "toml"))
        .collect();
    files.sort();

    let mut seen_ids = HashSet::new();
    for path in files {
        match load_automation_file(&path, ctx) {
            Ok(auto) => {
                if !seen_ids.insert(auto.config.id.clone()) {
                    errors.push(AutomationError::Validation {
                        path,
                        reasons: vec![format!("duplicate automation id '{}'", auto.config.id)],
                    });
                    continue;
                }
                loaded.push(auto);
            }
            Err(err) => errors.push(err),
        }
    }

    (loaded, errors)
}

/// Load and statically validate a single automation `.toml` file.
pub fn load_automation_file(
    path: &Path,
    ctx: Option<&ValidationContext>,
) -> Result<LoadedAutomation, AutomationError> {
    let raw = fs::read_to_string(path).map_err(|source| AutomationError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let config: AutomationConfig =
        toml::from_str(&raw).map_err(|source| AutomationError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
    match validate_automation(&config, ctx) {
        Ok(warnings) => Ok(LoadedAutomation {
            path: path.to_path_buf(),
            config,
            warnings,
        }),
        Err(reasons) => Err(AutomationError::Validation {
            path: path.to_path_buf(),
            reasons,
        }),
    }
}

/// Statically validate an `AutomationConfig`.
///
/// Returns `Ok(warnings)` on success or `Err(reasons)` when hard schema violations exist.
pub fn validate_automation(
    auto: &AutomationConfig,
    ctx: Option<&ValidationContext>,
) -> Result<Vec<ValidationWarning>, Vec<String>> {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    if auto.id.is_empty()
        || !auto
            .id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
    {
        errors.push("id must be non-empty lowercase ASCII [a-z0-9_-]".to_owned());
    }
    if auto.name.trim().is_empty() {
        errors.push("name must not be empty".to_owned());
    }
    if auto.max_queued == 0 {
        errors.push("max_queued must be >= 1".to_owned());
    }
    if auto.max_parallel == 0 {
        errors.push("max_parallel must be >= 1".to_owned());
    }
    if auto.triggers.is_empty() {
        errors.push("automation must declare at least one [[triggers]]".to_owned());
    }
    if auto.actions.is_empty() {
        errors.push("automation must declare at least one [[actions]]".to_owned());
    }

    for (idx, trigger) in auto.triggers.iter().enumerate() {
        validate_trigger(trigger, idx, &auto.id, ctx, &mut errors, &mut warnings);
    }
    for (idx, cond) in auto.conditions.iter().enumerate() {
        validate_condition(
            cond,
            &format!("conditions[{idx}]"),
            &auto.id,
            ctx,
            &mut errors,
            &mut warnings,
        );
    }
    for (idx, action) in auto.actions.iter().enumerate() {
        validate_action(
            action,
            &format!("actions[{idx}]"),
            &auto.id,
            ctx,
            &mut errors,
            &mut warnings,
        );
    }

    if errors.is_empty() {
        Ok(warnings)
    } else {
        Err(errors)
    }
}

fn validate_trigger(
    trigger: &TriggerSpec,
    idx: usize,
    auto_id: &str,
    ctx: Option<&ValidationContext>,
    errors: &mut Vec<String>,
    warnings: &mut Vec<ValidationWarning>,
) {
    let prefix = format!("triggers[{idx}]");
    match trigger {
        TriggerSpec::State { entity_id, .. } => {
            check_entity_id(entity_id, &prefix, auto_id, ctx, errors, warnings);
        }
        TriggerSpec::NumericState {
            entity_id,
            above,
            below,
            ..
        } => {
            check_entity_id(entity_id, &prefix, auto_id, ctx, errors, warnings);
            if above.is_none() && below.is_none() {
                errors.push(format!(
                    "{prefix}: numeric_state trigger requires at least one of 'above' or 'below'"
                ));
            }
            if above.is_some_and(|v| !v.is_finite()) || below.is_some_and(|v| !v.is_finite()) {
                errors.push(format!(
                    "{prefix}: numeric_state thresholds must be finite numbers"
                ));
            }
        }
        TriggerSpec::Time { at, cron, .. } => match (at, cron) {
            (Some(t), None) => {
                if let Err(e) = parse_wall_time(t) {
                    errors.push(format!("{prefix}: {e}"));
                }
            }
            (None, Some(c)) => {
                if let Err(e) = CronSchedule::parse(c) {
                    errors.push(format!("{prefix}: {e}"));
                }
            }
            (Some(_), Some(_)) => {
                errors.push(format!(
                    "{prefix}: time trigger cannot specify both 'at' and 'cron'"
                ));
            }
            (None, None) => {
                errors.push(format!(
                    "{prefix}: time trigger must specify either 'at' or 'cron'"
                ));
            }
        },
        TriggerSpec::Sun { offset_minutes, .. } => {
            if !(-720..=720).contains(offset_minutes) {
                errors.push(format!(
                    "{prefix}: sun offset_minutes must be within [-720, 720]"
                ));
            }
        }
        TriggerSpec::Webhook { webhook_id } => {
            if webhook_id.trim().is_empty() {
                errors.push(format!("{prefix}: webhook_id must not be empty"));
            }
        }
        TriggerSpec::Event { event_type } => {
            if event_type.trim().is_empty() {
                errors.push(format!("{prefix}: event_type must not be empty"));
            }
        }
    }
}

fn validate_condition(
    cond: &ConditionSpec,
    path: &str,
    auto_id: &str,
    ctx: Option<&ValidationContext>,
    errors: &mut Vec<String>,
    warnings: &mut Vec<ValidationWarning>,
) {
    match cond {
        ConditionSpec::State { entity_id, equals } => {
            check_entity_id(entity_id, path, auto_id, ctx, errors, warnings);
            if equals.trim().is_empty() {
                errors.push(format!(
                    "{path}: state condition 'equals' must not be empty"
                ));
            }
        }
        ConditionSpec::NumericState {
            entity_id,
            above,
            below,
        } => {
            check_entity_id(entity_id, path, auto_id, ctx, errors, warnings);
            if above.is_none() && below.is_none() {
                errors.push(format!(
                    "{path}: numeric_state condition requires at least one of 'above' or 'below'"
                ));
            }
            if let (Some(a), Some(b)) = (above, below)
                && a >= b
            {
                errors.push(format!(
                    "{path}: numeric_state condition 'above' ({a}) must be < 'below' ({b})"
                ));
            }
        }
        ConditionSpec::Time {
            after,
            before,
            weekdays,
        } => {
            if after.is_none() && before.is_none() && weekdays.is_empty() {
                errors.push(format!(
                    "{path}: time condition must specify 'after', 'before', or 'weekdays'"
                ));
            }
            if let Some(a) = after
                && let Err(e) = parse_wall_time(a)
            {
                errors.push(format!("{path}.after: {e}"));
            }
            if let Some(b) = before
                && let Err(e) = parse_wall_time(b)
            {
                errors.push(format!("{path}.before: {e}"));
            }
        }
        ConditionSpec::Sun { after, before, .. } => {
            if after.is_none() && before.is_none() {
                errors.push(format!(
                    "{path}: sun condition must specify 'after' and/or 'before'"
                ));
            }
        }
        ConditionSpec::And { conditions } | ConditionSpec::Or { conditions } => {
            if conditions.is_empty() {
                errors.push(format!("{path}: logical combinator must not be empty"));
            }
            for (i, sub) in conditions.iter().enumerate() {
                validate_condition(sub, &format!("{path}[{i}]"), auto_id, ctx, errors, warnings);
            }
        }
        ConditionSpec::Not {
            condition,
            conditions,
        } => {
            if condition.is_none() && conditions.is_empty() {
                errors.push(format!(
                    "{path}: 'not' condition must specify 'condition' or 'conditions'"
                ));
            }
            if let Some(c) = condition {
                validate_condition(
                    c,
                    &format!("{path}.condition"),
                    auto_id,
                    ctx,
                    errors,
                    warnings,
                );
            }
            for (i, sub) in conditions.iter().enumerate() {
                validate_condition(sub, &format!("{path}[{i}]"), auto_id, ctx, errors, warnings);
            }
        }
    }
}

fn validate_action(
    action: &ActionSpec,
    path: &str,
    auto_id: &str,
    ctx: Option<&ValidationContext>,
    errors: &mut Vec<String>,
    warnings: &mut Vec<ValidationWarning>,
) {
    match action {
        ActionSpec::EntityCall {
            entity_id,
            action: cmd,
            ..
        } => {
            check_entity_id(entity_id, path, auto_id, ctx, errors, warnings);
            if cmd.trim().is_empty() {
                errors.push(format!("{path}: entity.call 'action' must not be empty"));
            }
        }
        ActionSpec::SkillInvoke {
            skill_id,
            action: skill_action,
            timeout_secs,
            context_entities,
            ..
        } => {
            if skill_id.trim().is_empty() {
                errors.push(format!("{path}: skill.invoke 'skill_id' must not be empty"));
            } else if let Some(c) = ctx
                && !c.known_skills.contains(skill_id)
            {
                warnings.push(ValidationWarning {
                    automation_id: auto_id.to_owned(),
                    message: format!("{path}: referenced skill '{skill_id}' is not registered"),
                });
            }
            if skill_action.trim().is_empty() {
                errors.push(format!("{path}: skill.invoke 'action' must not be empty"));
            }
            if timeout_secs.is_some_and(|t| t == 0) {
                errors.push(format!("{path}: skill.invoke 'timeout_secs' must be >= 1"));
            }
            for ent in context_entities {
                check_entity_id(ent, path, auto_id, ctx, errors, warnings);
            }
        }
        ActionSpec::Delay { secs } => {
            if *secs == 0 {
                errors.push(format!("{path}: delay 'secs' must be >= 1"));
            }
        }
        ActionSpec::Parallel { steps } => {
            if steps.is_empty() {
                errors.push(format!(
                    "{path}: parallel action must contain at least one step"
                ));
            }
            for (i, step) in steps.iter().enumerate() {
                validate_action(
                    step,
                    &format!("{path}.steps[{i}]"),
                    auto_id,
                    ctx,
                    errors,
                    warnings,
                );
            }
        }
    }
}

fn check_entity_id(
    entity_id: &str,
    path: &str,
    auto_id: &str,
    ctx: Option<&ValidationContext>,
    errors: &mut Vec<String>,
    warnings: &mut Vec<ValidationWarning>,
) {
    let valid_format = entity_id.split_once('.').is_some_and(|(domain, slug)| {
        !domain.is_empty() && !slug.is_empty() && !slug.contains('.')
    });
    if !valid_format {
        errors.push(format!(
            "{path}: invalid entity_id '{entity_id}' (expected '<domain>.<slug>')"
        ));
        return;
    }
    if let Some(c) = ctx
        && !c.known_entities.contains(entity_id)
    {
        warnings.push(ValidationWarning {
            automation_id: auto_id.to_owned(),
            message: format!(
                "{path}: referenced entity '{entity_id}' is currently unregistered/offline"
            ),
        });
    }
}
