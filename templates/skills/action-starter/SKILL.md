# Starter Action Skill

Copy this folder into `skills/<your-skill-id>/`, update `[skill]` and `[actions.*]` in `skill.toml`, and implement your routine in `src/main.py`.

## Checklist
1. Rename `[skill].id` in `skill.toml` to a unique `[a-z0-9-]+` identifier.
2. Declare all action parameters under `[actions.<action_name>.params.<param>]`.
3. Use `await app.get_entity(...)` and `await app.call_entity(...)` to interact with Xiaohe Core.
4. Validate with `xiaohe validate --config-dir ./config --skills-dir ./skills`.
