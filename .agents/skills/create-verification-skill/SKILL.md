---
name: create-verification-skill
description: "Generate a project-local verification skill (.agents/skills/verify-<app>/) that drives Xiaohe the way a user does — launch, doctor, drive, capture evidence, and clean up. Use for /create-verification-skill, \"make a control skill for this repo\", or when a project lacks a scripted way to prove CLI/HTTP/service/UI behavior."
---

# Create a verification skill

Every serious project needs a scripted way to drive the real app and prove behavior: launch it, exercise a feature the way a user would, and capture evidence. This skill generates that as a project-local skill (`.agents/skills/verify-<app>/`) tailored to the repo. Write the generator's output for the next agent, not for a human: it will be read cold, mid-task, by an agent that has never seen the app.

## 1. Interview the repo, not the user

Answer these from the codebase and only ask the user what you cannot observe:

- **Surface:** what does a user actually touch? For Xiaohe, check what is currently built in the repo: CLI (`xiaohe core`, `xiaohe supervisor`), HTTP API (`/health`, `/health/ready`, `/api/webhooks/{id}`, and later REST/SSE), SQLite WAL persistence, ESPHome Native API, Docker Compose two-container stack, and (from Phase 6) the SolidJS web UI. Pick the primary surface(s) currently implemented and note the rest.
- **Run:** how does the app start locally? Prefer the repo's own documented dev commands (`Makefile`, `cargo run -- core`, `compose.yml`). Note exact env vars (`CONFIG_DIR`, `HOST`, `PORT`, `STORAGE_BACKEND`, `SQLITE_PATH`, `RUNTIME_SOCKET`, `SKILLS_DIR`), ports, seed configs, and auth.
- **Drive:** how can an agent interact with it programmatically? Existing harnesses first — `curl` against real HTTP endpoints, `sqlite3` read-only queries, `cargo run` CLI invocations, mock TCP/UDS peers only at real external boundaries, and browser automation once the SolidJS UI exists.
- **Observe:** what evidence can be captured? HTTP status codes + response bodies, structured logs, CLI stdout/stderr + exit codes, SQLite rows (`PRAGMA journal_mode`, table queries), and UI screenshots/ARIA snapshots once web exists.
- **Isolate:** can two instances run side by side (disposable `CONFIG_DIR`, `SQLITE_PATH`, ephemeral `PORT`, `RUNTIME_SOCKET`)? If not, say so explicitly: refusing to double-drive a shared instance beats corrupting another session.

If the checkout doesn't build or start as-is, fix that first (or report it precisely) before generating; a skill written against a broken base teaches wrong steps. When a missing asset blocks startup (a sample `system.toml` or empty `/config` dir), the generated skill may create it in a disposable scratch directory, clearly marked as verification scaffolding, and remove it in cleanup.

## 2. Generate the skill

Write `.agents/skills/verify-<app>/SKILL.md` with YAML frontmatter (`name: verify-<app>` and a `description` naming the app, the surface, and when to reach for it — without frontmatter the skill never registers) and these sections, each grounded in what the interview actually found (no placeholders left):

- **Launch:** the exact command that starts the app for verification from an empty disposable data/config dir, and how to tell it's ready (e.g., `/health/ready` returning `200 {"status":"ok","storage":"ok",...}`, not merely port open). Ready means usable, including migrations and seed configs. Include teardown. For short-lived CLIs (`xiaohe supervisor` in early phases, `xiaohe validate` later), build once and run each drive in isolation.
- **Doctor:** one read-only check that answers "is this instance worth driving?" — process PID alive, port owned by our PID, `/health/ready` healthy, expected `CONFIG_DIR` / `SQLITE_PATH`. An agent runs this first whenever anything looks off.
- **Drive:** the harness recipe with real routes, headers, env vars, and commands from this repo, not generic examples. Prefer stable handles (route paths, CLI flags, ARIA labels, data attributes) over brittle selectors.
- **Evidence:** what to capture for a proof and where it goes (e.g., `/tmp/xiaohe-verify-evidence/$RUN_ID/`). State the proof standards:
  - Exercise the real user path, not internal setters or test-only endpoints.
  - Capture the action and the resulting state, not just a status code.
  - Verify side effects (SQLite rows written, TOML files persisted, exit codes) alongside HTTP/UI output.
  - Use mocks only where a production boundary already isolates an external system (e.g., external ESPHome node or webhook caller).
- **Cleanup:** how to tear down instances the run created. **Never kill by process name** (`pkill xiaohe`); kill only the exact PID(s) recorded by this run (`kill -TERM "$PID"` + `wait`). Cleanup removes instances and disposable scratch state, **never the evidence**: proof artifacts survive teardown in the named location.
- **Helpers:** any script the skill ships (e.g., `scripts/verify-env.sh`) must be executable (`chmod +x`) and its exact invocation must be shown in `SKILL.md`. A helper the reader has to reverse-engineer is not a helper.

## 3. Seed the feature map

Create `.agents/skills/verify-<app>/features/README.md` plus one file per implemented user-facing feature (aim for the top 3–5 to start, grounded in real routes, CLI commands, and docs). Follow the shape in [`references/feature-map-example/`](references/feature-map-example/), with a README index and one file per feature.

Each feature file uses **exactly four H2 sections** in this order:
1. `Sub-features`
2. `How to get to it (user POV)`
3. `Driving it with <harness>` (starts with `Preconditions:`, then labeled bullets pairing each user action with an exact command and observable result)
4. `Gotchas`

If seed data lacks a precondition that a journey needs, report a failure and record the reason in `Gotchas` — never weaken the journey to make it pass. The map is the repo's maintained verification source; a proof that drives one convenient entry point is incomplete when the map lists others.

## 4. Prove the generated skill before handing it over

Run its own instructions end to end once:
1. **Launch**
2. **Doctor**
3. **Drive ONE mapped feature** (one is enough; the map exists so later runs can cover the rest)
4. **Capture evidence**
5. **Clean up**
6. **Confirm evidence survived cleanup** at the named location — a cleanup that eats the proof fails this step.

Fix what fails, and run the generated cleanup after every failed iteration too so broken attempts don't strand processes or ports. A generated skill that was never executed is a draft, not a deliverable. If proved in a stand-in for the real target (e.g., local binary instead of Docker Compose or mock ESPHome instead of real ESP32), list every setup step the stand-in did that the target will not.

## 5. Offer the maintenance loop

Point the user at `/maintain-verification-skill` for keeping the generated skill and feature map honest as Xiaohe evolves across phases. Suggest a cadence only if they ask.
