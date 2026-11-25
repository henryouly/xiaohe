# Xiaohe Skill Authoring Guide (AI Prompt Kit)

Use this guide — or paste the prompt below directly into an AI coding assistant — to generate new isolated Python Skills for `xiaohe-supervisor`.

## Quick Start from Templates

- **Device / Bridge Skill** (long-running hardware integration): copy [`templates/skills/device-starter/`](../templates/skills/device-starter/) into `skills/<skill-id>/`.
- **Action / Service Skill** (on-demand automation routine): copy [`templates/skills/action-starter/`](../templates/skills/action-starter/) into `skills/<skill-id>/`.

Validate your manifest and config offline before launching:

```bash
xiaohe validate --config-dir ./config --skills-dir ./skills
```

## Copy-Paste AI Authoring Prompt

```text
You are writing an isolated Python Skill for Xiaohe (a self-hosted smart home platform).
Follow docs/skill_spec.md strictly:
1. Create skills/<skill-id>/ containing:
   - skill.toml ([skill], [runtime] with type="python-uv" and entrypoint=["python", "src/main.py"], [config.*], [secrets.*], and [actions.*] for action skills)
   - pyproject.toml (PEP 621 dependencies managed by uv)
   - src/main.py using xiaohe_sdk.SkillApp (JSON-RPC 2.0 over stdin/stdout; NEVER print logs to stdout, use stderr only!)
   - SKILL.md starting with YAML frontmatter (`name` matching `[skill].id` and `description`), documenting config fields, secrets, entities/actions, and demo_mode testing
2. For Device Skills:
   - Register entities in @app.on_init via await app.register_entities(device={...}, entities=[...])
   - Emit state updates via await app.emit_state(entity_id, domain_state_dict)
   - Handle commands in @app.on_command() (note parentheses!)
   - Always include a `demo_mode = true` option for hardware-free verification.
3. For Action Skills:
   - Handle invocations in @app.on_invoke() (note parentheses!)
   - Read live entities with `await app.get_entity(entity_id)` and dispatch commands with `await app.call_entity(entity_id, action, params)`.
4. Verify with `xiaohe validate --config-dir ./config --skills-dir ./skills` and a Python unit test.
```

## Protocol Invariants

- **stdout is strictly reserved for newline-delimited JSON-RPC 2.0 frames**; send all logs/debug output to `sys.stderr`.
- **Scoped secrets**: declare every required secret under `[secrets.<key>]` in `skill.toml`; `xiaohe-core` injects only declared secrets at `init` / `invoke` time.
- **No external network calls in `demo_mode`**: keep `demo_mode = true` deterministic so CI and `xiaohe-verify.sh` can verify your Skill offline.
