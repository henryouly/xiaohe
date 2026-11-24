# Xiaohe verification map

This directory is the maintained source for verifying the implemented user-facing behavior of Xiaohe (Phases 1–4: CLI, TOML configuration, SQLite WAL persistence, health probes, HTTP webhook ingestion, `xiaohe supervisor` ↔ UDS JSON-RPC 2.0 ↔ isolated Python Skills, and declarative TOML automations). Read this index before driving the app, then follow the matching feature file.

## Baseline preconditions

- Build `./target/debug/xiaohe` via `cargo build` (or let `xiaohe-verify.sh launch` build it).
- Launch an isolated `xiaohe supervisor` + `xiaohe core` pair with `.agents/skills/verify-xiaohe/scripts/xiaohe-verify.sh launch "$RUN_ID" "$PORT"`.
- Run `.agents/skills/verify-xiaohe/scripts/xiaohe-verify.sh doctor "$RUN_ID"` before the first drive.
- Write all proof artifacts to `$EVIDENCE_DIR` (`/tmp/xiaohe-evidence-$RUN_ID/`).
- Never drive an instance not started by this verification run.

## Driving conventions

- Start every recipe from the baseline state unless its preconditions state otherwise.
- Treat every command as literal; preserve quoted headers, flags, and JSON payloads.
- Capture HTTP status codes and bodies together (`curl -sS -w '\nHTTP_STATUS:%{http_code}\n'`).
- Always run `xiaohe-verify.sh cleanup "$RUN_ID"` at the end of a run (and after failed iterations) and verify `$EVIDENCE_DIR` survived.

## Proof and skip reporting

- Capture both the user action and the resulting state/side effect.
- HTTP proof includes route, status code, response body, and server log/DB evidence where applicable.
- CLI proof includes the command, stdout/stderr, and exit code.
- Report any unreachable feature (e.g., real LAN ESPHome or physical Xiaomi hardware) with the exact unmet precondition; never report an unverified path as passed.

## Feature entry contract

Each feature file has an H1 title, a one-paragraph user-POV description, and exactly four H2 sections in order:
1. `Sub-features`
2. `How to get to it (user POV)`
3. `Driving it with <harness>` (starting with `Preconditions:`)
4. `Gotchas`

## Features

- [Health & Lifecycle](./health-and-lifecycle.md) covers `/health`, `/health/ready` (`storage: "ok"`, `runtime: "ok"`), SQLite WAL/schema initialization, and `SIGTERM` graceful shutdown.
- [HTTP Webhooks](./webhooks.md) covers `POST /api/webhooks/{webhook_id}` with Bearer and `X-Webhook-Secret`, rejection codes (`401`/`404`/`400`/`413`), and header sanitization.
- [CLI & Configuration](./cli-and-config.md) covers `xiaohe --help`, `xiaohe supervisor` UDS startup/shutdown, unprefixed env overrides, and nonzero exit on malformed `system.toml`.
- [Skill Runtime & UDS Bridge](./skill-runtime.md) covers `xiaohe supervisor` manifest scanning, `uv` environment isolation, auto-started `miio-airpurifier` (`demo_mode`), SQLite telemetry persistence from Python Skills, and `climate-optimizer` action invocation.
- [Declarative TOML Automations](./automations.md) covers startup loading of `$CONFIG_DIR/automations/*.toml`, webhook-triggered `entity.call` + `skill.invoke` execution, and SQLite `automation_runs` audit logging.
