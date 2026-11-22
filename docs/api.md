# API Reference

Base URL is the `xiaohe-core` HTTP server (`HOST:PORT`, default `0.0.0.0:4000`). All request and response bodies are JSON unless explicitly noted (TOML raw endpoints and the SSE stream `GET /api/events`). OpenAPI 3.1 interactive documentation is derived via `utoipa` and served at `/docs`.

The TypeScript mirror of these shapes lives in `web/src/lib/api.ts` and must be updated alongside backend schema changes.

---

## 1. Authentication & RBAC

Every route under `/api/*` (except `/api/auth/login`, `/api/auth/bootstrap`, `/api/auth/me`, and `/api/webhooks/*`) requires authentication via either:
- **Session Cookie**: `xiaohe_session` (`HttpOnly`, `SameSite=Lax`) set by `/api/auth/login`, or
- **Bearer Token**: `Authorization: Bearer <token>` (user-scoped API token).

Unauthenticated requests return `401 {"error": "unauthorized"}`. Insufficient role or unauthorized area access returns `403 {"error": "forbidden"}`.

### `POST /api/auth/bootstrap`
First-boot only (returns `409` once an Admin user exists). Creates the initial Admin account.
- Request: `{ "username": string, "password": string, "display_name": string }`
- Response `201`: `{ "user": UserSummary }`

### `POST /api/auth/login`
- Request: `{ "username": string, "password": string }`
- Response `200`: `{ "user": UserSummary }` + sets `Set-Cookie: xiaohe_session=...`
- Errors: `401` invalid credentials.

### `POST /api/auth/logout`
- Response `204` + clears session cookie.

### `GET /api/auth/me`
- Public check used by the SPA auth guard.
- Response `200`: `{ "initialized": boolean, "user": UserSummary | null }`

`UserSummary`: `{ "id", "username", "display_name", "role": "admin" | "member" | "guest", "allowed_areas": string[] | "*" }`

---

## 2. Areas, Devices & Entities

All read endpoints automatically filter results to entities/devices inside the caller's `allowed_areas` (Admins see everything).

### `GET /api/areas`
- Response `200`: `{ "areas": [{ "area_id", "name", "icon", "entity_count" }] }`

### `POST /api/areas` · `PUT /api/areas/{area_id}` · `DELETE /api/areas/{area_id}` *(Admin)*
- Manage room/zone definitions persisted to `config/areas.toml`.

### `GET /api/entities`
- Optional query params: `?area_id=...&domain=...&source=...`
- Response `200`: `{ "entities": EntityRecord[] }`

### `GET /api/entities/{entity_id}`
- Response `200`: `EntityRecord`
- Errors: `404` unknown entity · `403` entity belongs to an unauthorized area.

### `POST /api/entities/{entity_id}/command`
Dispatch a domain or custom command to an ESPHome device or Device Skill.
- Request: `{ "action": string, "params": object }`
  - Example (Light): `{ "action": "turn_on", "params": { "brightness": 75, "color_temp_k": 3200 } }`
  - Example (Climate): `{ "action": "set_temperature", "params": { "target_temp_c": 23.5 } }`
- Response `200`: `{ "ok": true, "entity_id": string }`
- Errors: `400` invalid action/params · `403` area/role forbidden (e.g., Guest attempting `lock.unlock`) · `404` unknown entity · `503` device or Skill offline · `504` Skill command timeout.

---

## 3. History & Telemetry

Served through the pluggable `HistoryStore` DAL (SQLite by default, InfluxDB when configured).

### `GET /api/entities/{entity_id}/history`
- Query params:
  - `from` (RFC3339 or Unix ms, default: now - 24h)
  - `to` (RFC3339 or Unix ms, default: now)
  - `bucket_secs` (optional integer for server-side downsampling, e.g., `300` for 5-minute buckets in `uPlot`)
- Response `200`:
  - Numeric sensor: `{ "kind": "numeric", "entity_id", "unit", "points": [{ "ts": number, "avg": number, "min": number, "max": number }] }`
  - Discrete entity: `{ "kind": "discrete", "entity_id", "transitions": [{ "ts": number, "state": string }] }`

---

## 4. Skill Management *(Admin Only)*

### `GET /api/skills`
- Response `200`: `{ "skills": SkillSummary[] }`
- `SkillSummary`: `{ "skill_id", "name", "version", "mode": "device" | "action", "enabled", "status": "stopped" | "installing" | "running" | "error", "restart_count", "last_error": string | null, "entity_count": number }`

### `GET /api/skills/{skill_id}`
- Response `200`: Full manifest schema (`config` fields, `secrets` keys with `is_set: boolean` — never plaintext), current non-secret config values, and health metadata.

### `PUT /api/skills/{skill_id}/config`
Update non-secret configuration and/or write-only secrets (encrypted via AES-256-GCM before saving to `config/secrets.toml`). Automatically reloads the Skill if running.
- Request: `{ "config": object, "secrets": Record<string, string> }`
- Response `200`: `{ "ok": true, "reloaded": boolean }`

### `POST /api/skills/{skill_id}/start` · `stop` · `restart` · `reinstall`
- Lifecycle controls forwarded to `xiaohe-supervisor` over UDS JSON-RPC (`reinstall` re-runs `uv sync`).
- Response `200`: `{ "skill_id", "status" }`

### `GET /api/skills/{skill_id}/logs`
- Query params: `?tail=200` (default 200, max 500)
- Response `200`: `{ "skill_id", "lines": [{ "ts": number, "stream": "stderr", "line": string }] }`

### `POST /api/skills/{skill_id}/invoke`
Manually invoke an `action` Skill (or custom action on a `device` Skill).
- Request: `{ "action": string, "params": object }`
- Response `200`: `{ "ok": true, "result": any }`

---

## 5. Automations *(Admin Only)*

### `GET /api/automations`
- Response `200`: `{ "automations": [{ "id", "name", "enabled", "mode", "file", "last_run_at", "last_outcome" }] }`

### `GET /api/automations/{id}`
- Response `200`: `{ "id", "file", "parsed": object, "raw_toml": string }`

### `POST /api/automations` · `PUT /api/automations/{id}`
Create or update an automation from raw TOML. Runs static validation before atomic write-back and hot-reloading the rule graph.
- Request: `{ "raw_toml": string }`
- Response `200` / `201`: `{ "id", "warnings": string[] }`
- Errors: `422 {"error": "validation_failed", "details": [{ "line", "message" }] }`

### `POST /api/automations/{id}/toggle`
- Request: `{ "enabled": boolean }`
- Response `200`: `{ "id", "enabled" }`

### `POST /api/automations/{id}/trigger`
Manually run an automation (optionally bypassing conditions via `{ "skip_conditions": true }`).
- Response `200`: `{ "run_id", "outcome" }`

### `DELETE /api/automations/{id}`
- Response `204`

### `GET /api/automations/runs`
- Query params: `?automation_id=...&limit=50&offset=0`
- Response `200`: `{ "runs": AutomationRunRecord[] }`

---

## 6. Users & API Tokens *(Admin Only)*

- `GET /api/users` — List household users (without password hashes).
- `POST /api/users` — Create user with `role` (`admin` | `member` | `guest`) and `allowed_areas`.
- `PUT /api/users/{user_id}` — Update role, `allowed_areas`, or reset password.
- `DELETE /api/users/{user_id}` — Delete user (cannot delete the last remaining Admin).
- `POST /api/users/{user_id}/tokens` — Mint a new API Bearer token (plaintext returned **once**).
- `DELETE /api/users/{user_id}/tokens/{token_id}` — Revoke token.

---

## 7. Webhooks

### `POST /api/webhooks/{webhook_id}`
- Authenticated via `Authorization: Bearer <webhook_secret>` or `X-Webhook-Secret: <webhook_secret>` (independent of user sessions).
- Request: Optional JSON body (max 64 KB).
- Response `202`: `{ "accepted": true, "webhook_id": string }`
- Errors: `401` invalid secret · `404` unknown or disabled webhook.

---

## 8. Live Events (`GET /api/events`, SSE)

Persistent `text/event-stream` connection with 15-second keep-alive comments (`: keep-alive`). Events are filtered server-side by the authenticated user's `Role` and `allowed_areas` before serialization.

| Event Name | Audience | Payload |
|------------|----------|---------|
| `entity_state` | Users allowed in `area_id` | `{ "entity_id", "area_id", "availability", "state", "extra", "last_changed_at", "last_updated_at" }` |
| `skill_status` | Admin only | `{ "skill_id", "status", "restart_count", "last_error", "at" }` |
| `skill_log` | Admin only (when `?logs=1`) | `{ "skill_id", "ts", "line" }` |
| `automation_run` | Admin only | `{ "run_id", "automation_id", "outcome", "started_at", "finished_at" }` |
| `resync` | Any lagged client | `{ "reason": "lagged" }` — client must refetch `GET /api/entities` (and `/api/skills` if Admin) |

### Client Snapshot Merge Rule
Follows `tesla-apiscraper-rs` semantics:
- REST snapshot fetches (`GET /api/entities`) only overwrite a cached entity in the SolidJS store if `entity.last_updated_at` is strictly newer than the locally cached timestamp.
- Live SSE `entity_state` frames apply strictly in arrival order (server-chronological). If the client receives `resync` or reconnects after a drop, it refetches `GET /api/entities`.

---

## 9. Health Probes

- `GET /health` → `200 {"status": "ok"}` (always public; liveness probe).
- `GET /health/ready` → `200 {"status": "ok", "storage": "ok", "runtime": "ok"}` or `503` if `HistoryStore` or `xiaohe-supervisor` UDS bridge is unreachable.
