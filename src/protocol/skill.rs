use serde::{Deserialize, Serialize};

/// Execution mode declared in `skill.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillMode {
    Device,
    Action,
}

/// Runtime lifecycle status reported by the supervisor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillStatus {
    #[default]
    Stopped,
    Installing,
    Running,
    Error,
}

/// Runtime environment type declared in `skill.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeType {
    PythonUv,
    Executable,
}

/// Restart policy for long-running device Skills.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestartPolicy {
    #[default]
    Exponential,
    Never,
}
