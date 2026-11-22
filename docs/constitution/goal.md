# Goal

## Project Vision

**Xiaohe** (*萧何*, named after the legendary Han Dynasty chief steward who mastered state records, logistics, and delegating to specialists) is a lightweight, self-hosted smart home control platform designed as a lean alternative to Home Assistant. The core state engine and automation controller are built in **Rust** for predictability and minimal resource consumption, the real-time web interface is built with **SolidJS**, and all third-party device integrations and complex control logic are extended through **AI- and human-friendly Skills** — self-contained packages that wrap third-party Python libraries or custom device protocols in isolated runtimes without weighing down the core.

---

## Core Goals

### 1. Lean, Rock-Solid Rust Core (State & Automation)
- Model core smart home domains (`light`, `switch`, `climate`, `sensor`, `binary_sensor`, `cover`, `lock`, `media_player`, `button`, etc.) with strongly typed Rust `enum`s and exhaustive `match` handling.
- Drive millisecond-level state transitions and automation evaluation over an in-process `tokio` event bus.
- Ship the core service as a fully static binary in a `scratch` container image with a tiny memory footprint, ideal for Raspberry Pi, mini PCs, or a home NAS.

### 2. AI- & Human-Friendly Skill Extension System
- **Unified Package Spec**: Every Skill is a self-contained directory (`skill.toml` manifest + `SKILL.md` specification/guide + source code). Its explicit, boilerplate-free contract allows both human developers and AI coding agents to author and debug new device adapters rapidly.
- **Dual Lifecycle Modes**:
  1. **Long-running (Device / Bridge) Skills** that maintain persistent device connections, poll/subscribe to telemetry, publish entity states, and execute device commands.
  2. **On-demand (Action / Service) Skills** for notifications, external lookups, and complex control algorithms invoked by automations or the API.
- **Strict Python Isolation**: Python Skills run inside a dedicated `xiaohe-runtime` container, where `uv` manages an isolated virtual environment per Skill and communication with the core happens over JSON-RPC. Dependency conflicts, memory leaks, or crashes in a third-party Python library never affect the Rust core.

### 3. Expression-Free Declarative TOML Automations
- Author all automations in unambiguous, strongly typed **TOML** files — eliminating YAML indentation pitfalls and implicit type coercion (`on`/`off`/`yes`/`no`).
- **No embedded expression or scripting engine** (no Jinja2, Rhai, or Lua) in the core. Keep core automation evaluation deterministic, statically validatable, and easy to edit via the UI. Delegate any complex branching, math, or stateful orchestration directly to Action Skills.

### 4. Pluggable Storage via Data Access Abstraction Layer (DAL)
- Store all configuration, areas, devices, and automation rules in human-readable TOML files on disk (Git-friendly, hand-editable, atomic write-back).
- Abstract state history, sensor telemetry, and automation audit logs behind an async Rust `HistoryStore` trait: default to zero-dependency embedded **SQLite (WAL mode)**, with a pluggable **InfluxDB** backend for long-term high-frequency time-series workloads.

### 5. Disciplined Native Protocol Boundary
- **No MQTT**: Do not introduce or require an MQTT broker (no Mosquitto container, no untyped topic conventions).
- The Rust core natively supports only **ESPHome Native API** (encrypted local TCP/Protobuf for DIY sensors & actuators) and **HTTP Webhooks** (external triggers). Every other vendor or proprietary protocol (MiHome/miio, Tuya, Midea, Hue, HomeKit, Matter bridges, etc.) lives in the Skill layer.

### 6. Real-Time, Lightweight SolidJS Web UI
- Deliver a fast, mobile-friendly SPA built with SolidJS, TypeScript, Tailwind CSS v4, uPlot, and REST + SSE (`GET /api/events`), embedded directly into the Rust binary.
- MVP includes four core views:
  1. Area/room-grouped device control dashboard (auto-rendered per entity domain)
  2. Entity history charts (uPlot) and discrete state timelines
  3. Skill Manager (install/start/stop/reload, config/secrets, health, live logs)
  4. TOML Automation Manager (list, toggle, manual run, validated TOML editor, execution audit logs)

### 7. Multi-User Household RBAC
- Support three roles — **Admin**, **Member**, and **Guest** — with area-scoped device visibility and control permissions.
- Admins manage system settings, Skills, automations, and users; Members and Guests only see and control entities in their permitted areas, enforced identically across REST endpoints and SSE event streams.

---

## Non-Goals

- **Multi-tenant / Cloud SaaS** — Strictly self-hosted for a single household.
- **Drop-in Home Assistant Python integration compatibility** — Xiaohe does not load HA `custom_components`; instead, Skills wrap underlying third-party Python libraries (e.g., `python-miio`, `tinytuya`) directly via a clean JSON-RPC contract.
- **MQTT broker or MQTT-based internal bus** — Excluded by design. Users who must bridge legacy MQTT devices can write a dedicated Skill.
- **In-core template or scripting engine** — No Jinja2, Lua, Rhai, or JS evaluator inside `xiaohe-core`.
- **Native iOS / Android apps** — The web UI is responsive and PWA-friendly; native mobile apps are out of scope for MVP.
- **Drag-and-drop floorplan / freeform dashboard builder (MVP)** — Deferred beyond MVP; MVP prioritizes clean, automatic area-grouped domain cards.

---

## Success Criteria

1. **Two-container turnkey deployment**: `docker compose up` launches `xiaohe-core` (`scratch` image < 25 MB, resident memory < 50 MB) and `xiaohe-runtime` (with `uv`) on a Raspberry Pi 4 or x86_64 host with zero external database setup required.
2. **Rapid Skill authoring**: Using `SKILL.md` and the Skill specification, a developer or AI coding agent can wrap a third-party Python device library into a working Skill in under 15 minutes and immediately see auto-rendered control cards in the web UI.
3. **Fault isolation**: Killing or crashing a Python Skill process has zero impact on `xiaohe-core`, ESPHome connections, automations, or the web UI; the supervisor restarts it with exponential backoff and surfaces its health and logs in the UI.
4. **Low-latency live updates**: State changes from ESPHome devices or Skills propagate to connected SolidJS clients via SSE in under 100 ms with no polling.
5. **Strict RBAC enforcement**: Guest and Member accounts cannot view, control, or receive SSE events for entities outside their authorized areas, nor access admin endpoints.
6. **Pluggable storage parity**: Switching `HistoryStore` between `sqlite` and `influxdb` in `config/system.toml` passes the full storage integration test suite with zero changes to core business logic.
