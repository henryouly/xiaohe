"""Xiaohe Reference Action Skill: Indoor Thermal Comfort Optimizer."""

from __future__ import annotations

import sys
from pathlib import Path
from typing import Any

# Allow running directly from the repo without pre-installing xiaohe-sdk into .venv.
_SDK_SRC = Path(__file__).resolve().parents[3] / "sdk" / "python" / "src"
if _SDK_SRC.is_dir() and str(_SDK_SRC) not in sys.path:
    sys.path.insert(0, str(_SDK_SRC))

from xiaohe_sdk import SkillApp  # noqa: E402

app = SkillApp()


def compute_heat_index_c(temp_c: float, rh: float) -> float:
    """Compute apparent temperature (°C) using Steadman / Rothfusz regression."""
    temp_f = temp_c * 9.0 / 5.0 + 32.0
    if temp_f < 80.0:
        hi_f = 0.5 * (temp_f + 61.0 + (temp_f - 68.0) * 1.2 + rh * 0.094)
    else:
        hi_f = (
            -42.379
            + 2.04901523 * temp_f
            + 10.14333127 * rh
            - 0.22475541 * temp_f * rh
            - 0.00683783 * temp_f * temp_f
            - 0.05481717 * rh * rh
            + 0.00122874 * temp_f * temp_f * rh
            + 0.00085282 * temp_f * rh * rh
            - 0.00000199 * temp_f * temp_f * rh * rh
        )
    return round((hi_f - 32.0) * 5.0 / 9.0, 2)


async def _read_numeric_sensor(entity_id: str, context: dict[str, Any]) -> float:
    """Prefer live core.get_entity callback; fall back to automation context snapshot."""
    ctx_entities = context.get("entities") or {}
    if entity_id in ctx_entities and "value" in ctx_entities[entity_id]:
        return float(ctx_entities[entity_id]["value"])
    ent = await app.get_entity(entity_id)
    state = ent.get("state") or {}
    if "value" not in state:
        raise ValueError(f"entity '{entity_id}' does not have a numeric sensor value")
    return float(state["value"])


@app.on_invoke("evaluate_comfort")
async def evaluate_comfort(params: dict[str, Any], context: dict[str, Any]) -> dict[str, Any]:
    temp_entity = str(params.get("temp_entity") or "").strip()
    humidity_entity = str(params.get("humidity_entity") or "").strip()
    if not temp_entity or not humidity_entity:
        raise ValueError("temp_entity and humidity_entity are required")

    threshold_c = float(params.get("threshold_c", 27.0))
    target_switch = params.get("target_switch")

    temp_c = await _read_numeric_sensor(temp_entity, context)
    rh = await _read_numeric_sensor(humidity_entity, context)
    heat_index_c = compute_heat_index_c(temp_c, rh)

    if heat_index_c >= threshold_c + 2.0:
        comfort = "hot"
        recommended_action = "cool"
    elif heat_index_c >= threshold_c:
        comfort = "warm"
        recommended_action = "ventilate"
    elif rh < 30.0:
        comfort = "dry"
        recommended_action = "humidify"
    else:
        comfort = "comfortable"
        recommended_action = "none"

    triggered_switch: str | None = None
    if target_switch and heat_index_c >= threshold_c:
        app.log(
            f"heat_index_c={heat_index_c} >= threshold_c={threshold_c}; calling {target_switch}.turn_on"
        )
        await app.call_entity(str(target_switch), "turn_on", {})
        triggered_switch = str(target_switch)

    return {
        "temperature_c": temp_c,
        "humidity_pct": rh,
        "heat_index_c": heat_index_c,
        "comfort": comfort,
        "recommended_action": recommended_action,
        "triggered_switch": triggered_switch,
    }


if __name__ == "__main__":
    app.run()
