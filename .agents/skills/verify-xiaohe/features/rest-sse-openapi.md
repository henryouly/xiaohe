# REST API, RBAC-Filtered SSE & OpenAPI 3.1

Xiaohe Core serves the complete authenticated REST API for Areas, Entities/History, Skills *(Admin)*, and Automations *(Admin)*, a live RBAC-filtered Server-Sent Events stream (`GET /api/events`), and public OpenAPI 3.1 documentation (`GET /api/openapi.json` and `GET /docs`).

## Sub-features

- **Areas CRUD & RBAC**: `GET/POST /api/areas`, `PUT/DELETE /api/areas/{area_id}` with atomic `areas.toml` persistence and area-scoped filtering for Members/Guests.
- **Entities, Commands & History**: `GET /api/entities`, `GET /api/entities/{entity_id}`, `POST /api/entities/{entity_id}/command`, `GET /api/entities/{entity_id}/history` enforcing `allowed_areas` and Guest lock denial (`403`).
- **Skills Admin API**: `GET /api/skills`, `GET /api/skills/{skill_id}`, `PUT /api/skills/{skill_id}/config` (write-only `0600` `secrets.toml`, redacted `is_set` response, preserves omitted `config`), `POST /api/skills/{skill_id}/{start,stop,restart,reinstall,invoke}`, and `GET /api/skills/{skill_id}/logs`.
- **Automations Admin API**: `GET/POST /api/automations`, `GET/PUT/DELETE /api/automations/{id}`, `POST /api/automations/{id}/{toggle,trigger}`, `POST /api/automations/reload`, `GET /api/automations/runs` with `422` schema validation and atomic hot-reload.
- **Live SSE (`GET /api/events`)**: requires authentication (`401` unauth), emits 15s `: keep-alive` and `resync` on lag, and re-validates credentials & RBAC on every event before serialization.
- **OpenAPI 3.1**: public `GET /api/openapi.json` (`openapi: "3.1.0"`) and interactive `GET /docs` HTML viewer.

## How to get to it (user POV)

Authenticate as an Admin (via session cookie or Bearer token) for full CRUD/admin endpoints, or as a scoped Member/Guest to verify area filtering, then connect to `GET /api/events` and `GET /api/openapi.json` / `GET /docs`.

## Driving it with `xiaohe-verify.sh` + `curl`

Preconditions: `xiaohe supervisor` + `xiaohe core` launched via `xiaohe-verify.sh launch "$RUN_ID" "$PORT"`, `doctor` passed, and Admin bootstrapped (or logged in with cookie jar `$JAR`) plus a Member Bearer token `$MEMBER_TOK` scoped to `["living_room"]`.

```bash
BASE="http://127.0.0.1:${PORT}"

# 1. Areas CRUD + Member area scoping
curl -sS -b "$JAR" -w '\nHTTP_STATUS:%{http_code}\n' -X POST "$BASE/api/areas" \
  -H 'Content-Type: application/json' -d '{"area_id":"living_room","name":"Living Room"}' \
  | tee "$EVIDENCE_DIR/rest-area-create-1.txt"
curl -sS -b "$JAR" -w '\nHTTP_STATUS:%{http_code}\n' -X POST "$BASE/api/areas" \
  -H 'Content-Type: application/json' -d '{"area_id":"bedroom","name":"Bedroom"}' \
  | tee "$EVIDENCE_DIR/rest-area-create-2.txt"
curl -sS -H "Authorization: Bearer ${MEMBER_TOK}" -w '\nHTTP_STATUS:%{http_code}\n' \
  "$BASE/api/areas" | tee "$EVIDENCE_DIR/rest-area-member-list.txt"

# 2. Skills Admin API + redacted secrets
curl -sS -b "$JAR" -w '\nHTTP_STATUS:%{http_code}\n' "$BASE/api/skills" \
  | tee "$EVIDENCE_DIR/rest-skills-list.txt"
curl -sS -b "$JAR" -w '\nHTTP_STATUS:%{http_code}\n' -X PUT "$BASE/api/skills/miio-airpurifier/config" \
  -H 'Content-Type: application/json' \
  -d '{"secrets":{"miio_token":"verify-secret-value"}}' | tee "$EVIDENCE_DIR/rest-skill-secret-put.txt"
curl -sS -b "$JAR" -w '\nHTTP_STATUS:%{http_code}\n' "$BASE/api/skills/miio-airpurifier" \
  | tee "$EVIDENCE_DIR/rest-skill-detail.txt"

# 3. Entities list, command, and history
curl -sS -b "$JAR" -w '\nHTTP_STATUS:%{http_code}\n' "$BASE/api/entities?source=skill:miio-airpurifier" \
  | tee "$EVIDENCE_DIR/rest-entities-list.txt"
curl -sS -b "$JAR" -w '\nHTTP_STATUS:%{http_code}\n' -X POST "$BASE/api/entities/switch.purifier_power/command" \
  -H 'Content-Type: application/json' -d '{"action":"turn_off"}' \
  | tee "$EVIDENCE_DIR/rest-entity-command.txt"
curl -sS -b "$JAR" -w '\nHTTP_STATUS:%{http_code}\n' "$BASE/api/entities/switch.purifier_power/history" \
  | tee "$EVIDENCE_DIR/rest-entity-history.txt"

# 4. Automations CRUD (422 invalid, 201 create, trigger, runs, delete)
curl -sS -b "$JAR" -w '\nHTTP_STATUS:%{http_code}\n' -X POST "$BASE/api/automations" \
  -H 'Content-Type: application/json' -d '{"raw_toml":"id = \"bad\"\n"}' \
  | tee "$EVIDENCE_DIR/rest-auto-invalid.txt"
curl -sS -b "$JAR" -w '\nHTTP_STATUS:%{http_code}\n' -X POST "$BASE/api/automations" \
  -H 'Content-Type: application/json' \
  -d '{"raw_toml":"id = \"verify_api_rule\"\nname = \"API Rule\"\nenabled = true\n\n[[triggers]]\ntype = \"webhook\"\nwebhook_id = \"verify_hook\"\n\n[[actions]]\ntype = \"entity.call\"\nentity_id = \"switch.purifier_power\"\naction = \"turn_on\"\n"}' \
  | tee "$EVIDENCE_DIR/rest-auto-create.txt"
curl -sS -b "$JAR" -w '\nHTTP_STATUS:%{http_code}\n' -X POST "$BASE/api/automations/verify_api_rule/trigger" \
  -H 'Content-Type: application/json' -d '{"skip_conditions":true}' \
  | tee "$EVIDENCE_DIR/rest-auto-trigger.txt"
sleep 1
curl -sS -b "$JAR" -w '\nHTTP_STATUS:%{http_code}\n' "$BASE/api/automations/runs?automation_id=verify_api_rule" \
  | tee "$EVIDENCE_DIR/rest-auto-runs.txt"
curl -sS -b "$JAR" -w '\nHTTP_STATUS:%{http_code}\n' -X DELETE "$BASE/api/automations/verify_api_rule" \
  | tee "$EVIDENCE_DIR/rest-auto-delete.txt"

# 5. Live SSE: 401 unauth + Admin receives entity_state on live command
curl -sS -w '\nHTTP_STATUS:%{http_code}\n' "$BASE/api/events" | tee "$EVIDENCE_DIR/sse-unauth.txt"
curl -sN -m 4 -b "$JAR" "$BASE/api/events" > "$EVIDENCE_DIR/sse-admin-stream.txt" &
SSE_PID=$!
sleep 0.5
curl -sS -b "$JAR" -X POST "$BASE/api/entities/switch.purifier_power/command" \
  -H 'Content-Type: application/json' -d '{"action":"turn_off"}' > /dev/null
wait "$SSE_PID" || true

# 6. OpenAPI 3.1 + /docs
curl -sS -w '\nHTTP_STATUS:%{http_code}\n' "$BASE/api/openapi.json" | tee "$EVIDENCE_DIR/openapi-json.txt"
curl -sS -w '\nHTTP_STATUS:%{http_code}\n' "$BASE/docs" | tee "$EVIDENCE_DIR/openapi-docs.txt"
```

## Gotchas

- `GET /api/skills/{skill_id}` returns `secrets` as `{ "<key>": { "is_set": bool } }`; plaintext secrets are never returned over HTTP.
- `PUT /api/skills/{skill_id}/config` preserves existing `config` in `devices.toml` when the `config` field is omitted.
- Live Guest lock `403` denial requires a `lock.*` entity registered in the Guest's allowed area (no demo lock skill is bundled; proven in `src/http/mod.rs` integration tests).
- `POST /api/automations` rejects duplicate IDs or clobbering an existing `{id}.toml` file with `409 Conflict`; `PUT /api/automations/{id}` returns `404 Not Found` for nonexistent IDs.
