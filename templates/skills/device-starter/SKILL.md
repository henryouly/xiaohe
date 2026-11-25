---
name: device-starter
description: Minimal Xiaohe Device Skill starter template with demo_mode support.
---

# Starter Device Skill

Copy this folder into `skills/<your-skill-id>/`, update `[skill]` in `skill.toml`, and add third-party device libraries to `pyproject.toml`.

## Checklist
1. Rename `[skill].id` in `skill.toml` to a unique `[a-z0-9-]+` identifier.
2. Add your Python dependencies (e.g., `python-miio`, `bleak`, `httpx`) to `pyproject.toml`.
3. Keep `demo_mode = true` support so the Skill can be verified offline without physical hardware.
4. Validate with `xiaohe validate --config-dir ./config --skills-dir ./skills`.
