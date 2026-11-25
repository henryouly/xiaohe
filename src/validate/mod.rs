use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::automation::{ValidationContext, load_automations_dir};
use crate::config::{AllowedAreas, ConfigStore, StorageBackend};
use crate::supervisor::manifest::scan_skills_dir;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticLevel {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    pub level: DiagnosticLevel,
    pub source: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ValidationReport {
    pub config_dir: String,
    pub skills_dir: Option<String>,
    pub checked_files: usize,
    pub automations_checked: usize,
    pub skills_checked: usize,
    pub diagnostics: Vec<Diagnostic>,
}

impl ValidationReport {
    pub fn error_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.level == DiagnosticLevel::Error)
            .count()
    }

    pub fn warning_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.level == DiagnosticLevel::Warning)
            .count()
    }

    pub fn is_valid(&self) -> bool {
        self.error_count() == 0
    }

    fn error(&mut self, source: impl Into<String>, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic {
            level: DiagnosticLevel::Error,
            source: source.into(),
            message: message.into(),
        });
    }

    fn warn(&mut self, source: impl Into<String>, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic {
            level: DiagnosticLevel::Warning,
            source: source.into(),
            message: message.into(),
        });
    }
}

/// Offline validator for Xiaohe TOML configuration, declarative automations, and optional Skill manifests.
pub fn validate_workspace(
    config_dir: impl AsRef<Path>,
    skills_dir: Option<&Path>,
) -> ValidationReport {
    let config_dir = config_dir.as_ref();
    let store = ConfigStore::new(config_dir);
    let mut report = ValidationReport {
        config_dir: config_dir.display().to_string(),
        skills_dir: skills_dir.map(|p| p.display().to_string()),
        ..Default::default()
    };

    if !config_dir.exists() {
        report.error(
            config_dir.display().to_string(),
            "config directory does not exist",
        );
        return report;
    }

    // 1. system.toml
    let system_path = config_dir.join("system.toml");
    if system_path.exists() {
        report.checked_files += 1;
    }
    match store.load_system() {
        Ok(mut system) => {
            system.apply_env_overrides();
            if system.server.port == 0 {
                report.error("system.toml", "server.port must be > 0");
            }
            if let Some(lat) = system.location.latitude
                && (!lat.is_finite() || !(-90.0..=90.0).contains(&lat))
            {
                report.error(
                    "system.toml",
                    format!("location.latitude ({lat}) must be within [-90, 90]"),
                );
            }
            if let Some(lon) = system.location.longitude
                && (!lon.is_finite() || !(-180.0..=180.0).contains(&lon))
            {
                report.error(
                    "system.toml",
                    format!("location.longitude ({lon}) must be within [-180, 180]"),
                );
            }
            if system.storage.backend == StorageBackend::InfluxDb {
                match &system.storage.influxdb {
                    None => report.error(
                        "system.toml",
                        "storage.backend is 'influxdb' but [storage.influxdb] is missing",
                    ),
                    Some(cfg) => {
                        let url = cfg.url.trim();
                        if !url.starts_with("http://") && !url.starts_with("https://") {
                            report.error(
                                "system.toml",
                                format!("storage.influxdb.url '{url}' must start with http:// or https://"),
                            );
                        }
                        let has_bucket = cfg
                            .bucket
                            .as_deref()
                            .or(cfg.database.as_deref())
                            .is_some_and(|s| !s.trim().is_empty());
                        if !has_bucket {
                            report.error(
                                "system.toml",
                                "storage.influxdb.bucket (or database) must be non-empty",
                            );
                        }
                    }
                }
            }
            let mut hook_ids = HashSet::new();
            for hook in &system.webhooks {
                if hook.id.trim().is_empty() {
                    report.error("system.toml", "webhook id must not be empty");
                } else if !hook_ids.insert(hook.id.clone()) {
                    report.error("system.toml", format!("duplicate webhook id '{}'", hook.id));
                }
                let raw_hex = hook
                    .secret_hash
                    .strip_prefix("sha256:")
                    .unwrap_or(&hook.secret_hash);
                if raw_hex.len() != 64 || !raw_hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                    report.error(
                        "system.toml",
                        format!(
                            "webhook '{}' secret_hash must be a 64-character hex SHA-256 digest (optional 'sha256:' prefix)",
                            hook.id
                        ),
                    );
                }
            }
        }
        Err(err) => report.error("system.toml", err.to_string()),
    }

    // 2. areas.toml
    let areas_path = config_dir.join("areas.toml");
    if areas_path.exists() {
        report.checked_files += 1;
    }
    let mut known_areas = HashSet::new();
    match store.load_areas() {
        Ok(areas) => {
            for area in &areas.areas {
                let valid_id = !area.area_id.is_empty()
                    && area.area_id.bytes().all(|b| {
                        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-'
                    });
                if !valid_id {
                    report.error(
                        "areas.toml",
                        format!("invalid area_id '{}': must match [a-z0-9_-]+", area.area_id),
                    );
                } else if !known_areas.insert(area.area_id.clone()) {
                    report.error(
                        "areas.toml",
                        format!("duplicate area_id '{}'", area.area_id),
                    );
                }
                if area.name.trim().is_empty() {
                    report.error(
                        "areas.toml",
                        format!("area '{}' must have a non-empty name", area.area_id),
                    );
                }
            }
        }
        Err(err) => report.error("areas.toml", err.to_string()),
    }

    // 2b. dashboard.toml (optional)
    let dashboard_path = config_dir.join("dashboard.toml");
    if dashboard_path.exists() {
        report.checked_files += 1;
        match store.load_dashboard() {
            Ok(dashboard) => {
                if let Err(errors) = dashboard.validate() {
                    for msg in errors {
                        report.error("dashboard.toml", msg);
                    }
                }
            }
            Err(err) => report.error("dashboard.toml", err.to_string()),
        }
    }

    // 3. devices.toml
    let devices_path = config_dir.join("devices.toml");
    if devices_path.exists() {
        report.checked_files += 1;
    }
    match store.load_devices() {
        Ok(devices) => {
            let mut node_ids = HashSet::new();
            for node in &devices.esphome.nodes {
                if node.id.trim().is_empty() {
                    report.error("devices.toml", "esphome node id must not be empty");
                } else if !node_ids.insert(node.id.clone()) {
                    report.error(
                        "devices.toml",
                        format!("duplicate esphome node id '{}'", node.id),
                    );
                }
                if node.host.trim().is_empty() {
                    report.error(
                        "devices.toml",
                        format!("esphome node '{}' must have a non-empty host", node.id),
                    );
                }
            }
        }
        Err(err) => report.error("devices.toml", err.to_string()),
    }

    // 4. secrets.toml
    let secrets_path = config_dir.join("secrets.toml");
    if secrets_path.exists() {
        report.checked_files += 1;
        check_sensitive_file_mode(&secrets_path, "secrets.toml", &mut report);
    }
    if let Err(err) = store.load_secrets() {
        report.error("secrets.toml", err.to_string());
    }

    // 5. users.toml
    let users_path = config_dir.join("users.toml");
    if users_path.exists() {
        report.checked_files += 1;
        check_sensitive_file_mode(&users_path, "users.toml", &mut report);
    }
    match store.load_users() {
        Ok(users) => {
            let mut usernames = HashSet::new();
            for user in &users.users {
                let lower = user.username.to_ascii_lowercase();
                if !usernames.insert(lower) {
                    report.error(
                        "users.toml",
                        format!("duplicate username '{}'", user.username),
                    );
                }
                if !known_areas.is_empty()
                    && let AllowedAreas::List(list) = &user.allowed_areas
                {
                    for area_id in list {
                        if !known_areas.contains(area_id) {
                            report.warn(
                                "users.toml",
                                format!(
                                    "user '{}' references unknown area '{}'",
                                    user.username, area_id
                                ),
                            );
                        }
                    }
                }
            }
        }
        Err(err) => report.error("users.toml", err.to_string()),
    }

    // 6. Optional skills directory manifests
    let mut ctx = ValidationContext::default();
    if let Some(sdir) = skills_dir {
        if !sdir.is_dir() {
            report.error(
                sdir.display().to_string(),
                "skills directory does not exist or is not a directory",
            );
        } else {
            let (skills, errors) = scan_skills_dir(sdir);
            report.skills_checked = skills.len() + errors.len();
            for skill in &skills {
                ctx.known_skills.insert(skill.manifest.skill.id.clone());
                check_skill_markdown(&skill.dir, &skill.manifest.skill.id, &mut report);
            }
            for err in errors {
                report.error(sdir.display().to_string(), err.to_string());
            }
        }
    }

    // 7. automations/*.toml
    let auto_dir = config_dir.join("automations");
    if auto_dir.is_dir() {
        let ctx_opt = if ctx.known_skills.is_empty() {
            None
        } else {
            Some(&ctx)
        };
        let (loaded, errors) = load_automations_dir(&auto_dir, ctx_opt);
        report.automations_checked = loaded.len() + errors.len();
        for item in loaded {
            for warning in item.warnings {
                report.warn(item.path.display().to_string(), warning.message);
            }
        }
        for err in errors {
            report.error(auto_dir.display().to_string(), err.to_string());
        }
    }

    report
}

#[cfg(unix)]
fn check_sensitive_file_mode(path: &Path, label: &str, report: &mut ValidationReport) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        let mode = meta.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            report.warn(
                label,
                format!(
                    "file permissions are {:04o}; recommend 0600 to protect credentials",
                    mode
                ),
            );
        }
    }
}

#[cfg(not(unix))]
fn check_sensitive_file_mode(_path: &Path, _label: &str, _report: &mut ValidationReport) {}

fn strip_yaml_scalar(raw: &str) -> &str {
    let trimmed = raw.trim();
    if trimmed.len() >= 2
        && ((trimmed.starts_with('"') && trimmed.ends_with('"'))
            || (trimmed.starts_with('\'') && trimmed.ends_with('\'')))
    {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    }
}

fn check_skill_markdown(skill_dir: &Path, expected_id: &str, report: &mut ValidationReport) {
    let md_path = skill_dir.join("SKILL.md");
    let source = md_path.display().to_string();
    if !md_path.is_file() {
        report.warn(
            source,
            "missing SKILL.md guide with YAML frontmatter (name, description)",
        );
        return;
    }

    let raw = match std::fs::read_to_string(&md_path) {
        Ok(content) => content,
        Err(err) => {
            report.error(source, format!("failed to read SKILL.md: {err}"));
            return;
        }
    };
    let content = raw.strip_prefix('\u{feff}').unwrap_or(&raw);
    let mut lines = content.lines();
    if lines.next().map(str::trim) != Some("---") {
        report.error(
            source,
            "SKILL.md must begin with YAML frontmatter delimited by '---'",
        );
        return;
    }

    let mut closed = false;
    let mut name: Option<String> = None;
    let mut description: Option<String> = None;

    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            closed = true;
            break;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = trimmed.split_once(':') {
            let key = key.trim();
            let val = strip_yaml_scalar(value).trim();
            match key {
                "name" => name = Some(val.to_string()),
                "description" => description = Some(val.to_string()),
                _ => {}
            }
        }
    }

    if !closed {
        report.error(
            source,
            "SKILL.md YAML frontmatter is missing closing '---' delimiter",
        );
        return;
    }

    match name.as_deref() {
        None | Some("") => {
            report.error(
                source.clone(),
                "SKILL.md frontmatter is missing required 'name' field",
            );
        }
        Some(actual) if actual != expected_id => {
            report.error(
                source.clone(),
                format!(
                    "SKILL.md frontmatter name '{actual}' does not match skill.toml id '{expected_id}'"
                ),
            );
        }
        Some(_) => {}
    }

    if description.as_deref().is_none_or(|d| d.trim().is_empty()) {
        report.error(
            source,
            "SKILL.md frontmatter is missing required 'description' field",
        );
    }
}

/// Format a `ValidationReport` for CLI stdout (either human-readable or JSON).
pub fn format_report(report: &ValidationReport, json: bool) -> String {
    if json {
        return serde_json::to_string_pretty(report).unwrap_or_else(|_| "{}".into());
    }

    let mut out = format!(
        "Xiaohe validation: {} config file(s), {} automation(s), {} skill(s)\n",
        report.checked_files, report.automations_checked, report.skills_checked
    );
    for d in &report.diagnostics {
        let tag = match d.level {
            DiagnosticLevel::Error => "ERROR",
            DiagnosticLevel::Warning => "WARN ",
        };
        out.push_str(&format!("  [{tag}] {}: {}\n", d.source, d.message));
    }
    if report.is_valid() {
        out.push_str(&format!("OK ({} warning(s))\n", report.warning_count()));
    } else {
        out.push_str(&format!(
            "FAILED ({} error(s), {} warning(s))\n",
            report.error_count(),
            report.warning_count()
        ));
    }
    out
}

/// Resolve default `config_dir` / `skills_dir` from CLI flags or env vars.
pub fn resolve_dirs(
    config_dir: Option<PathBuf>,
    skills_dir: Option<PathBuf>,
) -> (PathBuf, Option<PathBuf>) {
    let cfg = config_dir.unwrap_or_else(|| {
        std::env::var("CONFIG_DIR")
            .ok()
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(crate::core::DEFAULT_CONFIG_DIR))
    });
    let skills = skills_dir.or_else(|| {
        std::env::var("SKILLS_DIR")
            .ok()
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    });
    (cfg, skills)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_clean_workspace_and_detects_broken_configs() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let auto_dir = dir.path().join("automations");
        std::fs::create_dir_all(&auto_dir).expect("mkdir");

        // Valid system.toml, areas.toml, and automation
        std::fs::write(
            dir.path().join("system.toml"),
            r#"
[server]
host = "127.0.0.1"
port = 4000

[location]
latitude = 39.9
longitude = 116.4
"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("areas.toml"),
            r#"
[[areas]]
area_id = "living_room"
name = "Living Room"
"#,
        )
        .unwrap();
        std::fs::write(
            auto_dir.join("ok.toml"),
            r#"
id = "ok_rule"
name = "OK Rule"
enabled = true

[[triggers]]
type = "webhook"
webhook_id = "h1"

[[actions]]
type = "entity.call"
entity_id = "light.living"
action = "turn_on"
"#,
        )
        .unwrap();

        let clean = validate_workspace(dir.path(), None);
        assert!(
            clean.is_valid(),
            "unexpected errors: {:?}",
            clean.diagnostics
        );
        assert_eq!(clean.automations_checked, 1);

        // Inject invalid influxdb config, duplicate area_id, and broken automation
        std::fs::write(
            dir.path().join("system.toml"),
            r#"
[storage]
backend = "influxdb"
"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("areas.toml"),
            r#"
[[areas]]
area_id = "BAD AREA"
name = "Bad"
"#,
        )
        .unwrap();
        std::fs::write(auto_dir.join("bad.toml"), "id = \"broken\"\n").unwrap();

        let broken = validate_workspace(dir.path(), None);
        assert!(!broken.is_valid());
        assert!(broken.error_count() >= 3, "got: {:?}", broken.diagnostics);

        let json_out = format_report(&broken, true);
        let parsed: serde_json::Value = serde_json::from_str(&json_out).expect("valid json");
        assert!(parsed["diagnostics"].as_array().unwrap().len() >= 3);

        // Regression: explicit nonexistent skills_dir must report an error
        let missing_skills = dir.path().join("does_not_exist");
        let rep = validate_workspace(dir.path(), Some(&missing_skills));
        assert!(!rep.is_valid());
        assert!(
            rep.diagnostics
                .iter()
                .any(|d| d.level == DiagnosticLevel::Error
                    && d.message.contains("skills directory does not exist"))
        );
    }

    #[test]
    fn validates_skill_markdown_frontmatter() {
        let cfg = tempfile::TempDir::new().expect("cfg tempdir");
        let skills_root = tempfile::TempDir::new().expect("skills tempdir");
        let skill_dir = skills_root.path().join("demo-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("skill.toml"),
            r#"
[skill]
id = "demo-skill"
name = "Demo Skill"
version = "0.1.0"
mode = "device"
description = "Demo"

[runtime]
type = "executable"
entrypoint = ["/bin/true"]
"#,
        )
        .unwrap();

        // Missing SKILL.md -> warning only, still valid
        let rep_missing = validate_workspace(cfg.path(), Some(skills_root.path()));
        assert!(rep_missing.is_valid(), "got: {:?}", rep_missing.diagnostics);
        assert_eq!(rep_missing.warning_count(), 1);

        // Missing frontmatter -> error
        std::fs::write(skill_dir.join("SKILL.md"), "# Demo Skill\n").unwrap();
        let rep_no_fm = validate_workspace(cfg.path(), Some(skills_root.path()));
        assert!(!rep_no_fm.is_valid());

        // Mismatched name & missing description -> 2 errors
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: wrong-id\n---\n# Demo Skill\n",
        )
        .unwrap();
        let rep_bad = validate_workspace(cfg.path(), Some(skills_root.path()));
        assert_eq!(rep_bad.error_count(), 2, "got: {:?}", rep_bad.diagnostics);

        // Valid frontmatter -> 0 errors, 0 warnings
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: demo-skill\ndescription: \"Valid demo skill.\"\n---\n\n# Demo Skill\n",
        )
        .unwrap();
        let rep_ok = validate_workspace(cfg.path(), Some(skills_root.path()));
        assert!(rep_ok.is_valid(), "got: {:?}", rep_ok.diagnostics);
        assert_eq!(rep_ok.warning_count(), 0);
    }
}
