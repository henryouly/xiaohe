-- Initial schema for Xiaohe SQLite HistoryStore (WAL mode).

CREATE TABLE IF NOT EXISTS numeric_telemetry (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    entity_id TEXT NOT NULL,
    ts INTEGER NOT NULL,
    value REAL NOT NULL,
    unit TEXT
);

CREATE INDEX IF NOT EXISTS idx_numeric_telemetry_entity_ts
    ON numeric_telemetry (entity_id, ts);

CREATE TABLE IF NOT EXISTS entity_state_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    entity_id TEXT NOT NULL,
    ts INTEGER NOT NULL,
    state TEXT NOT NULL,
    attributes_json TEXT
);

CREATE INDEX IF NOT EXISTS idx_entity_state_history_entity_ts
    ON entity_state_history (entity_id, ts);

CREATE TABLE IF NOT EXISTS automation_runs (
    run_id TEXT PRIMARY KEY,
    automation_id TEXT NOT NULL,
    trigger_source TEXT NOT NULL,
    conditions_passed INTEGER NOT NULL,
    outcome TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    finished_at INTEGER NOT NULL,
    steps_json TEXT NOT NULL,
    error TEXT
);

CREATE INDEX IF NOT EXISTS idx_automation_runs_auto_started
    ON automation_runs (automation_id, started_at DESC);

CREATE INDEX IF NOT EXISTS idx_automation_runs_started
    ON automation_runs (started_at DESC);
