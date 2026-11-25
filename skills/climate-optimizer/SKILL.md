---
name: climate-optimizer
description: On-demand Action Skill that computes indoor heat index from Core sensor entities and optionally triggers a cooling or purifier switch.
---

# Climate Comfort Optimizer (`climate-optimizer`)

Reference **on-demand Action Skill** (`mode = "action"`) demonstrating how automations or operators invoke isolated Python logic that calls back into `xiaohe core` via `core.get_entity` and `core.call_entity`.

## Action: `evaluate_comfort`

### Parameters

| Param | Type | Required | Default | Description |
| :--- | :--- | :--- | :--- | :--- |
| `temp_entity` | `string` | yes | — | Temperature sensor entity ID (in `°C`) |
| `humidity_entity` | `string` | yes | — | Relative humidity sensor entity ID (in `%`) |
| `target_switch` | `string` | no | `null` | Optional switch entity ID to `turn_on` when `heat_index_c >= threshold_c` |
| `threshold_c` | `number` | no | `27.0` | Apparent temperature threshold (`°C`) |

### Example Result

```json
{
  "temperature_c": 28.0,
  "humidity_pct": 70.0,
  "heat_index_c": 31.4,
  "comfort": "hot",
  "recommended_action": "cool",
  "triggered_switch": "switch.purifier_power"
}
```

## Authoring Guide (AI & Human)

To create a new Action Skill:
1. Copy `skills/climate-optimizer/` to `skills/<your-action-skill>/` and set `mode = "action"` in `skill.toml`.
2. Declare at least one `[actions.<action_name>]` table in `skill.toml` and bind it with `@app.on_invoke("<action_name>")`.
3. Use `await app.get_entity(...)` and `await app.call_entity(...)` to read and act on live household state without embedding scripting engines into `xiaohe core`.
4. Return a JSON-serializable `dict` directly from your `@app.on_invoke` handler; `xiaohe supervisor` will automatically shut down the process after the response is delivered.
