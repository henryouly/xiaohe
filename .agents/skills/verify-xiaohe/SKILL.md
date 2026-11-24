---
name: verify-xiaohe
description: "Project-local verification skill that launches, health-checks, drives, captures evidence from, and cleans up Xiaohe (CLI, HTTP health/webhook endpoints, TOML config, SQLite WAL persistence). Use whenever proving a feature or verifying a change against the real xiaohe binary."
---

# Verify Xiaohe

This skill drives the real `xiaohe` Rust binary against disposable config and SQLite directories to prove user-facing behavior. Read [features/README.md](features/README.md) first, then follow the matching feature recipe.

## Launch

Build the binary once (if not already built) and launch an isolated `xiaohe core` instance with a unique `RUN_ID` and free loopback port:

```bash
export RUN_ID="run-$(date +%s)-$$"
export PORT=14000
export SCRATCH_DIR="/tmp/xiaohe-verify-${RUN_ID}"
export EVIDENCE_DIR="/tmp/xiaohe-evidence-${RUN_ID}"

.agents/skills/verify-xiaohe/scripts/xiaohe-verify.sh launch "$RUN_ID" "$PORT"
```

- Starts from an empty disposable `CONFIG_DIR` (`$SCRATCH_DIR/config`) and `SQLITE_PATH` (`$SCRATCH_DIR/data/xiaohe.db`).
- Seeds verification scaffolding (`system.toml` with `verify_hook` enabled and `disabled_hook` disabled).
- Waits until `GET http://127.0.0.1:$PORT/health/ready` returns `200` with `"storage":"ok"` (meaning SQLite migrations have completed and the server is usable).
- Multiple runs are fully isolated by `RUN_ID` and `PORT`; never reuse another session's `RUN_ID` or `PORT`.
- For short-lived CLI drives (`xiaohe --help`, `xiaohe supervisor`, malformed-config exit checks), run `./target/debug/xiaohe` directly with a disposable `CONFIG_DIR` inside `$SCRATCH_DIR`.

## Doctor

Run this read-only check before the first drive and after any unexpected failure:

```bash
.agents/skills/verify-xiaohe/scripts/xiaohe-verify.sh doctor "$RUN_ID"
```

Confirms:
1. Recorded PID in `$SCRATCH_DIR/core.pid` is alive.
2. `GET http://127.0.0.1:$PORT/health/ready` returns `200`.
3. `$SCRATCH_DIR/data/xiaohe.db` exists on disk.

## Drive

Use `curl -sS` against `http://127.0.0.1:$PORT` for HTTP routes, `./target/debug/xiaohe` for CLI subcommands, and `sqlite3` (or `python3 -c 'import sqlite3...'`) for read-only DB side-effect verification. Follow the exact recipes in [features/README.md](features/README.md).

## Evidence

Write all proof artifacts into `$EVIDENCE_DIR` (`/tmp/xiaohe-evidence-$RUN_ID/`, outside `$SCRATCH_DIR`):

- Exercise the real user path (HTTP requests, CLI invocations, real TOML files), never test-only endpoints.
- Capture both the action and the resulting state (HTTP status + body, CLI stdout/stderr + exit code, `core.log` lines, and SQLite query output).
- Verify side effects (SQLite `PRAGMA journal_mode`, tables created, files written) alongside HTTP responses.
- Mocks are only allowed at real external boundaries (e.g., an external ESPHome device or external webhook sender).

## Cleanup

Always tear down what this run started — even after a failed drive:

```bash
.agents/skills/verify-xiaohe/scripts/xiaohe-verify.sh cleanup "$RUN_ID"
```

- Sends `SIGTERM` strictly to the PID recorded in `$SCRATCH_DIR/core.pid` (never `pkill xiaohe`).
- Removes `$SCRATCH_DIR` (process state, disposable config, disposable SQLite DB).
- **Preserves `$EVIDENCE_DIR`** — verify evidence files still exist in `$EVIDENCE_DIR` after cleanup.

## Helpers

- [.agents/skills/verify-xiaohe/scripts/xiaohe-verify.sh](scripts/xiaohe-verify.sh) (`chmod +x`):
  - `launch <run_id> [port]` — builds if needed, seeds disposable config, starts `xiaohe core`, polls `/health/ready`.
  - `doctor <run_id>` — read-only PID, `/health/ready`, and SQLite file check.
  - `cleanup <run_id>` — sends `SIGTERM` to recorded PID and deletes `$SCRATCH_DIR` while keeping `$EVIDENCE_DIR`.
