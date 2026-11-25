# Skill Runtime & UDS Bridge

Skill Runtime & UDS Bridge lets operators run isolated Python Skills (`device` and `action` modes) supervised by `xiaohe supervisor` over a Unix Domain Socket (`RUNTIME_SOCKET`) connected to `xiaohe core`, persisting Skill telemetry into SQLite WAL history.

## Sub-features

- `supervisor-discovery` scans `SKILLS_DIR/*/skill.toml`, validates manifests, and reports discovered Skills (`miio-airpurifier`, `climate-optimizer`, and `open-meteo-weather`).
- `device-skill-autostart` provisions `miio-airpurifier`'s `.venv` via `uv sync`, starts it in `demo_mode = true`, and streams `entity.register` + `entity.state_changed` into `xiaohe core`.
- `skill-telemetry-persistence` persists `sensor.purifier_pm25`, `sensor.purifier_temp`, and `sensor.purifier_humidity` samples emitted by the Python Skill into `numeric_telemetry` in SQLite.
- `python-sdk-unit-suite` verifies `xiaohe_sdk.SkillApp` (device & action lifecycles, heartbeats, `core.get_entity` / `core.call_entity` callbacks) and `open-meteo-weather` directly over OS pipes.

## How to get to it (user POV)

- Launch `xiaohe supervisor` (`SKILLS_DIR=./skills`) and `xiaohe core` (`devices.toml` enabling `miio-airpurifier` with `demo_mode = true`) via `xiaohe-verify.sh launch`.
- Inspect `GET /health/ready` (`"runtime":"ok"`), `supervisor.log`, and `numeric_telemetry` in SQLite.
- Run `python3 -m unittest discover -s sdk/python/tests -v`.

## Driving it with xiaohe-verify.sh and python3

Preconditions:

- `xiaohe supervisor` and `xiaohe core` launched via `xiaohe-verify.sh launch "$RUN_ID" "$PORT"` and `doctor` passed.

- **Confirm supervisor discovery & auto-start.** Run `grep -E 'xiaohe supervisor initialized|device skill running|auto-starting configured device skill' "${SCRATCH_DIR}/supervisor.log" "${SCRATCH_DIR}/core.log" | tee "${EVIDENCE_DIR}/skill-startup.txt"`. Output shows `discovered_skills=3`, `auto-starting configured device skill`, and `device skill running` for `miio-airpurifier`.
- **Verify Skill telemetry persisted to SQLite.** Wait briefly for the 1s poll + history flush (`sleep 3`), then run `python3 -c 'import sqlite3, sys; c=sqlite3.connect(sys.argv[1]); rows=c.execute("SELECT entity_id, COUNT(*), ROUND(AVG(value),2) FROM numeric_telemetry GROUP BY entity_id ORDER BY entity_id").fetchall(); print(rows); assert any(r[0]=="sensor.purifier_pm25" and r[1]>=1 for r in rows)' "${SCRATCH_DIR}/data/xiaohe.db" | tee "${EVIDENCE_DIR}/skill-telemetry.txt"`. Output contains non-zero counts for `sensor.purifier_humidity`, `sensor.purifier_pm25`, and `sensor.purifier_temp`.
- **Verify `xiaohe-sdk` unit tests.** Run `python3 -m unittest discover -s sdk/python/tests -v 2>&1 | tee "${EVIDENCE_DIR}/python-sdk-tests.txt"`. Output ends with `OK`.

## Gotchas

- `HistoryWriter` flushes batches every 500 ms (or immediately on `SIGTERM` shutdown or when batch size reaches 64). Wait ~2 seconds after launch for the 1 s poll plus flush before querying `numeric_telemetry`.
- Physical Xiaomi Air Purifier hardware (`demo_mode = false`) requires a LAN device IP, 32-char `miio_token`, and `uv sync --extra miio`; report that path as skipped in hardware-free environments.
