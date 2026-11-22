# Roadmap

Each phase is a self-contained, verifiable deliverable ordered by architectural dependency: core foundation and storage DAL first, then entity state and native protocols, followed by the isolated Skill runtime, declarative TOML automation engine, multi-user RBAC, SolidJS frontend, and pluggable storage backends.

---

## Phase 1: Project Foundation & Storage DAL

**Goal:** A working two-container scaffold where `xiaohe-core` loads TOML configs, initializes the default SQLite `HistoryStore`, and passes health checks.

### 1.1 Project & Build Scaffolding
- Initialize single-crate Rust project `xiaohe` with `src/protocol` (shared domain & JSON-RPC types), `xiaohe core`, and `xiaohe supervisor` CLI subcommands.
- Add `Makefile` (`build`, `test`, `lint`, `web-build`, `docker-build`) and GitHub Actions CI (`cargo fmt`, `clippy`, `test`, `tsc`, `vite build`).
- Author `Dockerfile.core` (`rust:alpine` + `cargo-chef` → `scratch` static binary), `Dockerfile.runtime` (`debian:trixie-slim` + `uv` + `xiaohe supervisor`), and reference `compose.yml`.

### 1.2 TOML Configuration & Encrypted Secrets
- Implement typed config loaders and atomic disk writers (temp file + `rename`) for `config/system.toml`, `config/areas.toml`, `config/users.toml`, `config/devices.toml`, and `config/secrets.toml`.
- Implement AES-256-GCM encryption/decryption helpers (`aes-gcm` + `ring`) keyed by `XIAOHE_MASTER_KEY`.

### 1.3 Storage Data Access Layer (DAL)
- Define the async `HistoryStore` trait (state transitions, numeric sensor samples, range/downsampled queries, automation run audit logs).
- Implement default `SqliteStore` (`sqlx`, WAL mode, automatic migrations, configurable retention pruning).
- Wire up the `axum` HTTP skeleton with `tracing`, `GET /health`, `GET /health/ready`, and graceful SIGTERM/SIGINT shutdown.

---

## Phase 2: Core Entity State Engine & Native Protocols

**Goal:** The Rust state engine runs live, ingests native ESPHome devices and HTTP Webhooks, and records state history.

### 2.1 Strongly Typed Entity & State Store
- Implement core domain enums (`Light`, `Switch`, `Climate`, `Sensor`, `BinarySensor`, `Cover`, `Lock`, `MediaPlayer`, `Button`, `Custom`) and `EntityRecord` (with `extra` JSON attributes and `custom_actions`).
- Implement the thread-safe in-memory `StateStore` and `EventBus` (`tokio::sync::broadcast`), persisting state changes asynchronously to `HistoryStore`.

### 2.2 ESPHome Native API Client
- Compile ESPHome Protobuf definitions (`prost`) and implement Noise XX encrypted framing (`snow`).
- Spawn one `tokio` task per configured ESPHome node: entity discovery mapped to core domains, live state streaming, command dispatch, keep-alive pings, and exponential-backoff reconnection.

### 2.3 HTTP Webhook Ingestion
- Implement `POST /api/webhooks/{webhook_id}` with per-webhook secret verification, structured JSON payload extraction, and `WebhookEvent` publication to `EventBus`.

---

## Phase 3: Skill Runtime & Isolated Python Execution

**Goal:** End-to-end communication between `xiaohe-core`, `xiaohe-supervisor`, and isolated Python Skills wrapping third-party libraries.

### 3.1 Supervisor & `uv` Environment Isolation
- `xiaohe-supervisor` scans `/skills/*/skill.toml`, validates manifests, and provisions per-skill virtual environments via `uv sync` / `uv venv`.
- Manage child process lifecycles: start, stop, reload, heartbeat liveness checks, exponential-backoff crash recovery, and ring-buffered `stderr` log capture.

### 3.2 JSON-RPC 2.0 Bridge
- Implement bidirectional JSON-RPC 2.0 over Unix Domain Socket (shared volume) between `xiaohe-core` and `xiaohe-supervisor`.
- Support both **Device/Bridge Skills** (long-running: register devices/entities, emit `entity.state_changed`, handle `entity.command`) and **Action/Service Skills** (on-demand: handle `skill.invoke` and return structured JSON).
- Implement scoped secret injection so each Skill receives only the decrypted secrets declared in its `skill.toml`.

### 3.3 Reference Skills & AI Authoring Kit
- Publish lightweight `xiaohe-sdk` (Python) helper.
- Deliver two reference Skills with complete `SKILL.md` guides:
  1. A long-running device Skill wrapping a third-party Python library (e.g., `python-miio` air purifier / smart plug).
  2. An on-demand action Skill (e.g., notification or multi-entity control routine).

---

## Phase 4: Declarative TOML Automation Engine

**Goal:** Expression-free declarative TOML automations execute deterministically in Rust, delegating complex logic to Action Skills.

### 4.1 TOML Parser & Static Validator
- Load `config/automations/*.toml` with startup and hot-reload validation (verifying referenced `entity_id`s, `skill_id`s, domain command schemas, and time formats).
- Support triggers (`state`, `numeric_state`, `time`, `sun`, `webhook`, `event`), conditions (`state`, `numeric_state`, `time`, `and`/`or`/`not`), and actions (`entity.call`, `skill.invoke`, `delay`, `parallel`).

### 4.2 Execution Engine & Audit Logging
- Implement `for` duration debounce timers and concurrency modes (`single`, `restart`, `queued`, `parallel`).
- Persist every automation run (trigger source, condition result, step outcomes, duration, errors) to `HistoryStore` and broadcast run events over `EventBus`.

---

## Phase 5: Multi-User RBAC & Complete REST / SSE API

**Goal:** Enforce household role- and area-based access control across all REST endpoints and SSE streams.

### 5.1 Multi-User Auth & Area-Scoped RBAC
- Implement `Admin`, `Member`, and `Guest` roles with explicit `allowed_areas` lists.
- Implement Argon2id password auth, `HttpOnly` session cookies, API Bearer tokens, and first-boot Admin bootstrap flow.
- Enforce area-scoped filtering in `axum` middleware and SSE stream subscriptions.

### 5.2 Full REST + SSE Contract & OpenAPI
- Complete REST endpoints for Areas, Devices, Entities, Commands, History, Skills, Automations, and Users.
- Implement `GET /api/events` SSE stream with 15s keep-alive, `resync` lag notifications, and per-user RBAC filtering.
- Expose auto-generated OpenAPI 3.1 documentation via `utoipa` at `/docs`.

---

## Phase 6: SolidJS Web Frontend (MVP)

**Goal:** Ship a < 200 KB gzipped, mobile-responsive SolidJS web app embedded in `xiaohe-core`.

### 6.1 App Shell & Reactive Store
- Scaffold SolidJS + TypeScript + Vite + Tailwind CSS v4 + `@solidjs/router`.
- Build the global reactive SSE store with strict server-timestamp snapshot merging, automatic reconnect, and role-based route guards.

### 6.2 Four MVP Views
1. **Area Control Dashboard**: Entities grouped by area with domain-specific controls (dimmer/color temp for lights, thermostat dial for climate, lock confirmation, sensor readouts).
2. **History & Timeline**: `uPlot` numeric sensor graphs with selectable time ranges and discrete state timelines.
3. **Skill Manager (Admin)**: Skill status cards, `uv` sync status, start/stop/reload controls, config & secrets editor, and live log viewer.
4. **Automations & Run Logs (Admin)**: Enable/disable toggles, manual trigger button, TOML editor with server-side validation feedback, and execution history table.

---

## Phase 7: Pluggable Storage Backend & Ecosystem Polish

**Goal:** Validate the storage abstraction with a second backend and polish operational tooling.

### 7.1 InfluxDB Storage Backend
- Implement `InfluxDbStore` for the `HistoryStore` trait and add an optional `influxdb` profile to `compose.yml`.

### 7.2 CLI Diagnostics & Skill Templates
- Add `xiaohe validate` CLI subcommand for offline TOML config and automation linting.
- Publish starter Skill templates and an AI authoring prompt/guide so users can generate new device Skills reliably.
