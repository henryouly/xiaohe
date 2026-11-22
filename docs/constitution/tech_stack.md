# Tech Stack

This stack builds on the lean, strongly typed, self-hosted philosophy of `tesla-apiscraper-rs`, adapted for Xiaohe's two-container Skill isolation, pluggable storage DAL, and multi-user RBAC requirements.

---

## Core Backend (`xiaohe-core`)

### Language & Runtime

| Component | Choice | Rationale |
|-----------|--------|-----------|
| **Language** | Rust (stable) | Zero-cost abstractions, no GC, exhaustive `match` on entity domains and state transitions. Single static binary with minimal memory footprint — ideal for Raspberry Pi and home servers. |
| **Compiler** | `rustc` via `cargo` | Workspace build with cross-compilation for `x86_64` and `aarch64` (`musl` static linking). |
| **Async Runtime** | `tokio` (multi-threaded, work-stealing) | Drives the HTTP/SSE server, ESPHome TCP tasks, Skill Supervisor RPC bridge, timers, and in-process event bus. |
| **Build** | `cargo build --release` + Docker multi-stage | Multi-stage Dockerfile (`cargo-chef` + `sccache` → `scratch` runtime) producing a fully static binary with embedded frontend assets. |

### Core Framework & Libraries

| Concern | Choice | Rationale |
|---------|--------|-----------|
| **Web Framework** | `axum` | Tower-based, type-safe extractors, first-class SSE (`axum::response::Sse`), clean middleware layers for RBAC and request tracing. |
| **Internal Event Bus** | `tokio::sync::broadcast` + `mpsc` | Zero-dependency in-process pub/sub for state updates, automation triggers, history persistence, and SSE fan-out. |
| **ESPHome Native API** | `prost` (Protobuf) + `snow` (Noise XX) + `tokio::net::TcpStream` | Direct Rust implementation of ESPHome's encrypted native TCP protocol with entity discovery, live state streaming, command dispatch, and exponential-backoff reconnect. |
| **HTTP Client** | `reqwest` | Async HTTP client for the optional InfluxDB backend and outbound HTTP calls if needed. |
| **Serialization & Config** | `serde` + `serde_json` + `toml` | `toml` for all configuration files and automation rules; `serde_json` for REST API, SSE payloads, and JSON-RPC 2.0. |
| **Startup Configuration** | `figment` | Merges environment variables with `config/system.toml`, providing defaults and startup validation. |
| **OpenAPI Documentation** | `utoipa` + `utoipa-swagger-ui` | Derives OpenAPI 3.1 schemas directly from Rust types and `axum` handlers; Swagger UI served at `/docs`. |
| **Auth & Passwords** | `argon2` + `rand` + `axum-extra` (Cookies) | Argon2id password hashing; `HttpOnly` + `SameSite=Lax` session cookies for browsers; high-entropy Bearer tokens (SHA-256 hashed at rest) for webhooks and API clients. |
| **Secrets Storage** | `config/secrets.toml` (`0600`) + `${ENV_VAR}` | Plain TOML secrets file protected by `0600` permissions, `.gitignore`, redacted logging, write-only REST API, and per-Skill scoped injection; supports `${ENV_VAR}` resolution for Docker secrets / `.env`. |
| **Structured Logging** | `tracing` + `tracing-subscriber` | Structured span-based logging (JSON in production, compact text in dev) tagged with `entity_id`, `skill_id`, `automation_id`, and `user_id`. |
| **Error Handling** | `thiserror` + `anyhow` | `thiserror` for exhaustive domain error enums; `anyhow` for application-level error propagation. |
| **Time & Scheduling** | `chrono` + `chrono-tz` + `cron` | Timezone-aware schedules, sunrise/sunset offsets, `for` duration debouncing, and RFC3339 timestamps. |
| **Static Asset Embedding** | `rust-embed` | Embeds Vite's `web/dist` output directly into the Rust binary so `xiaohe-core` serves the SPA without an external web server. |
| **Testing** | `#[test]` + `rstest` + `wiremock` + `tokio::test` | Parameterized unit tests, async state-machine/automation tests, and RPC/HTTP contract tests. |

---

## Data Storage & Abstraction Layer (DAL)

Configuration and historical telemetry are cleanly separated: configuration lives in human-readable **TOML files**, while entity history, numeric telemetry, and automation audit logs are accessed through an async Rust **`HistoryStore` trait**.

### TOML Configuration Files

| Concern | Choice | Rationale |
|---------|--------|-----------|
| **Format** | TOML via `toml` | Native to the Rust ecosystem; avoids YAML indentation bugs and implicit boolean coercion (`on`/`off`). Atomic write-back via temp file + `rename`. |
| **Files** | `config/system.toml`, `config/areas.toml`, `config/users.toml`, `config/devices.toml`, `config/secrets.toml`, `config/automations/*.toml` | Persisted on a Docker volume, easy to inspect or version-control with Git. `secrets.toml` is `.gitignore`d and stored with `0600` permissions (with optional `${ENV_VAR}` references). |

### History & Telemetry Backends (`HistoryStore` Trait)

| Backend | Crate / Driver | Status | Rationale |
|---------|----------------|--------|-----------|
| **SQLite (Default)** | `sqlx` (sqlite, WAL mode, bundled static `libsqlite3`) | **Default** | Zero external database container, single-file persistence, strong read/write concurrency under WAL mode, ideal for household-scale history and audit logs. |
| **InfluxDB (Optional)** | `reqwest` (HTTP) + `influxdb` crate (line protocol) | **Pluggable** | Follows `tesla-apiscraper-rs` patterns for users who already run InfluxDB v1/v3 and want multi-year high-frequency analytics or external Grafana dashboards. |

---

## Skill Runtime (`xiaohe-runtime`)

A dedicated companion container that hosts third-party Python libraries and multi-language Skill subprocesses, isolating them completely from `xiaohe-core`.

| Concern | Choice | Rationale |
|---------|--------|-----------|
| **Base Image** | `debian:trixie-slim` | Compact Debian 13 image with full `glibc`, ensuring prebuilt PyPI wheels with C/Rust extensions (`cryptography`, `python-miio`, `numpy`, etc.) install cleanly without compiling from source. |
| **Python Env Manager** | `uv` (Astral) | Ultra-fast dependency resolution and installation; creates an isolated `.venv` per Python Skill so third-party packages never conflict. |
| **Skill Supervisor** | `xiaohe supervisor` (subcommand of the unified `xiaohe` Rust binary) | Scans `/skills`, provisions per-skill `uv` environments, spawns/supervises child processes, enforces health checks and exponential-backoff restarts, and streams `stderr` logs. |
| **Core ↔ Supervisor Transport** | JSON-RPC 2.0 over Unix Domain Socket (shared volume, default) or internal Docker TCP | UDS gives minimal latency and zero exposed network ports on a single host; TCP mode allows future multi-host scaling. |
| **Supervisor ↔ Skill Transport** | JSON-RPC 2.0 over `stdin`/`stdout` (NDJSON) | Simple, MCP/LSP-style language-agnostic wire protocol. Any language (Python, Node, Go, Rust) can implement a Skill simply by reading/writing JSON lines. |
| **Python Skill SDK** | Lightweight `xiaohe-sdk` helper (optional) | Optional thin async helper for JSON-RPC framing, heartbeats, and typed payloads; trivial Skills can also use pure Python stdlib. |

---

## Frontend (`web/`)

Follows `tesla-apiscraper-rs`'s lean, fine-grained reactive frontend architecture.

| Concern | Choice | Rationale |
|---------|--------|-----------|
| **Framework** | SolidJS | Fine-grained reactivity with no virtual DOM — compiles to direct DOM mutations, ideal for dashboards with dozens of live-updating entity cards. |
| **Language** | TypeScript (strict) | End-to-end type safety aligned with backend OpenAPI schemas. |
| **Build Tool** | Vite | Fast HMR and optimized production builds bundled into `xiaohe-core` via `rust-embed`. |
| **Routing** | `@solidjs/router` | Official lightweight router with lazy routes and role-based route guards. |
| **CSS Framework** | Tailwind CSS v4 (`@tailwindcss/vite`) | Utility-first styling with zero CSS runtime overhead and clean responsive layouts. |
| **Charts** | `uPlot` + custom Tailwind state-timeline components | High-performance, tiny time-series plots without heavy charting frameworks. |
| **Real-Time Updates** | Server-Sent Events (`GET /api/events`) | Unidirectional server→client streaming for live entity states, Skill health, and automation runs; commands and CRUD use REST. |
| **Icons** | `lucide-solid` | Lightweight, tree-shakeable SVG icon set. |
| **Bundle Size Target** | < 200 KB gzipped | Fast initial load on mobile devices over LAN or reverse proxy. |

---

## Deployment

| Concern | Choice | Rationale |
|---------|--------|-----------|
| **Orchestration** | Docker Compose (`compose.yml`) | Standard two-container stack: `xiaohe-core` + `xiaohe-runtime`, plus an optional `influxdb` compose profile. |
| **Core Container** | `scratch` static image (~15–25 MB) | No shell, no Python, minimal attack surface. Mounts `/config`, `/data` (SQLite), and shared socket volume `/run/xiaohe`. |
| **Runtime Container** | `debian:trixie-slim` + `uv` + `xiaohe supervisor` | Mounts `/skills`, `/run/xiaohe`, and `/var/cache/uv`. Supports optional `network_mode: host` for LAN broadcast/mDNS device discovery (e.g., MiHome, Tuya, Hue). |
| **Port** | `4000` (Web UI, REST, SSE, Webhooks) | Single port for simple reverse-proxy setups (Caddy, Nginx, Tailscale). |
| **Health Checks** | `GET /health` (liveness) & `GET /health/ready` (readiness: Storage + Runtime bridge) | Standard Docker and orchestrator health probes. |

---

## What We Won't Use (and Why)

| Library / Tool | Why Not |
|----------------|---------|
| **MQTT / Mosquitto** | Explicitly excluded to avoid running an extra broker and untyped topic schemas. DIY LAN devices use native ESPHome API; all other devices use Skills. |
| **YAML** | Error-prone indentation and implicit boolean coercion (`on`/`off`/`yes`/`no`). TOML is cleaner, unambiguous, and idiomatic in Rust. |
| **Expression / Scripting Engine (Jinja2 / Rhai / Lua)** | Adds parser complexity, runtime overhead, and security surface to the core. Core automations stay purely declarative TOML; complex logic belongs in Action Skills. |
| **PyO3 (In-process Python)** | Sharing one Python interpreter and GIL inside the Rust core risks dependency conflicts and C-extension crashes bringing down the entire hub. |
| **Actix-Web / Rocket** | `axum` works on stable Rust, has no actor-model boilerplate, and provides first-class Tower middleware and SSE support. |
| **Diesel / SeaORM** | Heavy ORM abstraction is unnecessary and fights our pluggable SQLite/InfluxDB DAL. Direct `sqlx` queries behind `HistoryStore` stay thin and explicit. |
| **GraphQL / Browser gRPC** | REST + SSE is simpler, browser-native, and directly documentable via OpenAPI 3.1. |
| **React / Vue / Svelte** | SolidJS delivers smaller bundles and direct fine-grained DOM updates for live telemetry dashboards. |
| **Redis / External Message Queues** | In-process `tokio` channels easily handle household event throughput without extra infrastructure. |
