# Skill Specification

A **Skill** is Xiaohe's sole extension unit for third-party device protocols, Python library adapters, notifications, and complex control logic. Every Skill is designed to be **AI- and human-friendly**: self-contained, explicitly declared, isolated in its own runtime environment, and communicating through a minimal JSON-RPC 2.0 wire protocol.

---

## 1. Directory Layout

Skills live under `/skills/<skill_id>/` (mounted into `xiaohe-runtime`):

```text
/skills/miio-airpurifier/
├── skill.toml          # Machine-readable manifest (schema, runtime, config, secrets)
├── SKILL.md            # Human & AI guide: device protocol notes, setup, troubleshooting
├── pyproject.toml      # Python dependencies managed by uv (for Python Skills)
├── uv.lock             # Optional lockfile for deterministic builds
└── src/
    └── main.py         # Entrypoint speaking JSON-RPC 2.0 over stdin/stdout
```

### Why Both `skill.toml` and `SKILL.md`?
- **`skill.toml`** is strictly parsed by `xiaohe-supervisor` and `xiaohe-core` to provision dependencies, generate the Skill Manager configuration form, inject scoped secrets, and route commands.
- **`SKILL.md`** documents hardware quirks, pairing steps (e.g., how to obtain a MiHome token), third-party library caveats, and expected entity schemas so both humans and AI coding agents can maintain or extend the Skill without reverse-engineering source code.

---

## 2. Lifecycle Modes

Every Skill declares one `mode` in `skill.toml`:

| Mode | Purpose | Process Lifecycle | Typical Examples |
|------|---------|-------------------|------------------|
| **`device`** (Long-running) | Device/protocol bridge that maintains connections, polls/subscribes to hardware, publishes entities, and executes commands. | Spawned on startup (or when enabled); supervised continuously with heartbeat checks and exponential-backoff restart on crash. | `python-miio` air purifier, `tinytuya` LAN plugs, Philips Hue bridge, Midea AC, Tesla local bridge. |
| **`action`** (On-demand) | Stateless or short-lived service / complex control routine invoked by TOML automations or REST API. | Spawned on invocation (or kept warm for an idle TTL, e.g., 60s), executes `skill.invoke`, returns JSON result, and exits cleanly. | Telegram/Pushover notifier, PID climate optimizer, dynamic electricity-rate scheduler, LLM summary generator. |

---

## 3. Manifest Schema (`skill.toml`)

```toml
[skill]
id = "miio-airpurifier"
name = "Xiaomi Air Purifier (miio)"
version = "0.1.0"
mode = "device" # "device" | "action"
description = "Controls Xiaomi/Miio air purifiers over LAN using python-miio."

[runtime]
type = "python-uv"                  # "python-uv" | "executable"
python = ">=3.11"
entrypoint = ["python", "src/main.py"]
heartbeat_interval_secs = 30        # Only for mode = "device"
invoke_timeout_secs = 15            # Max time for entity.command / skill.invoke
restart_policy = "exponential"      # "exponential" | "never"

# Non-sensitive user configuration (rendered in Web UI Skill Manager)
[config.host]
type = "string"
required = true
description = "LAN IP address of the air purifier"

[config.poll_interval_secs]
type = "integer"
default = 10
min = 2
max = 300
description = "Telemetry polling interval in seconds"

[config.area_id]
type = "area_ref"
required = false
description = "Default room/area for discovered entities"

# Sensitive credentials (stored in core's 0600-protected config/secrets.toml or via ${ENV_VAR})
[secrets.miio_token]
required = true
description = "32-character hex MiHome device token"

# Declared actions (required for mode = "action"; optional extra actions for mode = "device")
[actions.set_favorite_level]
description = "Set manual fan level (0-16)"
[actions.set_favorite_level.params.level]
type = "integer"
min = 0
max = 16
required = true
```

---

## 4. Environment Isolation (`uv`)

When `xiaohe-supervisor` discovers or reloads a `python-uv` Skill:

1. Creates/updates an isolated virtual environment at `/skills/<skill_id>/.venv` (using shared download cache `/var/cache/uv`).
2. Runs `uv sync --frozen` (if `uv.lock` exists) or `uv sync` against `pyproject.toml`.
3. Reports installation progress and any dependency errors back to `xiaohe-core` so the Admin UI displays real-time status (`Installing` → `Running` or `Error`).
4. Spawns the entrypoint using `/skills/<skill_id>/.venv/bin/python` with:
   - Restricted environment variables (no core master key, no other Skills' secrets).
   - `stdout` reserved exclusively for JSON-RPC 2.0 NDJSON frames.
   - `stderr` captured by the supervisor into a per-skill ring buffer (last 500 lines) and streamed to the Admin UI.

---

## 5. Wire Protocol (JSON-RPC 2.0 over NDJSON)

Both hops — `xiaohe-core ↔ xiaohe-supervisor` (Unix Domain Socket) and `xiaohe-supervisor ↔ Skill` (`stdin`/`stdout`) — use newline-delimited JSON-RPC 2.0 (`\n`-terminated single-line JSON objects). The supervisor transparently multiplexes `skill_id` envelopes on the UDS hop so individual Skills remain simple single-tenant stdin/stdout processes.

### 5.1 Initialization Handshake (Core/Supervisor → Skill)

Immediately after spawning a Skill process, the supervisor sends `skill.init`:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "skill.init",
  "params": {
    "protocol_version": "1.0",
    "skill_id": "miio-airpurifier",
    "config": { "host": "192.168.1.42", "poll_interval_secs": 10, "area_id": "living_room" },
    "secrets": { "miio_token": "0123456789abcdef0123456789abcdef" }
  }
}
```

Skill responds with `{ "jsonrpc": "2.0", "id": 1, "result": { "ok": true } }`.

### 5.2 Device / Entity Registration (Skill → Core, Notification)

A `device` Skill registers its device(s) and entities after `skill.init`:

```json
{
  "jsonrpc": "2.0",
  "method": "entity.register",
  "params": {
    "device": {
      "device_id": "miio_living_purifier",
      "name": "Living Room Air Purifier",
      "manufacturer": "Xiaomi",
      "model": "zhimi.airpurifier.mb3",
      "area_id": "living_room"
    },
    "entities": [
      {
        "entity_id": "switch.living_purifier_power",
        "name": "Purifier Power",
        "domain": "switch",
        "capabilities": []
      },
      {
        "entity_id": "sensor.living_purifier_pm25",
        "name": "Living Room PM2.5",
        "domain": "sensor",
        "unit": "µg/m³",
        "device_class": "pm25"
      }
    ]
  }
}
```

### 5.3 State Updates (Skill → Core, Notification)

```json
{
  "jsonrpc": "2.0",
  "method": "entity.state_changed",
  "params": {
    "entity_id": "sensor.living_purifier_pm25",
    "availability": "online",
    "state": {
      "domain": "sensor",
      "value": 14.0,
      "unit": "µg/m³",
      "device_class": "pm25"
    },
    "extra": {
      "filter_life_remaining": 78
    }
  }
}
```

### 5.4 Entity Command Execution (Core → Skill, Request)

```json
{
  "jsonrpc": "2.0",
  "id": 42,
  "method": "entity.command",
  "params": {
    "entity_id": "switch.living_purifier_power",
    "action": "turn_on",
    "params": {}
  }
}
```

Response:
```json
{ "jsonrpc": "2.0", "id": 42, "result": { "ok": true } }
```

### 5.5 Action Skill Invocation (Core → Skill, Request)

Used by TOML automations (`action.type = "skill.invoke"`) or REST API:

```json
{
  "jsonrpc": "2.0",
  "id": 99,
  "method": "skill.invoke",
  "params": {
    "action": "adjust_climate_comfort",
    "params": { "area_id": "living_room" },
    "context": {
      "trigger_entity_id": "sensor.living_temp",
      "entities": {
        "sensor.living_temp": { "value": 27.4 },
        "sensor.living_humidity": { "value": 68.0 }
      }
    }
  }
}
```

An Action Skill may also call back to Core during execution via `core.call_entity` (to issue commands to other devices) or `core.get_entity` (to inspect current household state), enabling arbitrarily complex multi-device control logic without putting a scripting engine inside Rust.

### 5.6 Heartbeat & Graceful Shutdown

- **Heartbeat**: Supervisor sends `skill.ping` every `heartbeat_interval_secs`; Skill replies `{ "ok": true }`. Three consecutive missed pings trigger a restart.
- **Shutdown**: Supervisor sends `skill.shutdown` notification and waits up to 5 seconds before sending `SIGTERM` / `SIGKILL`.

---

## 6. Minimal Python Device Skill Example

```python
# src/main.py — Minimal adapter wrapping python-miio
import asyncio
from xiaohe_sdk import SkillApp
from miio import AirPurifierMiot

app = SkillApp()
device: AirPurifierMiot | None = None

@app.on_init
async def init(config: dict, secrets: dict):
    global device
    device = AirPurifierMiot(ip=config["host"], token=secrets["miio_token"])
    await app.register_entities(
        device={"device_id": "miio_purifier", "name": "Air Purifier", "area_id": config.get("area_id")},
        entities=[
            {"entity_id": "switch.purifier_power", "name": "Power", "domain": "switch"},
            {"entity_id": "sensor.purifier_pm25", "name": "PM2.5", "domain": "sensor", "unit": "µg/m³"},
        ],
    )
    app.spawn_poll_loop(poll_status, interval=config["poll_interval_secs"])

async def poll_status():
    status = await asyncio.to_thread(device.status)
    await app.emit_state("switch.purifier_power", {"domain": "switch", "on": status.is_on})
    await app.emit_state("sensor.purifier_pm25", {"domain": "sensor", "value": float(status.aqi), "unit": "µg/m³"})

@app.on_command("switch.purifier_power")
async def handle_power(action: str, params: dict):
    if action == "turn_on":
        await asyncio.to_thread(device.on)
    elif action == "turn_off":
        await asyncio.to_thread(device.off)
    await poll_status()

if __name__ == "__main__":
    app.run()
```
