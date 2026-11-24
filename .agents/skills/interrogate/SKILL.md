---
name: interrogate
description: "Use for \"interrogate\", \"adversarial review\", \"challenge this\", \"stress test this code\", \"find blind spots\", \"tear this apart\", or before merging high-risk PRs. Parallel adversaries try to break a change or design, and only findings proven by an executable run survive."
---

# Interrogate (Xiaohe)

Parallel adversaries try to break a change, module, or design. **A finding survives only when an executable run proves it breaks the stated intent.** Report the verified findings. **Do not apply fixes during an interrogate run** (hand verified findings to `/tdd` afterward).

## 1. Frame

1. **Scope.** Use whatever the user names (files, module, PR, or design plan). Otherwise default to the branch diff from `git merge-base main HEAD` through the current working tree (`git diff "$(git merge-base main HEAD)"`).
2. **Intent.** Restate in 2–4 plain sentences what the change is supposed to achieve, derived from the user's message, commits, PR description, and relevant `docs/` specs (`docs/api.md`, `docs/skill_spec.md`, `docs/automation_spec.md`, `docs/architecture.md`). Ask the user only when the intent is genuinely ambiguous.

## 2. Prepare Isolation

Because subagents in this environment run with `Workspace: "inherit"`, parallel adversaries **must never edit tracked files in the main repository root**.

Choose a unique `<run_id>` (e.g., `$(date +%s)`):
- Create per-adversary scratch directories under `/tmp/xiaohe-interrogate-<run_id>/<adversary>/` for Python, `curl`, SQLite, CLI, or live-binary repros.
- If an adversary needs to compile or run a Rust repro test that adds/edits `.rs` code, create a detached `git worktree` per adversary and apply any uncommitted working-tree diff onto it:
  ```bash
  mkdir -p "/tmp/xiaohe-interrogate-${RUN_ID}"
  git worktree add --detach "/tmp/xiaohe-interrogate-${RUN_ID}/adv-1" HEAD
  # If there are uncommitted changes in the main tree:
  git diff HEAD > "/tmp/xiaohe-interrogate-${RUN_ID}/working.patch"
  [ -s "/tmp/xiaohe-interrogate-${RUN_ID}/working.patch" ] && \
    git -C "/tmp/xiaohe-interrogate-${RUN_ID}/adv-1" apply "/tmp/xiaohe-interrogate-${RUN_ID}/working.patch"
  ```
  Run `cargo test` inside the worktree with `CARGO_TARGET_DIR="/tmp/xiaohe-interrogate-${RUN_ID}/target-<adversary>"` (or reuse `target/` only sequentially) so parallel builds never contend on locks.

## 3. Fan Out

Spawn **2–3 adversaries concurrently** in a single `invoke_subagent` call (`TypeName: "self"`, `Workspace: "inherit"`, `Model: "inherit"`), assigning each a distinct Xiaohe attack lens:

1. **Adversary 1 — Concurrency, Lifecycle & Security**:
   `tokio::select!` cancel-safety, dropped `watch` / lagged `broadcast` channels, UDS / mutex deadlocks, child-process env/secret leakage, constant-time auth & RBAC/area-scoping bypasses, `SIGTERM` shutdown hangs or orphan processes.
2. **Adversary 2 — State, Storage & Protocol Edge Cases**:
   TOML parsing/validation bypasses, SQLite WAL / downsampling / retention boundary bugs, `StateStore` causal ordering & placeholder-history leaks, debounce / edge-trigger / concurrency-mode races, JSON-RPC 2.0 / ESPHome Protobuf / Noise frame limits, Python SDK stdio framing.
3. **Adversary 3 — Live Binary & Integration** *(include when scope touches HTTP routes, CLI, `xiaohe supervisor`, `SkillBridge`, or `AutomationEngine`)*:
   Drives the real compiled `xiaohe` binary on an isolated port/socket/temp dir (never touching running user instances or repo `data/`).

Give every adversary the **Scope**, the **Intent**, its isolated **worktree/scratch path**, its **lens**, and this exact brief:

> Break this change under your assigned lens. Prove each finding with an executable run against the real code — such as a failing `cargo test`, a Python repro script, a `curl` / SQLite / UDS probe, or command output demonstrating wrong behavior. For a design or plan, run a prototype or probe the current tree. Keep all repro files strictly inside your assigned scratch/worktree path; never modify tracked files in the main repo root. Report each finding with:
> 1. Title
> 2. Exact runnable command
> 3. Observed output printed by the command
> 4. Expected output required by the intent
>
> Leave out anything you did not run. That excludes style, naming, structure, lint preferences, and hypothetical risks that require a future change to matter. "No findings" is a valid result.

## 4. Verify (Coordinator Gate)

Once all adversaries return, the coordinator independently verifies every reported finding:

1. **Re-run** the exact command reported by each adversary against the real code/worktree.
2. **Keep** a finding only when the observed output reproduces a real failure **and** that failure breaks the stated intent.
3. **Drop** findings that fail to reproduce, only test a broken mock, violate no part of the intent, or are style/speculative opinions. Record the drop count and brief reason per adversary.
4. **Merge** duplicate findings that share a single root cause, crediting every adversary that surfaced it.

Do not pick a single "winning" adversary — adversaries catch different failure classes, so the final verdict keeps **every** verified finding across all adversaries.

## 5. Cleanup

Unconditionally remove all temporary worktrees and scratch files before finishing (even if verification fails):

```bash
for wt in "/tmp/xiaohe-interrogate-${RUN_ID}"/adv-*; do
  [ -d "$wt" ] && git worktree remove --force "$wt" 2>/dev/null || true
done
rm -rf "/tmp/xiaohe-interrogate-${RUN_ID}"
```

Confirm `git status --short` and `git worktree list` match their pre-run state.

## 6. Output

Present the verdict concisely in this exact structure:

- **Intent**: the plain-language restatement from Step 1.
- **Findings**: for each verified finding — title, exact command, observed output, how it breaks the intent, and which adversary/adversaries found it. Say **"No verified findings."** when none survive.
- **Dropped**: per adversary — count of dropped findings and why.
- **Next step**: confirm no production code was modified, and offer to fix any verified findings using `/tdd`.
