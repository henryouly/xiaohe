---
name: verify-xiaohe
description: "Project-local verification skill that launches, health-checks, drives, captures evidence from, and cleans up Xiaohe (CLI, TOML config, SQLite WAL persistence, health/webhook endpoints, supervisor + Python Skills over UDS, declarative TOML automations, multi-user Auth/RBAC, full REST API, RBAC-filtered SSE, OpenAPI 3.1 docs, and SolidJS SPA static serving). Use whenever proving a feature or verifying a change against the real xiaohe binary."
---

# Verify Xiaohe

This skill drives the real `xiaohe` Rust binary (`xiaohe supervisor`, `xiaohe core`, and `xiaohe validate`) against disposable config and SQLite directories to prove user-facing behavior across Phases 1–7. Read [features/README.md](features/README.md) first, then follow the matching feature recipe.

## Launch

Build the binary once (if not already built) and launch an isolated `xiaohe supervisor` + `xiaohe core` pair with a unique `RUN_ID` and free loopback port:

```bash
export RUN_ID="run-$(date +%s)-$$"
export PORT=14000
export SCRATCH_DIR="/tmp/xiaohe-verify-${RUN_ID}"
export EVIDENCE_DIR="/tmp/xiaohe-evidence-${RUN_ID}"

.agents/skills/verify-xiaohe/scripts/xiaohe-verify.sh launch "$RUN_ID" "$PORT"
```

- Starts from an empty disposable `CONFIG_DIR` (`$SCRATCH_DIR/config`), `SQLITE_PATH` (`$SCRATCH_DIR/data/xiaohe.db`), and `RUNTIME_SOCKET` (`$SCRATCH_DIR/rpc.sock`).
- Seeds verification scaffolding:
  - `system.toml` with `verify_hook` enabled and `disabled_hook` disabled.
  - `devices.toml` enabling `miio-airpurifier` in `demo_mode = true`.
  - `automations/webhook_purifier.toml` (`verify_webhook_purifier`).
- Starts `xiaohe supervisor` (scanning `SKILLS_DIR=./skills`) and waits for `$SCRATCH_DIR/rpc.sock`, then starts `xiaohe core`.
- Waits until `GET http://127.0.0.1:$PORT/health/ready` returns `200` with `"runtime":"ok"` (confirming SQLite migrations completed and the Core ↔ Supervisor UDS bridge is connected).
- Multiple runs are fully isolated by `RUN_ID` and `PORT`; never reuse another session's `RUN_ID` or `PORT`.
- For short-lived CLI drives (`xiaohe --help`, malformed-config exit checks) or standalone `xiaohe supervisor` tests (which must be stopped with `SIGTERM`), run `./target/debug/xiaohe` directly with a disposable `CONFIG_DIR` inside `$SCRATCH_DIR`.

## Doctor

Run this read-only check before the first drive and after any unexpected failure:

```bash
.agents/skills/verify-xiaohe/scripts/xiaohe-verify.sh doctor "$RUN_ID"
```

Confirms:
1. Recorded PIDs in `$SCRATCH_DIR/core.pid` and `$SCRATCH_DIR/supervisor.pid` are alive.
2. UDS socket `$SCRATCH_DIR/rpc.sock` exists.
3. `GET http://127.0.0.1:$PORT/health/ready` returns `200` with `"runtime":"ok"`.
4. `$SCRATCH_DIR/data/xiaohe.db` exists on disk.

## Drive

Use `curl -sS` against `http://127.0.0.1:$PORT` for HTTP/SSE routes, `./target/debug/xiaohe` for CLI subcommands, and `sqlite3` (or `python3 -c 'import sqlite3...'`) for read-only DB side-effect verification. Follow the exact recipes in [features/README.md](features/README.md).

## Evidence

Write all proof artifacts into `$EVIDENCE_DIR` (`/tmp/xiaohe-evidence-$RUN_ID/`, outside `$SCRATCH_DIR`):

- Exercise the real user path (HTTP/SSE requests, CLI invocations, real TOML files), never test-only endpoints.
- Capture both the action and the resulting state (HTTP status + body, CLI stdout/stderr + exit code, `core.log` / `supervisor.log` lines, and SQLite query output).
- Verify side effects (SQLite `PRAGMA journal_mode`, tables created, files written, `0600` permissions on `users.toml`/`secrets.toml`) alongside HTTP responses.
- Mocks are only allowed at real external boundaries (e.g., an external ESPHome device or external webhook sender).

## Cleanup

Always tear down what this run started — even after a failed drive:

```bash
.agents/skills/verify-xiaohe/scripts/xiaohe-verify.sh cleanup "$RUN_ID"
```

- Sends `SIGTERM` strictly to the PIDs recorded in `$SCRATCH_DIR/core.pid` and `$SCRATCH_DIR/supervisor.pid` (with `SIGKILL` fallback after 3s; never `pkill xiaohe`).
- Removes `$SCRATCH_DIR` (process state, disposable config, UDS socket, disposable SQLite DB).
- **Preserves `$EVIDENCE_DIR`** — verify evidence files still exist in `$EVIDENCE_DIR` after cleanup.

## Helpers

- [.agents/skills/verify-xiaohe/scripts/xiaohe-verify.sh](scripts/xiaohe-verify.sh) (`chmod +x`):
  - `launch <run_id> [port]` — builds if needed, seeds disposable config, starts `xiaohe supervisor` + `xiaohe core`, polls `/health/ready` for `"runtime":"ok"`.
  - `doctor <run_id>` — read-only check of `core.pid`, `supervisor.pid`, `rpc.sock`, `/health/ready` (`"runtime":"ok"`), and `xiaohe.db`.
  - `cleanup <run_id>` — sends `SIGTERM` to recorded `core.pid` and `supervisor.pid` and deletes `$SCRATCH_DIR` while keeping `$EVIDENCE_DIR`.
