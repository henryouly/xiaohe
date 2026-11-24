# Declarative TOML Automations

Declarative TOML Automations let operators author expression-free automation rules under `$CONFIG_DIR/automations/*.toml` that react to webhooks, entity state/numeric transitions, schedules, and solar events, delegate complex logic to Action Skills via `skill.invoke`, and persist every execution to SQLite `automation_runs`.

## Sub-features

- `automation-startup-load` scans `$CONFIG_DIR/automations/*.toml`, validates schemas statically, and logs `loaded declarative automations` (`automations=1` in the verification harness).
- `webhook-to-skill-execution` fires `verify_webhook_purifier` when `POST /api/webhooks/verify_hook` is accepted, executing both `entity.call` (`switch.purifier_power` → `turn_on`) and `skill.invoke` (`climate-optimizer.evaluate_comfort`).
- `automation-audit-trail` persists a UUIDv7 `AutomationRunRecord` with `outcome = 'succeeded'`, `conditions_passed = 1`, and per-step timing JSON into SQLite `automation_runs`.

## How to get to it (user POV)

- Place a `.toml` rule in `$CONFIG_DIR/automations/` (seeded automatically by `xiaohe-verify.sh launch` as `webhook_purifier.toml`).
- Trigger `POST /api/webhooks/verify_hook` with a valid secret after `miio-airpurifier` entities are online.
- Inspect `core.log` and query `automation_runs` in `$SCRATCH_DIR/data/xiaohe.db`.

## Driving it with curl and python3

Preconditions:

- `xiaohe supervisor` and `xiaohe core` launched via `xiaohe-verify.sh launch "$RUN_ID" "$PORT"`, `doctor` passed, and ~2 seconds elapsed so `miio-airpurifier` has registered `switch.purifier_power`, `sensor.purifier_temp`, and `sensor.purifier_humidity`.

- **Confirm startup load.** Run `grep -E 'loaded declarative automations|automations=1' "${SCRATCH_DIR}/core.log" | tee "${EVIDENCE_DIR}/automation-load.txt"`. Output shows `loaded declarative automations` and `automations=1`.
- **Trigger automation via webhook.** Run `curl -sS -w '\nHTTP_STATUS:%{http_code}\n' -X POST -H 'Authorization: Bearer verify-secret' -H 'Content-Type: application/json' -d '{"source":"verify-automation"}' "http://127.0.0.1:${PORT}/api/webhooks/verify_hook" | tee "${EVIDENCE_DIR}/automation-webhook.txt"`. Returns `HTTP_STATUS:202`.
- **Verify SQLite `automation_runs` audit record.** Wait briefly (`sleep 2`) for `climate-optimizer` `uv sync` + execution, then run `python3 -c 'import json, sqlite3, sys; c=sqlite3.connect(sys.argv[1]); rows=c.execute("SELECT automation_id, trigger_source, conditions_passed, outcome, steps_json FROM automation_runs ORDER BY started_at DESC").fetchall(); print(rows); assert any(r[0]=="verify_webhook_purifier" and r[2]==1 and r[3]=="succeeded" and len(json.loads(r[4]))==2 for r in rows)' "${SCRATCH_DIR}/data/xiaohe.db" | tee "${EVIDENCE_DIR}/automation-runs.txt"`. Output shows `verify_webhook_purifier`, `trigger_source='webhook:verify_hook'`, `outcome='succeeded'`, and 2 succeeded steps (`entity.call` + `skill.invoke`).

## Gotchas

- Triggering `verify_hook` immediately after `launch` before `miio-airpurifier` finishes `entity.register` will cause `entity.call switch.purifier_power` to fail with `entity 'switch.purifier_power' not found`; wait ~2 seconds (or verify `device skill running` in `supervisor.log`) first.
- `verify_webhook_purifier` uses `mode = "single"`; rapid duplicate webhook calls while `climate-optimizer` is still running will legitimately record `skipped_single` audit entries.
