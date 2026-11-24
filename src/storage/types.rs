use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("storage connection error: {0}")]
    Connection(String),
    #[error("storage migration error: {0}")]
    Migration(String),
    #[error("storage query error: {0}")]
    Query(String),
    #[error("invalid query parameter: {0}")]
    InvalidQuery(String),
    #[error("storage serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("storage backend '{0}' is not implemented yet")]
    UnsupportedBackend(String),
}

/// Inclusive Unix-millisecond time range for history queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeRange {
    pub from_ms: i64,
    pub to_ms: i64,
}

impl TimeRange {
    pub fn new(from_ms: i64, to_ms: i64) -> Result<Self, StorageError> {
        if from_ms > to_ms {
            return Err(StorageError::InvalidQuery(format!(
                "from_ms ({from_ms}) must be <= to_ms ({to_ms})"
            )));
        }
        Ok(Self { from_ms, to_ms })
    }

    pub fn validate(&self) -> Result<(), StorageError> {
        if self.from_ms > self.to_ms {
            return Err(StorageError::InvalidQuery(format!(
                "from_ms ({}) must be <= to_ms ({})",
                self.from_ms, self.to_ms
            )));
        }
        Ok(())
    }
}

/// A single entity state or telemetry sample to persist in `HistoryStore`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateHistoryEntry {
    pub entity_id: String,
    pub ts: i64,
    pub kind: HistoryEntryKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HistoryEntryKind {
    Numeric {
        value: f64,
        unit: Option<String>,
    },
    Discrete {
        state: String,
        attributes: Option<Value>,
    },
}

/// Downsampled or raw numeric telemetry point (matches `uPlot` API contract).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NumericPoint {
    pub ts: i64,
    pub avg: f64,
    pub min: f64,
    pub max: f64,
}

/// Discrete entity state transition point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscreteTransition {
    pub ts: i64,
    pub state: String,
}

/// Result of `HistoryStore::query_entity_history`, matching `GET /api/entities/{id}/history`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EntityHistorySeries {
    Numeric {
        entity_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
        points: Vec<NumericPoint>,
    },
    Discrete {
        entity_id: String,
        transitions: Vec<DiscreteTransition>,
    },
}

/// Outcome of a single automation execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomationOutcome {
    Succeeded,
    Skipped,
    ConditionsNotMet,
    Cancelled,
    SkippedSingle,
    Failed,
}

impl AutomationOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Skipped => "skipped",
            Self::ConditionsNotMet => "conditions_not_met",
            Self::Cancelled => "cancelled",
            Self::SkippedSingle => "skipped_single",
            Self::Failed => "failed",
        }
    }
}

impl fmt::Display for AutomationOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AutomationOutcome {
    type Err = StorageError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "succeeded" => Ok(Self::Succeeded),
            "skipped" => Ok(Self::Skipped),
            "conditions_not_met" => Ok(Self::ConditionsNotMet),
            "cancelled" => Ok(Self::Cancelled),
            "skipped_single" => Ok(Self::SkippedSingle),
            "failed" => Ok(Self::Failed),
            other => Err(StorageError::Query(format!(
                "unknown automation outcome '{other}'"
            ))),
        }
    }
}

/// Audit log record for an automation execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutomationRunRecord {
    pub run_id: String,
    pub automation_id: String,
    pub trigger_source: String,
    pub conditions_passed: bool,
    pub outcome: AutomationOutcome,
    pub started_at: i64,
    pub finished_at: i64,
    pub steps: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
