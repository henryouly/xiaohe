use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Declarative, expression-free TOML automation definition (`config/automations/*.toml`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutomationConfig {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub mode: ExecutionMode,
    #[serde(default = "default_max_queued")]
    pub max_queued: usize,
    #[serde(default = "default_max_parallel")]
    pub max_parallel: usize,
    #[serde(default)]
    pub triggers: Vec<TriggerSpec>,
    #[serde(default)]
    pub conditions: Vec<ConditionSpec>,
    #[serde(default)]
    pub actions: Vec<ActionSpec>,
}

/// Concurrency mode when a rule is triggered while a previous run is still active.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    #[default]
    Single,
    Restart,
    Queued,
    Parallel,
}

/// Day-of-week filter for `time` triggers and conditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Weekday {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

impl Weekday {
    pub fn from_chrono(wd: chrono::Weekday) -> Self {
        match wd {
            chrono::Weekday::Mon => Self::Mon,
            chrono::Weekday::Tue => Self::Tue,
            chrono::Weekday::Wed => Self::Wed,
            chrono::Weekday::Thu => Self::Thu,
            chrono::Weekday::Fri => Self::Fri,
            chrono::Weekday::Sat => Self::Sat,
            chrono::Weekday::Sun => Self::Sun,
        }
    }
}

/// Solar event kind for `sun` triggers and conditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SunEvent {
    Sunrise,
    Sunset,
}

/// Trigger definitions (`[[triggers]]`, evaluated with OR semantics).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TriggerSpec {
    /// Fires on discrete entity state transition, optionally debounced via `for_secs`.
    State {
        entity_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<String>,
        #[serde(default)]
        for_secs: u64,
    },
    /// Edge-triggered numeric threshold crossing, optionally debounced via `for_secs`.
    NumericState {
        entity_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        above: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        below: Option<f64>,
        #[serde(default)]
        for_secs: u64,
    },
    /// Fixed wall-clock time (`at = "HH:MM:SS"`) or cron expression (`cron = "0 */15 * * * *"`).
    Time {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cron: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        weekdays: Vec<Weekday>,
    },
    /// Solar trigger (`sunrise` | `sunset`) with signed minute offset.
    Sun {
        event: SunEvent,
        #[serde(default)]
        offset_minutes: i32,
    },
    /// Incoming HTTP webhook (`POST /api/webhooks/{webhook_id}`).
    Webhook { webhook_id: String },
    /// Generic CoreEvent trigger (e.g. `"skill_status_changed"`, `"entity_availability_changed"`).
    Event { event_type: String },
}

/// Condition definitions (`[[conditions]]`, top-level evaluated with AND semantics).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConditionSpec {
    State {
        entity_id: String,
        equals: String,
    },
    NumericState {
        entity_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        above: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        below: Option<f64>,
    },
    Time {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        weekdays: Vec<Weekday>,
    },
    Sun {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<SunEvent>,
        #[serde(default)]
        after_offset_minutes: i32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before: Option<SunEvent>,
        #[serde(default)]
        before_offset_minutes: i32,
    },
    And {
        conditions: Vec<ConditionSpec>,
    },
    Or {
        conditions: Vec<ConditionSpec>,
    },
    Not {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        condition: Option<Box<ConditionSpec>>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        conditions: Vec<ConditionSpec>,
    },
}

/// Action steps (`[[actions]]`, executed sequentially unless wrapped in `parallel`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ActionSpec {
    #[serde(rename = "entity.call")]
    EntityCall {
        entity_id: String,
        action: String,
        #[serde(default)]
        params: Map<String, Value>,
    },
    #[serde(rename = "skill.invoke")]
    SkillInvoke {
        skill_id: String,
        action: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_secs: Option<u64>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        context_entities: Vec<String>,
        #[serde(default)]
        params: Map<String, Value>,
    },
    #[serde(rename = "delay")]
    Delay { secs: u64 },
    #[serde(rename = "parallel")]
    Parallel { steps: Vec<ActionSpec> },
}

fn default_true() -> bool {
    true
}

fn default_max_queued() -> usize {
    10
}

fn default_max_parallel() -> usize {
    5
}
