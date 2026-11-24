# Xiaohe verification map (example)

This directory is the maintained source for verifying the user-facing behavior of Xiaohe. Read this index before driving the app, then use the matching feature file as the recipe.

## Baseline preconditions

- Build the binary once (`cargo build`) or use the built binary at `./target/debug/xiaohe`.
- Launch `xiaohe core` on an isolated loopback port (e.g. `HOST=127.0.0.1 PORT=14000`) with disposable `CONFIG_DIR=/tmp/xiaohe-verify-$RUN_ID/config` and `SQLITE_PATH=/tmp/xiaohe-verify-$RUN_ID/data/xiaohe.db`.
- Store proof artifacts in `/tmp/xiaohe-evidence-$RUN_ID` (outside the disposable scratch dir so cleanup never removes evidence).
- Record the server PID in `/tmp/xiaohe-verify-$RUN_ID/core.pid` and run the `Doctor` check (`GET /health/ready` returning `200` + PID alive) before driving.
- Never drive an instance that was not started by this verification run.

## Driving conventions

- Start every recipe from the baseline state unless its preconditions say otherwise.
- Treat every command as literal. Keep quoted headers, JSON payloads, and flags unchanged.
- Send HTTP requests via `curl -sS` capturing status code and body.
- Verify side effects directly (SQLite tables via `sqlite3`, TOML files on disk, process exit code on `SIGTERM`).
- Restore or discard scratch fixtures during cleanup, never the proof artifacts.

## Proof and skip reporting

- Capture the user action and the resulting state, not only a status code.
- HTTP proof includes the request route/method, status code, response body, and any persisted side effect or log line.
- CLI proof includes the command, stdout, stderr, and exit code.
- Record the feature ID and entry point used with every artifact.
- Report an unreachable path with the attempted command and the unmet precondition.
- Do not report a skipped entry point as verified through a different path.

## Feature entry contract

Each feature file starts with an H1 title and one paragraph describing the user-visible behavior. It then uses exactly four H2 sections in this order:

1. `Sub-features` lists short IDs with one line for each behavior.
2. `How to get to it (user POV)` lists every user entry point.
3. `Driving it with <harness>` starts with `Preconditions:` and uses labeled bullets that pair each user action with an exact command and observable result.
4. `Gotchas` lists traps that can waste or invalidate a verification run.

Keep implementation details out of the map. Name only user paths, stable handles, required state, commands, and observable proof.

## Features

- [HTTP Webhooks](./webhooks.md) covers authenticated webhook ingestion via Bearer and `X-Webhook-Secret`, rejection of invalid/oversized requests, and header sanitization.
