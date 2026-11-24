---
name: tdd
description: "Use when the user explicitly asks for TDD, a failing test, or a regression test (including addressing PR review comments), OR when a bug has an obvious cheap local test target. Skip when the test path is unclear, expensive, integration-heavy, or not requested."
---

# TDD Bug Fix (Xiaohe)

When fixing a bug or addressing a verified PR review comment with a clear, cheap test path, make the broken behavior executable before changing production code. The goal is a focused regression test that fails before the fix and passes after it.

Do not force a test when it would be impractical. If the available test would require broad harness setup, brittle mocks, slow end-to-end infrastructure, production-only state, vague reproduction steps, or large unrelated fixture churn, skip adding a new test and use the closest useful verification instead.

## Workflow

1. **Understand and verify the bug first.** Identify the intended behavior, current behavior, affected path, and smallest observable reproduction. For PR review comments (human or automated), verify the claim against the actual code before changing anything.
2. **Choose the narrowest executable check.** Prefer the closest unit or in-process integration test already used for that codepath (`cargo test <module::test_name>`). If no practical test path is obvious, do not invent one from scratch just to satisfy the workflow.
3. **Write the failing test first (Red).** Add the smallest focused test that would have caught the bug. The test must encode intended behavior, not mirror the current implementation.
4. **Run the new test before fixing.** Run `cargo test <test_name> -- --nocapture` and confirm it fails for the intended reason. If it passes immediately or fails for an unrelated reason, tighten the test or reproduction before editing production code.
5. **Fix the bug (Green).** Make the smallest production change that satisfies the intended behavior while preserving nearby contracts and MSRV (`rust-version = "1.88"`).
6. **Rerun and verify.** Confirm the regression test now passes, then run the full local gate:
   ```bash
   make fmt && make lint && make test
   ```
   Commit the regression test and implementation fix together. If iterating on a PR, push and confirm every CI check passes before considering the PR ready to merge.

## If a Failing Test Is Impractical

Use the closest executable regression check instead: a targeted script, manual reproduction command, live HTTP/CLI check, SQLite inspection, or focused integration check.

Prefer no new test over a bad test. A bad test is one that mostly tests mocks, encodes current implementation details, depends on timing or unrelated global state, needs expensive infrastructure for a small fix, or would be deleted immediately after proving the fix.

## Guardrails (Xiaohe-Specific)

- **No tautological tests.** Never compute expected values with the production helper under test (e.g., use an independent `sha256sum` vector instead of calling `hash_webhook_secret` in the fixture). Never rely solely on blind `deserialize(serialize(x)) == x` round-trips — assert exact documented wire JSON, TOML, or frame bytes. Keep mock servers decoupled from production protocol constants.
- **Test both removal and preservation.** For pruning, filtering, or RBAC rules, assert that out-of-scope data is preserved (or blocked) alongside the targeted action.
- **Do not change tests merely to match a wrong implementation**, and do not weaken existing assertions unless the expected behavior has genuinely changed and the reason is clear.
- **Keep the regression test focused on the bug.** Avoid broad fixture churn or unrelated coverage expansion.
- **Make concurrency/async tests deterministic** (e.g., `Barrier` for race reproduction, `tokio::select!` partial-read cancellation, `tokio::time::timeout` for shutdown liveness).

## Final Response

Report the evidence, not just the outcome:

- Name the failing-before test (or executable check) and the exact failure it produced.
- Name the passing-after test run and full verification results (`make fmt`, `make lint`, `make test`, and CI status when pushing to a PR).
- If failing-before evidence could not be demonstrated, state why and describe the closest regression check used instead.
