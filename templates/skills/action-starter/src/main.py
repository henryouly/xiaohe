"""Minimal Xiaohe Action Skill starter template using xiaohe_sdk.SkillApp."""

from __future__ import annotations

import sys
from pathlib import Path
from typing import Any

_SDK_SRC = Path(__file__).resolve().parents[4] / "sdk" / "python" / "src"
if _SDK_SRC.is_dir() and str(_SDK_SRC) not in sys.path:
    sys.path.insert(0, str(_SDK_SRC))

from xiaohe_sdk import SkillApp  # noqa: E402

app = SkillApp()


@app.on_invoke
async def handle_invoke(
    action: str, params: dict[str, Any], _context: dict[str, Any]
) -> dict[str, Any]:
    if action != "ping_entity":
        raise ValueError(f"unsupported action: {action}")
    entity_id = str(params.get("entity_id") or "").strip()
    if not entity_id:
        raise ValueError("entity_id parameter is required")

    entity = await app.get_entity(entity_id)
    return {
        "entity_id": entity_id,
        "availability": entity.get("availability"),
        "state": entity.get("state"),
    }


if __name__ == "__main__":
    app.run()
