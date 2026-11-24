pub mod influxdb;
pub mod sqlite;
pub mod types;

use std::sync::Arc;

use async_trait::async_trait;

use crate::config::{SecretsConfig, StorageBackend, StorageConfig};
pub use influxdb::InfluxDbStore;
pub use sqlite::SqliteStore;
pub use types::{
    AutomationOutcome, AutomationRunRecord, DiscreteTransition, EntityHistorySeries,
    HistoryEntryKind, NumericPoint, StateHistoryEntry, StorageError, TimeRange,
};

/// Pluggable async Data Access Layer (DAL) for entity history, numeric telemetry, and automation
/// execution audit logs.
#[async_trait]
pub trait HistoryStore: Send + Sync {
    async fn health_check(&self) -> Result<(), StorageError>;

    async fn record_state_batch(&self, records: &[StateHistoryEntry]) -> Result<(), StorageError>;

    async fn record_automation_run(&self, run: &AutomationRunRecord) -> Result<(), StorageError>;

    async fn query_entity_history(
        &self,
        entity_id: &str,
        range: TimeRange,
        bucket_seconds: Option<u32>,
    ) -> Result<EntityHistorySeries, StorageError>;

    async fn query_automation_runs(
        &self,
        automation_id: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<AutomationRunRecord>, StorageError>;

    async fn prune_before(&self, cutoff_unix_ms: i64) -> Result<u64, StorageError>;
}

/// Build the configured `HistoryStore` backend from `StorageConfig`.
pub async fn build_history_store(
    cfg: &StorageConfig,
) -> Result<Arc<dyn HistoryStore>, StorageError> {
    build_history_store_with_secrets(cfg, &SecretsConfig::default()).await
}

/// Build the configured `HistoryStore` backend with access to `SecretsConfig` (for InfluxDB tokens).
pub async fn build_history_store_with_secrets(
    cfg: &StorageConfig,
    secrets: &SecretsConfig,
) -> Result<Arc<dyn HistoryStore>, StorageError> {
    match cfg.backend {
        StorageBackend::Sqlite => {
            let store = SqliteStore::open(&cfg.sqlite.path).await?;
            Ok(Arc::new(store))
        }
        StorageBackend::InfluxDb => {
            let influx_cfg = cfg.influxdb.as_ref().ok_or_else(|| {
                StorageError::Connection(
                    "storage.backend is 'influxdb' but [storage.influxdb] is not configured".into(),
                )
            })?;
            let store = InfluxDbStore::from_config(influx_cfg, secrets)?;
            Ok(Arc::new(store))
        }
    }
}
