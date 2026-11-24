"""Minimal Xiaohe Device Skill starter template using xiaohe_sdk.SkillApp."""

from __future__ import annotations

import asyncio
import sys
from pathlib import Path
from typing import Any

for _parent in Path(__file__).resolve().parents:
    _candidate = _parent / "sdk" / "python" / "src"
    if _candidate.is_dir():
        if str(_candidate) not in sys.path:
            sys.path.insert(0, str(_candidate))
        break

from xiaohe_sdk import SkillApp  # noqa: E402

app = SkillApp()
_power_on = False
_poll_task: asyncio.Task[None] | None = None


async def _poll_loop(interval: float) -> None:
    while True:
        await app.emit_state(
            "switch.starter_power",
            {"domain": "switch", "on": _power_on},
        )
        await asyncio.sleep(interval)


@app.on_init
async def handle_init(config: dict[str, Any], _secrets: dict[str, str]) -> None:
    global _poll_task
    area_id = str(config.get("area_id") or "living_room")
    interval = max(1.0, float(config.get("poll_interval_secs") or 15))

    await app.register_entities(
        device={
            "id": "starter_device_1",
            "name": "Starter Device",
            "area_id": area_id,
        },
        entities=[
            {
                "entity_id": "switch.starter_power",
                "name": "Starter Power Switch",
                "area_id": area_id,
                "state": {"domain": "switch", "on": _power_on},
                "capabilities": ["on_off"],
            }
        ],
    )
    _poll_task = asyncio.create_task(_poll_loop(interval))


@app.on_command()
async def handle_command(
    entity_id: str, action: str, _params: dict[str, Any]
) -> dict[str, Any]:
    global _power_on
    if entity_id != "switch.starter_power":
        raise ValueError(f"unknown entity_id: {entity_id}")
    if action == "turn_on":
        _power_on = True
    elif action == "turn_off":
        _power_on = False
    else:
        raise ValueError(f"unsupported action: {action}")

    await app.emit_state(
        "switch.starter_power",
        {"domain": "switch", "on": _power_on},
    )
    return {"ok": True, "on": _power_on}


@app.on_shutdown
async def handle_shutdown() -> None:
    if _poll_task is not None:
        _poll_task.cancel()


if __name__ == "__main__":
    app.run()
