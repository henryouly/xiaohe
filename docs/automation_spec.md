# Declarative TOML Automation Specification

Xiaohe's automation engine runs natively inside `xiaohe-core` in Rust. It is **strictly declarative and expression-free**: rules are authored in TOML files with zero embedded template or scripting languages (no Jinja2, Rhai, or Lua). Whenever an automation needs dynamic calculations, multi-step branching, external API lookups, or stateful control algorithms, it delegates cleanly to an **Action Skill** via `type = "skill.invoke"`.

---

## 1. File Organization & Reloading

- Each automation (or logical group) is stored as a `.toml` file in `/config/automations/` (e.g., `/config/automations/living_room_motion.toml`).
- `xiaohe-core` parses and **statically validates** every file on startup and whenever modified via the Web UI or `POST /api/automations/reload`.
- Static validation verifies:
  - Unique `id` across all files
  - Valid trigger, condition, and action schemas
  - Valid `HH:MM:SS` / Cron / duration (`"5m"`, `"30s"`) syntax
  - Optional warning (non-blocking) if a referenced `entity_id` or `skill_id` is currently offline/unregistered

---

## 2. Top-Level Schema

```toml
id = "living_room_motion_lights"
name = "Living Room Motion Lights"
description = "Turn on living room ceiling light on motion after sunset; turn off after 5 minutes idle."
enabled = true
mode = "restart" # "single" | "restart" | "queued" | "parallel"
```

### Execution Modes (`mode`)

| Mode | Behavior when triggered while a previous run is still active |
|------|--------------------------------------------------------------|
| **`single`** (default) | Ignore the new trigger and record a `skipped_single` audit entry. |
| **`restart`** | Cancel the in-flight run (e.g., an active `delay` step) and start fresh. |
| **`queued`** | Queue the new run behind the current one (bounded queue depth, default 10). |
| **`parallel`** | Run concurrently up to `max_parallel` (default 5). |

---

## 3. Triggers (`[[triggers]]`)

Multiple `[[triggers]]` entries are evaluated with **OR** semantics (any matching trigger fires the rule).

### 3.1 `state` — Exact State Transition
Fires when an entity transitions to/from a discrete state, optionally requiring it to hold for `for_secs`.

```toml
[[triggers]]
type = "state"
entity_id = "binary_sensor.living_motion"
from = "off"       # Optional
to = "on"          # Optional (omit both from/to to trigger on any change)
for_secs = 0       # Optional debounce duration in seconds
```

### 3.2 `numeric_state` — Threshold Crossing
Fires when a numeric sensor (or numeric entity field like `climate.current_temp_c`) **crosses** a threshold (edge-triggered, not level-triggered, to avoid repeated firing).

```toml
[[triggers]]
type = "numeric_state"
entity_id = "sensor.living_purifier_pm25"
above = 50.0       # Optional: fires when crossing from <= 50.0 to > 50.0
below = 10.0       # Optional: fires when crossing from >= 10.0 to < 10.0
for_secs = 120     # Must stay above/below for 2 minutes
```

### 3.3 `time` — Fixed Time or Cron
```toml
[[triggers]]
type = "time"
at = "07:30:00"
weekdays = ["mon", "tue", "wed", "thu", "fri"] # Optional

# Or cron syntax:
# type = "time"
# cron = "0 */15 * * * *"
```

### 3.4 `sun` — Sunrise / Sunset
Uses the household latitude/longitude configured in `config/system.toml`.

```toml
[[triggers]]
type = "sun"
event = "sunset"       # "sunrise" | "sunset"
offset_minutes = -15   # 15 minutes before sunset
```

### 3.5 `webhook` — Incoming HTTP Webhook
```toml
[[triggers]]
type = "webhook"
webhook_id = "ios_arriving_home"
```

---

## 4. Conditions (`[[conditions]]`)

Top-level `[[conditions]]` entries are evaluated with **AND** semantics. All conditions must pass after a trigger fires for actions to execute.

### 4.1 `state`
```toml
[[conditions]]
type = "state"
entity_id = "switch.vacation_mode"
equals = "off"
```

### 4.2 `numeric_state`
```toml
[[conditions]]
type = "numeric_state"
entity_id = "sensor.living_lux"
below = 30.0
```

### 4.3 `time`
```toml
[[conditions]]
type = "time"
after = "18:00:00"     # Optional; supports midnight wrap-around (e.g., after 22:00, before 06:00)
before = "06:30:00"    # Optional
weekdays = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"]
```

### 4.4 `sun`
```toml
[[conditions]]
type = "sun"
after = "sunset"
after_offset_minutes = -15
```

### 4.5 Logical Combinators (`and` / `or` / `not`)
For nested boolean logic without an expression parser, use structured combinator tables:

```toml
[[conditions]]
type = "or"
conditions = [
  { type = "state", entity_id = "binary_sensor.alice_home", equals = "on" },
  { type = "state", entity_id = "binary_sensor.bob_home", equals = "on" },
]
```

---

## 5. Actions (`[[actions]]`)

Actions execute sequentially unless wrapped in a `parallel` step.

### 5.1 `entity.call` — Standard or Custom Entity Command
```toml
[[actions]]
type = "entity.call"
entity_id = "light.living_ceiling"
action = "turn_on"
[actions.params]
brightness = 80
color_temp_k = 3000
```

### 5.2 `skill.invoke` — Delegate Complex Logic to an Action Skill
Passes static `params` plus an optional list of `context_entities` whose current states `xiaohe-core` snapshots and forwards to the Skill.

```toml
[[actions]]
type = "skill.invoke"
skill_id = "climate-comfort-optimizer"
action = "rebalance_room"
timeout_secs = 15
context_entities = [
  "sensor.living_temp",
  "sensor.living_humidity",
  "climate.living_ac",
]
[actions.params]
area_id = "living_room"
target_comfort = "eco"
```

### 5.3 `delay` — Timed Wait (Cancellable in `restart` Mode)
```toml
[[actions]]
type = "delay"
secs = 300
```

### 5.4 `parallel` — Concurrent Sub-Actions
```toml
[[actions]]
type = "parallel"
steps = [
  { type = "entity.call", entity_id = "light.living_ceiling", action = "turn_off" },
  { type = "entity.call", entity_id = "switch.living_tv_plug", action = "turn_off" },
]
```

---

## 6. Complete Example: Delegation Pattern

Instead of writing fragile inline Jinja templates to compute heat index and pick an AC fan speed, the TOML rule stays trivial and delegates the math to a Python Skill:

```toml
id = "smart_living_climate"
name = "Smart Living Room Climate"
enabled = true
mode = "single"

[[triggers]]
type = "numeric_state"
entity_id = "sensor.living_temp"
above = 26.0
for_secs = 180

[[conditions]]
type = "state"
entity_id = "binary_sensor.living_window"
equals = "off"

[[actions]]
type = "skill.invoke"
skill_id = "climate-comfort-optimizer"
action = "apply_cooling_curve"
context_entities = ["sensor.living_temp", "sensor.living_humidity", "climate.living_ac"]
[actions.params]
area_id = "living_room"
```

---

## 7. Audit Trail

Every automation execution writes an `AutomationRunRecord` to `HistoryStore` containing:
- `run_id` (UUIDv7), `automation_id`, `started_at`, `finished_at`
- `trigger_summary` (which trigger fired and what value caused it)
- `outcome` (`succeeded` | `conditions_not_met` | `failed` | `cancelled` | `skipped_single`)
- `steps` (per-action status, duration in ms, and error message if failed)
