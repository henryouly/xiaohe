use std::collections::HashMap;
use std::fmt::Write as _;
use std::str::FromStr;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde_json::json;

use crate::config::{InfluxDbConfig, SecretsConfig};
use crate::storage::HistoryStore;
use crate::storage::types::{
    AutomationOutcome, AutomationRunRecord, DiscreteTransition, EntityHistorySeries,
    HistoryEntryKind, NumericPoint, StateHistoryEntry, StorageError, TimeRange,
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

/// InfluxDB v2 implementation of `HistoryStore` using the HTTP Line Protocol & Flux Query APIs.
#[derive(Debug, Clone)]
pub struct InfluxDbStore {
    client: Client,
    base_url: String,
    org: String,
    bucket: String,
    token: Option<String>,
}

impl InfluxDbStore {
    /// Construct an `InfluxDbStore` from `InfluxDbConfig` and optional resolved token.
    pub fn from_config(
        cfg: &InfluxDbConfig,
        secrets: &SecretsConfig,
    ) -> Result<Self, StorageError> {
        let base_url = cfg.url.trim().trim_end_matches('/').to_owned();
        if base_url.is_empty()
            || (!base_url.starts_with("http://") && !base_url.starts_with("https://"))
        {
            return Err(StorageError::Connection(format!(
                "invalid influxdb.url '{}': must start with http:// or https://",
                cfg.url
            )));
        }
        let org = cfg
            .org
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("xiaohe")
            .to_owned();
        let bucket = cfg
            .bucket
            .as_deref()
            .or(cfg.database.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                StorageError::Connection("influxdb.bucket (or database) must be configured".into())
            })?
            .to_owned();

        let token = std::env::var("INFLUXDB_TOKEN")
            .ok()
            .filter(|v| !v.is_empty())
            .or_else(|| cfg.token_secret.as_deref().and_then(|k| secrets.get(k)));

        let client = Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .build()
            .map_err(|e| StorageError::Connection(format!("failed to build HTTP client: {e}")))?;

        Ok(Self {
            client,
            base_url,
            org,
            bucket,
            token,
        })
    }

    /// Direct constructor useful for unit/integration tests.
    pub fn new(
        base_url: impl Into<String>,
        org: impl Into<String>,
        bucket: impl Into<String>,
        token: Option<String>,
    ) -> Result<Self, StorageError> {
        let client = Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .build()
            .map_err(|e| StorageError::Connection(format!("failed to build HTTP client: {e}")))?;
        Ok(Self {
            client,
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            org: org.into(),
            bucket: bucket.into(),
            token,
        })
    }

    fn apply_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.token {
            Some(tok) if !tok.is_empty() => req.header("Authorization", format!("Token {tok}")),
            _ => req,
        }
    }

    async fn write_lines(&self, lines: &str) -> Result<(), StorageError> {
        if lines.is_empty() {
            return Ok(());
        }
        let url = format!("{}/api/v2/write", self.base_url);
        let resp = self
            .apply_auth(self.client.post(&url))
            .query(&[
                ("org", self.org.as_str()),
                ("bucket", self.bucket.as_str()),
                ("precision", "ms"),
            ])
            .header("Content-Type", "text/plain; charset=utf-8")
            .body(lines.to_owned())
            .send()
            .await
            .map_err(|e| StorageError::Connection(format!("influxdb write failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StorageError::Query(format!(
                "influxdb write returned HTTP {status}: {body}"
            )));
        }
        Ok(())
    }

    async fn query_flux(&self, flux: &str) -> Result<String, StorageError> {
        let url = format!("{}/api/v2/query", self.base_url);
        let resp = self
            .apply_auth(self.client.post(&url))
            .query(&[("org", self.org.as_str())])
            .header("Content-Type", "application/vnd.flux")
            .header("Accept", "application/csv")
            .body(flux.to_owned())
            .send()
            .await
            .map_err(|e| StorageError::Connection(format!("influxdb query failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StorageError::Query(format!(
                "influxdb query returned HTTP {status}: {body}"
            )));
        }
        resp.text()
            .await
            .map_err(|e| StorageError::Query(format!("failed reading influxdb CSV response: {e}")))
    }
}

#[async_trait]
impl HistoryStore for InfluxDbStore {
    async fn health_check(&self) -> Result<(), StorageError> {
        let url = format!("{}/health", self.base_url);
        let resp = self
            .apply_auth(self.client.get(&url))
            .send()
            .await
            .map_err(|e| StorageError::Connection(format!("influxdb health check failed: {e}")))?;
        if !resp.status().is_success() {
            return Err(StorageError::Connection(format!(
                "influxdb /health returned HTTP {}",
                resp.status()
            )));
        }
        Ok(())
    }

    async fn record_state_batch(&self, records: &[StateHistoryEntry]) -> Result<(), StorageError> {
        let mut buf = String::new();
        for rec in records {
            if let Some(line) = encode_state_entry_line(rec) {
                if !buf.is_empty() {
                    buf.push('\n');
                }
                buf.push_str(&line);
            }
        }
        self.write_lines(&buf).await
    }

    async fn record_automation_run(&self, run: &AutomationRunRecord) -> Result<(), StorageError> {
        let line = encode_automation_run_line(run)?;
        self.write_lines(&line).await
    }

    async fn query_entity_history(
        &self,
        entity_id: &str,
        range: TimeRange,
        bucket_seconds: Option<u32>,
    ) -> Result<EntityHistorySeries, StorageError> {
        range.validate()?;
        if let Some(0) = bucket_seconds {
            return Err(StorageError::InvalidQuery(
                "bucket_seconds must be > 0".into(),
            ));
        }

        let start = ms_to_rfc3339(range.from_ms)?;
        let stop = ms_to_rfc3339(range.to_ms.saturating_add(1))?;
        let bucket_lit = escape_flux_string(&self.bucket);
        let entity_lit = escape_flux_string(entity_id);

        // 1. Try numeric telemetry first (raw or downsampled via aggregateWindow).
        // `unit` is stored as a tag on `numeric_telemetry` so aggregateWindow preserves it.
        let numeric_flux = match bucket_seconds {
            Some(secs) => format!(
                r#"base = from(bucket: "{bucket_lit}")
  |> range(start: {start}, stop: {stop})
  |> filter(fn: (r) => r._measurement == "numeric_telemetry" and r.entity_id == "{entity_lit}" and r._field == "value")
mean_t = base |> aggregateWindow(every: {secs}s, fn: mean, createEmpty: false) |> set(key: "_field", value: "avg")
min_t = base |> aggregateWindow(every: {secs}s, fn: min, createEmpty: false) |> set(key: "_field", value: "min")
max_t = base |> aggregateWindow(every: {secs}s, fn: max, createEmpty: false) |> set(key: "_field", value: "max")
union(tables: [mean_t, min_t, max_t])
  |> pivot(rowKey: ["_time"], columnKey: ["_field"], valueColumn: "_value")
  |> group()
  |> sort(columns: ["_time"])"#
            ),
            None => format!(
                r#"from(bucket: "{bucket_lit}")
  |> range(start: {start}, stop: {stop})
  |> filter(fn: (r) => r._measurement == "numeric_telemetry" and r.entity_id == "{entity_lit}")
  |> pivot(rowKey: ["_time"], columnKey: ["_field"], valueColumn: "_value")
  |> group()
  |> sort(columns: ["_time"])"#
            ),
        };

        let numeric_csv = self.query_flux(&numeric_flux).await?;
        let numeric_rows = parse_influx_csv(&numeric_csv);
        if !numeric_rows.is_empty() {
            let mut unit: Option<String> = None;
            let mut points = Vec::with_capacity(numeric_rows.len());
            for row in numeric_rows {
                let Some(ts) = row.get("_time").and_then(|s| parse_rfc3339_ms(s)) else {
                    continue;
                };
                if unit.is_none()
                    && let Some(u) = row.get("unit").filter(|s| !s.is_empty())
                {
                    unit = Some(u.clone());
                }
                if let (Some(avg), Some(min), Some(max)) = (
                    row.get("avg").and_then(|s| s.parse::<f64>().ok()),
                    row.get("min").and_then(|s| s.parse::<f64>().ok()),
                    row.get("max").and_then(|s| s.parse::<f64>().ok()),
                ) {
                    points.push(NumericPoint { ts, avg, min, max });
                } else if let Some(val) = row.get("value").and_then(|s| s.parse::<f64>().ok()) {
                    points.push(NumericPoint {
                        ts,
                        avg: val,
                        min: val,
                        max: val,
                    });
                }
            }
            if !points.is_empty() {
                return Ok(EntityHistorySeries::Numeric {
                    entity_id: entity_id.to_owned(),
                    unit,
                    points,
                });
            }
        }

        // 2. Fall back to discrete entity state transitions.
        let discrete_flux = format!(
            r#"from(bucket: "{bucket_lit}")
  |> range(start: {start}, stop: {stop})
  |> filter(fn: (r) => r._measurement == "entity_state_history" and r.entity_id == "{entity_lit}" and r._field == "state")
  |> group()
  |> sort(columns: ["_time"])"#
        );
        let discrete_csv = self.query_flux(&discrete_flux).await?;
        let discrete_rows = parse_influx_csv(&discrete_csv);
        let mut transitions = Vec::with_capacity(discrete_rows.len());
        for row in discrete_rows {
            if let (Some(ts), Some(state)) = (
                row.get("_time").and_then(|s| parse_rfc3339_ms(s)),
                row.get("_value"),
            ) {
                transitions.push(DiscreteTransition {
                    ts,
                    state: state.clone(),
                });
            }
        }

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
        if limit == 0 {
            return Ok(Vec::new());
        }
        let bucket_lit = escape_flux_string(&self.bucket);
        let id_filter = match automation_id {
            Some(id) => format!(r#" and r.automation_id == "{}""#, escape_flux_string(id)),
            None => String::new(),
        };
        let flux = format!(
            r#"from(bucket: "{bucket_lit}")
  |> range(start: 0)
  |> filter(fn: (r) => r._measurement == "automation_runs"{id_filter})
  |> pivot(rowKey: ["_time", "run_id", "automation_id", "outcome"], columnKey: ["_field"], valueColumn: "_value")
  |> group()
  |> sort(columns: ["_time"], desc: true)
  |> limit(n: {limit}, offset: {offset})"#
        );
        let csv = self.query_flux(&flux).await?;
        let rows = parse_influx_csv(&csv);
        let mut runs = Vec::with_capacity(rows.len());
        for row in rows {
            let Some(run_id) = row.get("run_id").filter(|s| !s.is_empty()).cloned() else {
                continue;
            };
            let Some(auto_id) = row.get("automation_id").filter(|s| !s.is_empty()).cloned() else {
                continue;
            };
            let outcome = row
                .get("outcome")
                .and_then(|s| AutomationOutcome::from_str(s).ok())
                .unwrap_or(AutomationOutcome::Failed);
            let started_at = row
                .get("started_at")
                .and_then(|s| s.parse::<i64>().ok())
                .or_else(|| row.get("_time").and_then(|s| parse_rfc3339_ms(s)))
                .unwrap_or(0);
            let finished_at = row
                .get("finished_at")
                .and_then(|s| s.parse::<i64>().ok())
                .unwrap_or(started_at);
            let conditions_passed = row
                .get("conditions_passed")
                .map(|s| s.eq_ignore_ascii_case("true"))
                .unwrap_or(true);
            let steps = row
                .get("steps")
                .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                .unwrap_or_else(|| json!([]));
            let trigger_source = row
                .get("trigger_source")
                .cloned()
                .unwrap_or_else(|| "unknown".into());
            let error = row.get("error").filter(|s| !s.is_empty()).cloned();
            runs.push(AutomationRunRecord {
                run_id,
                automation_id: auto_id,
                trigger_source,
                conditions_passed,
                outcome,
                started_at,
                finished_at,
                steps,
                error,
            });
        }
        Ok(runs)
    }

    async fn prune_before(&self, cutoff_unix_ms: i64) -> Result<u64, StorageError> {
        let stop = ms_to_rfc3339(cutoff_unix_ms)?;
        let url = format!("{}/api/v2/delete", self.base_url);
        for measurement in [
            "numeric_telemetry",
            "entity_state_history",
            "automation_runs",
        ] {
            let body = json!({
                "start": "1970-01-01T00:00:00Z",
                "stop": stop,
                "predicate": format!("_measurement=\"{measurement}\""),
            });
            let resp = self
                .apply_auth(self.client.post(&url))
                .query(&[("org", self.org.as_str()), ("bucket", self.bucket.as_str())])
                .json(&body)
                .send()
                .await
                .map_err(|e| StorageError::Connection(format!("influxdb delete failed: {e}")))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                return Err(StorageError::Query(format!(
                    "influxdb delete returned HTTP {status}: {text}"
                )));
            }
        }
        // InfluxDB v2 /api/v2/delete returns 204 No Content without a deleted-row count.
        Ok(0)
    }
}

/// Escape an InfluxDB Line Protocol tag key or tag value (commas, equals, spaces, backslashes, newlines).
pub(crate) fn escape_lp_tag(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            ',' | '=' | ' ' | '\\' => {
                out.push('\\');
                out.push(ch);
            }
            '\n' | '\r' => out.push('_'),
            _ => out.push(ch),
        }
    }
    out
}

/// Escape an InfluxDB Line Protocol string field value (double quotes, backslashes, newlines).
pub(crate) fn escape_lp_string(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '"' | '\\' => {
                out.push('\\');
                out.push(ch);
            }
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(ch),
        }
    }
    out
}

/// Escape a Flux double-quoted string literal to prevent query and string-interpolation injection.
pub(crate) fn escape_flux_string(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '$' if chars.peek() == Some(&'{') => out.push_str("\\$"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out
}

pub(crate) fn encode_state_entry_line(entry: &StateHistoryEntry) -> Option<String> {
    let entity = escape_lp_tag(&entry.entity_id);
    if entity.is_empty() {
        return None;
    }
    let mut line = String::new();
    match &entry.kind {
        HistoryEntryKind::Numeric { value, unit } => {
            if !value.is_finite() {
                return None;
            }
            // Store `unit` as a tag so Flux `aggregateWindow` preserves it during downsampling.
            let _ = write!(line, "numeric_telemetry,entity_id={entity}");
            if let Some(u) = unit.as_deref().filter(|s| !s.is_empty()) {
                let _ = write!(line, ",unit={}", escape_lp_tag(u));
            }
            let _ = write!(line, " value={value} {}", entry.ts);
        }
        HistoryEntryKind::Discrete { state, attributes } => {
            let _ = write!(
                line,
                "entity_state_history,entity_id={entity} state=\"{}\"",
                escape_lp_string(state)
            );
            if let Some(attrs) = attributes {
                let serialized = attrs.to_string();
                let _ = write!(line, ",attributes=\"{}\"", escape_lp_string(&serialized));
            }
            let _ = write!(line, " {}", entry.ts);
        }
    }
    Some(line)
}

pub(crate) fn encode_automation_run_line(
    run: &AutomationRunRecord,
) -> Result<String, StorageError> {
    let auto_tag = escape_lp_tag(&run.automation_id);
    let run_tag = escape_lp_tag(&run.run_id);
    if auto_tag.is_empty() || run_tag.is_empty() {
        return Err(StorageError::InvalidQuery(
            "automation_id and run_id must be non-empty".into(),
        ));
    }
    let steps_json = run.steps.to_string();
    let mut line = format!(
        "automation_runs,automation_id={},run_id={},outcome={} trigger_source=\"{}\",conditions_passed={},started_at={}i,finished_at={}i,steps=\"{}\"",
        auto_tag,
        run_tag,
        escape_lp_tag(run.outcome.as_str()),
        escape_lp_string(&run.trigger_source),
        run.conditions_passed,
        run.started_at,
        run.finished_at,
        escape_lp_string(&steps_json),
    );
    if let Some(err) = run.error.as_deref() {
        let _ = write!(line, ",error=\"{}\"", escape_lp_string(err));
    }
    let _ = write!(line, " {}", run.started_at);
    Ok(line)
}

fn ms_to_rfc3339(ms: i64) -> Result<String, StorageError> {
    DateTime::<Utc>::from_timestamp_millis(ms)
        .map(|dt| dt.to_rfc3339())
        .ok_or_else(|| StorageError::InvalidQuery(format!("invalid unix ms timestamp: {ms}")))
}

fn parse_rfc3339_ms(raw: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(raw.trim())
        .ok()
        .map(|dt| dt.timestamp_millis())
}

/// RFC-4180 CSV parser for InfluxDB v2 annotated CSV responses.
/// Properly handles embedded newlines/commas/escaped quotes inside quoted fields,
/// skips `#datatype`/`#group`/`#default` annotation rows, and resets headers on blank separator rows.
pub(crate) fn parse_influx_csv(csv: &str) -> Vec<HashMap<String, String>> {
    let records = split_csv_records(csv);
    let mut rows = Vec::new();
    let mut headers: Option<Vec<String>> = None;

    for fields in records {
        if fields.len() == 1 && fields[0].is_empty() {
            // Blank line separates Flux result tables; reset header state.
            headers = None;
            continue;
        }
        if fields.first().is_some_and(|f| f.starts_with('#')) {
            continue;
        }
        if headers.is_none() {
            headers = Some(fields);
            continue;
        }
        if let Some(ref cols) = headers {
            if fields == *cols {
                continue;
            }
            let mut map = HashMap::with_capacity(cols.len());
            for (idx, col) in cols.iter().enumerate() {
                if !col.is_empty()
                    && let Some(val) = fields.get(idx)
                {
                    map.insert(col.clone(), val.clone());
                }
            }
            rows.push(map);
        }
    }

    rows
}

fn split_csv_records(input: &str) -> Vec<Vec<String>> {
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut field_started = false;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if in_quotes {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    cur.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                cur.push(ch);
            }
            continue;
        }

        match ch {
            ',' => {
                record.push(std::mem::take(&mut cur));
                field_started = false;
            }
            '"' if !field_started => {
                in_quotes = true;
                field_started = true;
            }
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                record.push(std::mem::take(&mut cur));
                records.push(std::mem::take(&mut record));
                field_started = false;
            }
            '\n' => {
                record.push(std::mem::take(&mut cur));
                records.push(std::mem::take(&mut record));
                field_started = false;
            }
            _ => {
                cur.push(ch);
                field_started = true;
            }
        }
    }

    if !cur.is_empty() || !record.is_empty() {
        record.push(cur);
        records.push(record);
    }

    records
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::Router;
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::{get, post};

    use super::*;

    #[derive(Default)]
    struct MockInfluxState {
        writes: Mutex<Vec<String>>,
        queries: Mutex<Vec<String>>,
        deletes: Mutex<Vec<serde_json::Value>>,
        auth_headers: Mutex<Vec<Option<String>>>,
        next_csv: Mutex<String>,
    }

    async fn start_mock_influx(state: Arc<MockInfluxState>) -> String {
        let app = Router::new()
            .route(
                "/health",
                get(
                    |State(st): State<Arc<MockInfluxState>>, headers: HeaderMap| async move {
                        st.auth_headers.lock().unwrap().push(
                            headers
                                .get("authorization")
                                .and_then(|v| v.to_str().ok())
                                .map(str::to_owned),
                        );
                        (StatusCode::OK, r#"{"status":"pass"}"#)
                    },
                ),
            )
            .route(
                "/api/v2/write",
                post(
                    |State(st): State<Arc<MockInfluxState>>, body: String| async move {
                        st.writes.lock().unwrap().push(body);
                        StatusCode::NO_CONTENT
                    },
                ),
            )
            .route(
                "/api/v2/query",
                post(
                    |State(st): State<Arc<MockInfluxState>>, body: String| async move {
                        st.queries.lock().unwrap().push(body);
                        let csv = st.next_csv.lock().unwrap().clone();
                        (StatusCode::OK, csv)
                    },
                ),
            )
            .route(
                "/api/v2/delete",
                post(
                    |State(st): State<Arc<MockInfluxState>>,
                     axum::Json(v): axum::Json<serde_json::Value>| async move {
                        st.deletes.lock().unwrap().push(v);
                        StatusCode::NO_CONTENT
                    },
                ),
            )
            .with_state(state);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock influx");
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        format!("http://{addr}")
    }

    #[test]
    fn line_protocol_and_flux_escaping_handles_special_chars_and_non_finite() {
        let entry = StateHistoryEntry {
            entity_id: "sensor.living room,pm=2.5".into(),
            ts: 1_700_000_000_000,
            kind: HistoryEntryKind::Numeric {
                value: 23.5,
                unit: Some("ug/m3 \"raw\"".into()),
            },
        };
        let line = encode_state_entry_line(&entry).expect("encoded");
        assert_eq!(
            line,
            r#"numeric_telemetry,entity_id=sensor.living\ room\,pm\=2.5,unit=ug/m3\ "raw" value=23.5 1700000000000"#
        );

        let nan_entry = StateHistoryEntry {
            entity_id: "sensor.bad".into(),
            ts: 1_700_000_000_000,
            kind: HistoryEntryKind::Numeric {
                value: f64::NAN,
                unit: None,
            },
        };
        assert!(encode_state_entry_line(&nan_entry).is_none());

        assert_eq!(
            escape_flux_string(r#"foo") |> drop() // ${r.entity_id}"#),
            r#"foo\") |> drop() // \${r.entity_id}"#
        );

        // Regression: empty automation_id or run_id must be rejected instead of emitting invalid LP
        let empty_run = AutomationRunRecord {
            run_id: String::new(),
            automation_id: String::new(),
            trigger_source: "manual".into(),
            conditions_passed: true,
            outcome: AutomationOutcome::Succeeded,
            started_at: 1_700_000_000_000,
            finished_at: 1_700_000_000_120,
            steps: json!([]),
            error: None,
        };
        assert!(encode_automation_run_line(&empty_run).is_err());

        // Regression: RFC-4180 multi-line quoted CSV fields with blank lines parse intact
        let multiline_csv = ",result,table,_time,run_id,error\r\n,,0,2023-11-14T22:13:20Z,run_1,\"Traceback:\n\nValueError: bad \"\"payload\"\"\"\r\n,,0,2023-11-14T22:13:25Z,run_2,\r\n";
        let parsed = parse_influx_csv(multiline_csv);
        assert_eq!(parsed.len(), 2);
        assert_eq!(
            parsed[0].get("error").map(String::as_str),
            Some("Traceback:\n\nValueError: bad \"payload\"")
        );
        assert_eq!(parsed[1].get("run_id").map(String::as_str), Some("run_2"));
    }

    #[tokio::test]
    async fn influxdb_store_round_trips_health_write_query_and_prune() {
        let mock_state = Arc::new(MockInfluxState::default());
        let base_url = start_mock_influx(Arc::clone(&mock_state)).await;
        let store = InfluxDbStore::new(&base_url, "xiaohe", "home", Some("secret-tok".into()))
            .expect("store");

        // 1. Health check sends Token auth header
        store.health_check().await.expect("health ok");
        assert_eq!(
            mock_state.auth_headers.lock().unwrap()[0].as_deref(),
            Some("Token secret-tok")
        );

        // 2. Write numeric + discrete batch
        store
            .record_state_batch(&[
                StateHistoryEntry {
                    entity_id: "sensor.temp".into(),
                    ts: 1_700_000_000_000,
                    kind: HistoryEntryKind::Numeric {
                        value: 22.5,
                        unit: Some("C".into()),
                    },
                },
                StateHistoryEntry {
                    entity_id: "light.living".into(),
                    ts: 1_700_000_005_000,
                    kind: HistoryEntryKind::Discrete {
                        state: "on".into(),
                        attributes: Some(json!({"brightness": 80})),
                    },
                },
            ])
            .await
            .expect("write batch");
        let writes = mock_state.writes.lock().unwrap().clone();
        assert_eq!(writes.len(), 1);
        assert!(writes[0].contains("numeric_telemetry,entity_id=sensor.temp,unit=C value=22.5"));
        assert!(writes[0].contains("entity_state_history,entity_id=light.living state=\"on\""));

        // 3. Query downsampled numeric series from annotated CSV
        *mock_state.next_csv.lock().unwrap() = "\
#datatype,string,long,dateTime:RFC3339,double,double,double,string
#group,false,false,false,false,false,false,false
#default,_result,,,,,,
,result,table,_time,avg,min,max,unit
,,0,2023-11-14T22:13:20Z,22.5,21.0,24.0,C
"
        .into();
        let series = store
            .query_entity_history(
                "sensor.temp",
                TimeRange::new(1_700_000_000_000, 1_700_000_600_000).unwrap(),
                Some(300),
            )
            .await
            .expect("query numeric");
        match series {
            EntityHistorySeries::Numeric {
                entity_id,
                unit,
                points,
            } => {
                assert_eq!(entity_id, "sensor.temp");
                assert_eq!(unit.as_deref(), Some("C"));
                assert_eq!(points.len(), 1);
                assert_eq!(points[0].avg, 22.5);
                assert_eq!(points[0].min, 21.0);
                assert_eq!(points[0].max, 24.0);
            }
            other => panic!("expected numeric series, got {other:?}"),
        }

        // 4. Record and query automation run (verify |> group() precedes sort/limit)
        store
            .record_automation_run(&AutomationRunRecord {
                run_id: "run_1".into(),
                automation_id: "auto_1".into(),
                trigger_source: "manual".into(),
                conditions_passed: true,
                outcome: AutomationOutcome::Succeeded,
                started_at: 1_700_000_000_000,
                finished_at: 1_700_000_000_120,
                steps: json!([]),
                error: None,
            })
            .await
            .expect("record run");

        *mock_state.next_csv.lock().unwrap() = "\
,result,table,_time,run_id,automation_id,outcome,trigger_source,conditions_passed,started_at,finished_at,steps,error
,,0,2023-11-14T22:13:20Z,run_1,auto_1,succeeded,manual,true,1700000000000,1700000000120,[],
"
        .into();
        let runs = store
            .query_automation_runs(Some("auto_1"), 10, 0)
            .await
            .expect("query runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, "run_1");
        assert_eq!(runs[0].outcome, AutomationOutcome::Succeeded);
        assert!(
            mock_state
                .queries
                .lock()
                .unwrap()
                .last()
                .unwrap()
                .contains("|> group()\n  |> sort(")
        );

        // 5. Prune before cutoff issues /api/v2/delete for all 3 measurements
        store
            .prune_before(1_700_000_000_000)
            .await
            .expect("prune ok");
        assert_eq!(mock_state.deletes.lock().unwrap().len(), 3);
    }
}
