# Health & Lifecycle

Health & Lifecycle lets operators verify that `xiaohe core` is alive, its SQLite WAL history store is initialized and reachable, and the server shuts down cleanly on `SIGTERM`.

## Sub-features

- `health-liveness` returns `200 {"status":"ok"}` from `GET /health`.
- `health-readiness` returns `200 {"status":"ok","storage":"ok","runtime":"disabled"}` from `GET /health/ready`.
- `sqlite-wal-schema` initializes `$SCRATCH_DIR/data/xiaohe.db` in WAL mode with `numeric_telemetry`, `entity_state_history`, and `automation_runs` tables.
- `graceful-sigterm` shuts down cleanly with exit code `0` and logs `xiaohe core shut down cleanly` on `SIGTERM`.

## How to get to it (user POV)

- Request `GET http://127.0.0.1:$PORT/health` and `GET http://127.0.0.1:$PORT/health/ready`.
- Inspect the SQLite database file at `$SCRATCH_DIR/data/xiaohe.db`.
- Send `SIGTERM` to the running `xiaohe core` process.

## Driving it with curl and python3

Preconditions:

- `xiaohe core` launched via `xiaohe-verify.sh launch "$RUN_ID" "$PORT"` and `doctor` passed.

- **Liveness probe.** Run `curl -sS -w '\nHTTP_STATUS:%{http_code}\n' "http://127.0.0.1:${PORT}/health" | tee "${EVIDENCE_DIR}/health.txt"`. Body is `{"status":"ok"}` and `HTTP_STATUS:200`.
- **Readiness probe.** Run `curl -sS -w '\nHTTP_STATUS:%{http_code}\n' "http://127.0.0.1:${PORT}/health/ready" | tee "${EVIDENCE_DIR}/health-ready.txt"`. Body is `{"status":"ok","storage":"ok","runtime":"disabled"}` and `HTTP_STATUS:200`.
- **SQLite WAL & schema side effect.** Run `python3 -c 'import sqlite3, sys; c=sqlite3.connect(sys.argv[1]); print("mode:", c.execute("PRAGMA journal_mode").fetchone()[0]); print("tables:", sorted(r[0] for r in c.execute("SELECT name FROM sqlite_master WHERE type=\"table\"")))' "${SCRATCH_DIR}/data/xiaohe.db" | tee "${EVIDENCE_DIR}/sqlite-schema.txt"`. Output shows `mode: wal` and includes `automation_runs`, `entity_state_history`, and `numeric_telemetry`.
- **Graceful shutdown.** Copy `${SCRATCH_DIR}/core.log` after cleanup or run `kill -TERM "$(cat "${SCRATCH_DIR}/core.pid")" && sleep 0.3 && cp "${SCRATCH_DIR}/core.log" "${EVIDENCE_DIR}/core-shutdown.log"`. Log contains `received SIGTERM, initiating graceful shutdown` and `xiaohe core shut down cleanly`.

## Gotchas

- Sending `SIGTERM` stops the shared verification instance; run this step last (or relaunch before further HTTP drives).
- `runtime` reports `"disabled"` until Phase 3 wires `xiaohe supervisor` over the Unix Domain Socket.
