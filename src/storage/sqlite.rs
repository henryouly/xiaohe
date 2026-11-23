use std::fs;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use async_trait::async_trait;
use sqlx::Row;
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteSynchronous,
};

use crate::storage::HistoryStore;
use crate::storage::types::{
    AutomationOutcome, AutomationRunRecord, DiscreteTransition, EntityHistorySeries,
    HistoryEntryKind, NumericPoint, StateHistoryEntry, StorageError, TimeRange,
};

/// Default SQLite implementation of `HistoryStore` using WAL mode and bundled `libsqlite3`.
#[derive(Debug, Clone)]
pub struct SqliteStore {
    pool: SqlitePool,
}

impl SqliteStore {
    /// Open a SQLite database at `path` (or `":memory:"`), configure WAL mode, and run migrations.
    pub async fn open(path: &str) -> Result<Self, StorageError> {
        let is_memory = path == ":memory:" || path.starts_with("sqlite::memory:");

        let options = if is_memory {
            SqliteConnectOptions::from_str("sqlite::memory:")
                .map_err(|e| StorageError::Connection(e.to_string()))?
                .busy_timeout(Duration::from_secs(5))
        } else {
            let fs_path = Path::new(path);
            if let Some(parent) = fs_path.parent()
                && !parent.as_os_str().is_empty()
            {
                fs::create_dir_all(parent).map_err(|e| {
                    StorageError::Connection(format!(
                        "failed to create sqlite directory {}: {e}",
                        parent.display()
                    ))
                })?;
            }

            SqliteConnectOptions::new()
                .filename(fs_path)
                .create_if_missing(true)
                .journal_mode(SqliteJournalMode::Wal)
                .synchronous(SqliteSynchronous::Normal)
                .busy_timeout(Duration::from_secs(5))
        };

        let max_conns = if is_memory { 1 } else { 8 };
        let pool = SqlitePoolOptions::new()
            .max_connections(max_conns)
            .connect_with(options)
            .await
            .map_err(|e| StorageError::Connection(e.to_string()))?;

        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(|e| StorageError::Migration(e.to_string()))?;

        Ok(Self { pool })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

#[async_trait]
impl HistoryStore for SqliteStore {
    async fn health_check(&self) -> Result<(), StorageError> {
        sqlx::query_scalar::<_, i64>("SELECT 1")
            .fetch_one(&self.pool)
            .await
            .map(|_| ())
            .map_err(|e| StorageError::Query(e.to_string()))
    }

    async fn record_state_batch(&self, records: &[StateHistoryEntry]) -> Result<(), StorageError> {
        if records.is_empty() {
            return Ok(());
        }

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        for record in records {
            match &record.kind {
                HistoryEntryKind::Numeric { value, unit } => {
                    sqlx::query(
                        "INSERT INTO numeric_telemetry (entity_id, ts, value, unit) VALUES (?1, ?2, ?3, ?4)",
                    )
                    .bind(&record.entity_id)
                    .bind(record.ts)
                    .bind(value)
                    .bind(unit)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| StorageError::Query(e.to_string()))?;
                }
                HistoryEntryKind::Discrete { state, attributes } => {
                    let attrs_json = attributes.as_ref().map(serde_json::to_string).transpose()?;
                    sqlx::query(
                        "INSERT INTO entity_state_history (entity_id, ts, state, attributes_json) VALUES (?1, ?2, ?3, ?4)",
                    )
                    .bind(&record.entity_id)
                    .bind(record.ts)
                    .bind(state)
                    .bind(attrs_json)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| StorageError::Query(e.to_string()))?;
                }
            }
        }

        tx.commit()
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(())
    }

    async fn record_automation_run(&self, run: &AutomationRunRecord) -> Result<(), StorageError> {
        let steps_json = serde_json::to_string(&run.steps)?;
        sqlx::query(
            "INSERT INTO automation_runs \
             (run_id, automation_id, trigger_source, conditions_passed, outcome, started_at, finished_at, steps_json, error) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )
        .bind(&run.run_id)
        .bind(&run.automation_id)
        .bind(&run.trigger_source)
        .bind(if run.conditions_passed { 1_i64 } else { 0_i64 })
        .bind(run.outcome.as_str())
        .bind(run.started_at)
        .bind(run.finished_at)
        .bind(steps_json)
        .bind(&run.error)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(())
    }

    async fn query_entity_history(
        &self,
        entity_id: &str,
        range: TimeRange,
        bucket_seconds: Option<u32>,
    ) -> Result<EntityHistorySeries, StorageError> {
        range.validate()?;
        if bucket_seconds == Some(0) {
            return Err(StorageError::InvalidQuery(
                "bucket_seconds must be greater than 0".to_owned(),
            ));
        }

        let numeric_meta = sqlx::query(
            "SELECT unit FROM numeric_telemetry WHERE entity_id = ?1 ORDER BY ts DESC LIMIT 1",
        )
        .bind(entity_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Query(e.to_string()))?;

        if let Some(meta_row) = numeric_meta {
            let unit: Option<String> = meta_row
                .try_get("unit")
                .map_err(|e| StorageError::Query(e.to_string()))?;

            let points = if let Some(secs) = bucket_seconds {
                let bucket_ms = i64::from(secs) * 1000;
                let rows = sqlx::query(
                    "SELECT (ts / ?1) * ?1 AS bucket_ts, AVG(value) AS avg_val, MIN(value) AS min_val, MAX(value) AS max_val \
                     FROM numeric_telemetry \
                     WHERE entity_id = ?2 AND ts >= ?3 AND ts <= ?4 \
                     GROUP BY bucket_ts \
                     ORDER BY bucket_ts ASC",
                )
                .bind(bucket_ms)
                .bind(entity_id)
                .bind(range.from_ms)
                .bind(range.to_ms)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| StorageError::Query(e.to_string()))?;

                rows.into_iter()
                    .map(|r| {
                        Ok(NumericPoint {
                            ts: r.try_get("bucket_ts").map_err(query_err)?,
                            avg: r.try_get("avg_val").map_err(query_err)?,
                            min: r.try_get("min_val").map_err(query_err)?,
                            max: r.try_get("max_val").map_err(query_err)?,
                        })
                    })
                    .collect::<Result<Vec<_>, StorageError>>()?
            } else {
                let rows = sqlx::query(
                    "SELECT ts, value FROM numeric_telemetry \
                     WHERE entity_id = ?1 AND ts >= ?2 AND ts <= ?3 \
                     ORDER BY ts ASC",
                )
                .bind(entity_id)
                .bind(range.from_ms)
                .bind(range.to_ms)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| StorageError::Query(e.to_string()))?;

                rows.into_iter()
                    .map(|r| {
                        let ts: i64 = r.try_get("ts").map_err(query_err)?;
                        let value: f64 = r.try_get("value").map_err(query_err)?;
                        Ok(NumericPoint {
                            ts,
                            avg: value,
                            min: value,
                            max: value,
                        })
                    })
                    .collect::<Result<Vec<_>, StorageError>>()?
            };

            return Ok(EntityHistorySeries::Numeric {
                entity_id: entity_id.to_owned(),
                unit,
                points,
            });
        }

        let rows = sqlx::query(
            "SELECT ts, state FROM entity_state_history \
             WHERE entity_id = ?1 AND ts >= ?2 AND ts <= ?3 \
             ORDER BY ts ASC",
        )
        .bind(entity_id)
        .bind(range.from_ms)
        .bind(range.to_ms)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Query(e.to_string()))?;

        let transitions = rows
            .into_iter()
            .map(|r| {
                Ok(DiscreteTransition {
                    ts: r.try_get("ts").map_err(query_err)?,
                    state: r.try_get("state").map_err(query_err)?,
                })
            })
            .collect::<Result<Vec<_>, StorageError>>()?;

        Ok(EntityHistorySeries::Discrete {
            entity_id: entity_id.to_owned(),
            transitions,
        })
    }

    async fn query_automation_runs(
        &self,
        automation_id: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<AutomationRunRecord>, StorageError> {
        let rows = if let Some(auto_id) = automation_id {
            sqlx::query(
                "SELECT run_id, automation_id, trigger_source, conditions_passed, outcome, started_at, finished_at, steps_json, error \
                 FROM automation_runs \
                 WHERE automation_id = ?1 \
                 ORDER BY started_at DESC \
                 LIMIT ?2 OFFSET ?3",
            )
            .bind(auto_id)
            .bind(i64::from(limit))
            .bind(i64::from(offset))
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?
        } else {
            sqlx::query(
                "SELECT run_id, automation_id, trigger_source, conditions_passed, outcome, started_at, finished_at, steps_json, error \
                 FROM automation_runs \
                 ORDER BY started_at DESC \
                 LIMIT ?1 OFFSET ?2",
            )
            .bind(i64::from(limit))
            .bind(i64::from(offset))
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?
        };

        rows.into_iter()
            .map(|r| {
                let outcome_str: String = r.try_get("outcome").map_err(query_err)?;
                let steps_str: String = r.try_get("steps_json").map_err(query_err)?;
                let cond_passed: i64 = r.try_get("conditions_passed").map_err(query_err)?;
                Ok(AutomationRunRecord {
                    run_id: r.try_get("run_id").map_err(query_err)?,
                    automation_id: r.try_get("automation_id").map_err(query_err)?,
                    trigger_source: r.try_get("trigger_source").map_err(query_err)?,
                    conditions_passed: cond_passed != 0,
                    outcome: AutomationOutcome::from_str(&outcome_str)?,
                    started_at: r.try_get("started_at").map_err(query_err)?,
                    finished_at: r.try_get("finished_at").map_err(query_err)?,
                    steps: serde_json::from_str(&steps_str)?,
                    error: r.try_get("error").map_err(query_err)?,
                })
            })
            .collect()
    }

    async fn prune_before(&self, cutoff_unix_ms: i64) -> Result<u64, StorageError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let d1 = sqlx::query("DELETE FROM numeric_telemetry WHERE ts < ?1")
            .bind(cutoff_unix_ms)
            .execute(&mut *tx)
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?
            .rows_affected();

        let d2 = sqlx::query("DELETE FROM entity_state_history WHERE ts < ?1")
            .bind(cutoff_unix_ms)
            .execute(&mut *tx)
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?
            .rows_affected();

        let d3 = sqlx::query("DELETE FROM automation_runs WHERE started_at < ?1")
            .bind(cutoff_unix_ms)
            .execute(&mut *tx)
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?
            .rows_affected();

        tx.commit()
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(d1 + d2 + d3)
    }
}

fn query_err(err: sqlx::Error) -> StorageError {
    StorageError::Query(err.to_string())
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    #[tokio::test]
    async fn file_backed_sqlite_enables_wal_and_passes_health_check() {
        let dir = TempDir::new().expect("tempdir");
        let db_path = dir.path().join("nested").join("xiaohe.db");
        let store = SqliteStore::open(db_path.to_str().expect("utf8 path"))
            .await
            .expect("open sqlite");

        store.health_check().await.expect("health_check");

        let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(store.pool())
            .await
            .expect("pragma journal_mode");
        assert_eq!(mode.to_ascii_lowercase(), "wal");
    }

    #[tokio::test]
    async fn numeric_and_discrete_history_with_downsampling() {
        let store = SqliteStore::open(":memory:").await.expect("open memory db");

        let batch = vec![
            StateHistoryEntry {
                entity_id: "sensor.living_temp".into(),
                ts: 60_000,
                kind: HistoryEntryKind::Numeric {
                    value: 20.0,
                    unit: Some("°C".into()),
                },
            },
            StateHistoryEntry {
                entity_id: "sensor.living_temp".into(),
                ts: 90_000,
                kind: HistoryEntryKind::Numeric {
                    value: 22.0,
                    unit: Some("°C".into()),
                },
            },
            StateHistoryEntry {
                entity_id: "sensor.living_temp".into(),
                ts: 120_000,
                kind: HistoryEntryKind::Numeric {
                    value: 24.0,
                    unit: Some("°C".into()),
                },
            },
            StateHistoryEntry {
                entity_id: "light.living_ceiling".into(),
                ts: 60_000,
                kind: HistoryEntryKind::Discrete {
                    state: "on".into(),
                    attributes: Some(json!({ "brightness": 80 })),
                },
            },
            StateHistoryEntry {
                entity_id: "light.living_ceiling".into(),
                ts: 120_000,
                kind: HistoryEntryKind::Discrete {
                    state: "off".into(),
                    attributes: None,
                },
            },
        ];
        store
            .record_state_batch(&batch)
            .await
            .expect("record batch");

        let range = TimeRange::new(0, 200_000).expect("valid range");

        // Raw numeric query
        let raw = store
            .query_entity_history("sensor.living_temp", range, None)
            .await
            .expect("raw numeric query");
        match raw {
            EntityHistorySeries::Numeric {
                entity_id,
                unit,
                points,
            } => {
                assert_eq!(entity_id, "sensor.living_temp");
                assert_eq!(unit.as_deref(), Some("°C"));
                assert_eq!(points.len(), 3);
                assert_eq!(points[0].avg, 20.0);
                assert_eq!(points[2].avg, 24.0);
            }
            other => panic!("expected numeric series, got {other:?}"),
        }

        // Downsampled numeric query (60s buckets: [60_000, 90_000] -> avg 21, min 20, max 22; [120_000] -> 24)
        let bucketed = store
            .query_entity_history("sensor.living_temp", range, Some(60))
            .await
            .expect("bucketed numeric query");
        match bucketed {
            EntityHistorySeries::Numeric { points, .. } => {
                assert_eq!(points.len(), 2);
                assert_eq!(points[0].ts, 60_000);
                assert_eq!(points[0].avg, 21.0);
                assert_eq!(points[0].min, 20.0);
                assert_eq!(points[0].max, 22.0);
                assert_eq!(points[1].ts, 120_000);
                assert_eq!(points[1].avg, 24.0);
            }
            other => panic!("expected numeric series, got {other:?}"),
        }

        // Discrete query
        let discrete = store
            .query_entity_history("light.living_ceiling", range, None)
            .await
            .expect("discrete query");
        match discrete {
            EntityHistorySeries::Discrete {
                entity_id,
                transitions,
            } => {
                assert_eq!(entity_id, "light.living_ceiling");
                assert_eq!(transitions.len(), 2);
                assert_eq!(transitions[0].state, "on");
                assert_eq!(transitions[1].state, "off");
            }
            other => panic!("expected discrete series, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn automation_runs_query_filter_pagination_and_prune() {
        let store = SqliteStore::open(":memory:").await.expect("open memory db");

        for i in 1..=3_i64 {
            let run = AutomationRunRecord {
                run_id: format!("run-{i}"),
                automation_id: if i == 2 {
                    "auto_night".into()
                } else {
                    "auto_motion".into()
                },
                trigger_source: "state:binary_sensor.motion".into(),
                conditions_passed: true,
                outcome: if i == 3 {
                    AutomationOutcome::Failed
                } else {
                    AutomationOutcome::Succeeded
                },
                started_at: i * 1_000,
                finished_at: i * 1_000 + 50,
                steps: json!([{ "step": 0, "ok": i != 3 }]),
                error: (i == 3).then(|| "device offline".into()),
            };
            store.record_automation_run(&run).await.expect("record run");
        }

        // All runs newest-first with pagination
        let page = store
            .query_automation_runs(None, 2, 0)
            .await
            .expect("query all runs");
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].run_id, "run-3");
        assert_eq!(page[0].outcome, AutomationOutcome::Failed);
        assert_eq!(page[0].error.as_deref(), Some("device offline"));
        assert_eq!(page[1].run_id, "run-2");

        // Filtered by automation_id
        let filtered = store
            .query_automation_runs(Some("auto_motion"), 10, 0)
            .await
            .expect("filtered runs");
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].run_id, "run-3");
        assert_eq!(filtered[1].run_id, "run-1");

        // Also insert state rows and prune everything before ts = 2_500
        store
            .record_state_batch(&[
                StateHistoryEntry {
                    entity_id: "sensor.temp".into(),
                    ts: 1_000,
                    kind: HistoryEntryKind::Numeric {
                        value: 19.5,
                        unit: None,
                    },
                },
                StateHistoryEntry {
                    entity_id: "switch.plug".into(),
                    ts: 1_500,
                    kind: HistoryEntryKind::Discrete {
                        state: "on".into(),
                        attributes: None,
                    },
                },
            ])
            .await
            .expect("record state batch");

        let pruned = store.prune_before(2_500).await.expect("prune");
        // 2 automation runs (run-1, run-2) + 1 numeric + 1 discrete = 4 rows
        assert_eq!(pruned, 4);

        let remaining = store
            .query_automation_runs(None, 10, 0)
            .await
            .expect("remaining runs");
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].run_id, "run-3");
    }

    #[tokio::test]
    async fn invalid_query_parameters_return_error() {
        let store = SqliteStore::open(":memory:").await.expect("open memory db");

        assert!(TimeRange::new(200, 100).is_err());

        let bad_range = TimeRange {
            from_ms: 200,
            to_ms: 100,
        };
        let err = store
            .query_entity_history("sensor.temp", bad_range, None)
            .await
            .expect_err("expected invalid range");
        assert!(matches!(err, StorageError::InvalidQuery(_)));

        let err = store
            .query_entity_history("sensor.temp", TimeRange::new(0, 100).unwrap(), Some(0))
            .await
            .expect_err("expected invalid bucket_seconds");
        assert!(matches!(err, StorageError::InvalidQuery(_)));
    }
}
