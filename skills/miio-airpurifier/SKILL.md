---
name: miio-airpurifier
description: Long-running Device Skill for Xiaomi Air Purifier via python-miio (with demo_mode for hardware-free testing).
---

# Xiaomi Air Purifier (`miio-airpurifier`)

Reference **long-running Device Skill** (`mode = "device"`) demonstrating how to wrap a LAN IoT library (`python-miio`) inside an isolated `uv` Python process using `xiaohe-sdk`.

## Entities Exposed

| `entity_id` | Domain | Capabilities / Unit | Description |
| :--- | :--- | :--- | :--- |
| `switch.purifier_power` | `switch` | `turn_on`, `turn_off` | Main power toggle |
| `sensor.purifier_pm25` | `sensor` | `µg/m³` (`device_class = "pm25"`) | Real-time indoor PM2.5 reading |
| `sensor.purifier_temp` | `sensor` | `°C` (`device_class = "temperature"`) | On-device temperature sensor |
| `sensor.purifier_humidity` | `sensor` | `%` (`device_class = "humidity"`) | On-device relative humidity sensor |

## Configuration (`config/devices.toml` & `config/secrets.toml`)

```toml
# config/devices.toml
[skills.miio-airpurifier]
enabled = true

[skills.miio-airpurifier.config]
host = "192.168.1.55"
poll_interval_secs = 10
area_id = "living_room"
demo_mode = false
```

```toml
# config/secrets.toml (0600 permissions)
miio_token = "0123456789abcdef0123456789abcdef"
```

Install the optional hardware driver inside the Skill's isolated `.venv`:

```bash
cd skills/miio-airpurifier && uv sync --extra miio
```

## Hardware-Free Verification (`demo_mode = true`)

Set `demo_mode = true` in `[skills.miio-airpurifier.config]` to run a deterministic in-memory virtual purifier. Turning `switch.purifier_power` on/off immediately updates `sensor.purifier_pm25` and emits real `entity.state_changed` events over the JSON-RPC 2.0 stdio bridge.

## Authoring Guide (AI & Human)

To adapt this template for another LAN device:
1. Copy `skills/miio-airpurifier/` to `skills/<your-skill-id>/` and update `skill.toml` (`[skill]`, `[config.*]`, `[secrets.*]`).
2. Never read host environment variables or other Skills' secrets — use only `config` and `secrets` passed to `@app.on_init`.
3. Never print to `stdout` — use `app.log(...)` so `stderr` is captured by `xiaohe supervisor`'s 500-line ring buffer.
4. Keep blocking hardware SDK calls wrapped in `await asyncio.to_thread(...)` so `skill.ping` heartbeats are never delayed.
