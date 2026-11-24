# `xiaohe-sdk` (Python)

Zero-dependency async Python SDK for building Xiaohe smart home **Device Skills** and **Action Skills** over the NDJSON JSON-RPC 2.0 stdio protocol defined in `docs/skill_spec.md`.

## Quick Start

```python
from xiaohe_sdk import SkillApp

app = SkillApp()

@app.on_init
async def init(config: dict, secrets: dict):
    await app.register_entities(
        device={"device_id": "demo", "name": "Demo Device"},
        entities=[{"entity_id": "switch.demo", "name": "Power", "domain": "switch"}],
    )

@app.on_command("switch.demo")
async def on_power(action: str, params: dict):
    on = action == "turn_on"
    await app.emit_state("switch.demo", {"domain": "switch", "on": on})

if __name__ == "__main__":
    app.run()
```

## Key Conventions

- **`stdout` is reserved for JSON-RPC 2.0 frames**: always use `app.log(...)` (or `sys.stderr`) for diagnostics.
- **Heartbeat & shutdown are automatic**: `skill.ping` replies `{"ok": true}`, and `skill.shutdown` cancels background poll loops and runs `@app.on_shutdown` hooks.
- **Bidirectional callbacks**: Action and Device Skills can inspect or control household state via `await app.get_entity(entity_id)` and `await app.call_entity(entity_id, action, params)`.
