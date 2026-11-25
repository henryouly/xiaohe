use std::collections::HashSet;

use serde::{Deserialize, Serialize};

fn default_widget_size() -> WidgetSize {
    WidgetSize::Md
}

fn default_window_hours() -> u32 {
    24
}

/// Responsive 12-column grid span for a Dashboard widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WidgetSize {
    /// 4 of 12 columns on desktop (1/3 width).
    Sm,
    /// 6 of 12 columns on desktop (1/2 width).
    #[default]
    Md,
    /// 12 of 12 columns on desktop (full width).
    Lg,
}

/// Discriminated configuration for each supported Dashboard widget type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WidgetConfig {
    /// Large numeric readout for a single sensor entity with optional thresholds.
    Metric {
        entity_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        accent: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        warn_above: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        crit_above: Option<f64>,
    },
    /// Inline SVG trend sparkline backed by `HistoryStore` numeric telemetry.
    Sparkline {
        entity_id: String,
        #[serde(default = "default_window_hours")]
        window_hours: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        accent: Option<String>,
    },
    /// Full interactive domain control card pinned to a single entity.
    EntityCard { entity_id: String },
    /// Weather banner aggregating `sensor.weather_*` entities.
    Weather {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prefix: Option<String>,
    },
    /// Compact multi-row status table filtered by area, domain, or explicit entity IDs.
    EntityList {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        area_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        domain: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        entity_ids: Vec<String>,
    },
    /// Area-grouped entity cards (either a single fixed area or all areas with filter pills).
    AreaGroup {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        area_id: Option<String>,
    },
    /// Live internal platform overview (SSE status, online/offline entities, skills, automations).
    SystemStatus {},
}

/// Single widget instance placed on the Dashboard grid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DashboardWidget {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default = "default_widget_size")]
    pub size: WidgetSize,
    #[serde(flatten)]
    pub config: WidgetConfig,
}

/// Persistent Dashboard layout stored in `config/dashboard.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DashboardConfig {
    #[serde(default)]
    pub widgets: Vec<DashboardWidget>,
}

impl DashboardConfig {
    /// Starter layout used when `config/dashboard.toml` has not been customized yet.
    pub fn default_layout() -> Self {
        Self {
            widgets: vec![
                DashboardWidget {
                    id: "default-weather".into(),
                    title: None,
                    size: WidgetSize::Lg,
                    config: WidgetConfig::Weather { prefix: None },
                },
                DashboardWidget {
                    id: "default-system-status".into(),
                    title: Some("System Overview".into()),
                    size: WidgetSize::Lg,
                    config: WidgetConfig::SystemStatus {},
                },
                DashboardWidget {
                    id: "default-all-areas".into(),
                    title: Some("Devices & Sensors".into()),
                    size: WidgetSize::Lg,
                    config: WidgetConfig::AreaGroup { area_id: None },
                },
            ],
        }
    }

    /// Validate widget IDs and widget-specific configuration constraints.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        let mut seen_ids = HashSet::new();

        for (idx, widget) in self.widgets.iter().enumerate() {
            let id = widget.id.trim();
            if id.is_empty() {
                errors.push(format!("widgets[{idx}]: id must not be empty"));
            } else if !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            {
                errors.push(format!(
                    "widgets[{idx}]: invalid id '{id}' (must match [A-Za-z0-9_-]+)"
                ));
            } else if !seen_ids.insert(id.to_string()) {
                errors.push(format!("widgets[{idx}]: duplicate widget id '{id}'"));
            }

            match &widget.config {
                WidgetConfig::Metric {
                    entity_id,
                    warn_above,
                    crit_above,
                    ..
                } => {
                    if entity_id.trim().is_empty() {
                        errors.push(format!("widget '{id}': entity_id must not be empty"));
                    }
                    if let Some(w) = warn_above
                        && !w.is_finite()
                    {
                        errors.push(format!("widget '{id}': warn_above must be finite"));
                    }
                    if let Some(c) = crit_above
                        && !c.is_finite()
                    {
                        errors.push(format!("widget '{id}': crit_above must be finite"));
                    }
                }
                WidgetConfig::Sparkline {
                    entity_id,
                    window_hours,
                    ..
                } => {
                    if entity_id.trim().is_empty() {
                        errors.push(format!("widget '{id}': entity_id must not be empty"));
                    }
                    if *window_hours == 0 || *window_hours > 720 {
                        errors.push(format!(
                            "widget '{id}': window_hours ({window_hours}) must be within 1..=720"
                        ));
                    }
                }
                WidgetConfig::EntityCard { entity_id } => {
                    if entity_id.trim().is_empty() {
                        errors.push(format!("widget '{id}': entity_id must not be empty"));
                    }
                }
                WidgetConfig::Weather { .. }
                | WidgetConfig::EntityList { .. }
                | WidgetConfig::AreaGroup { .. }
                | WidgetConfig::SystemStatus {} => {}
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}
