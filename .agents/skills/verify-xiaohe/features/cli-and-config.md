# CLI & Configuration

CLI & Configuration covers the unified `xiaohe` binary's subcommands (`core`, `supervisor`, `--help`), unprefixed environment overrides (`CONFIG_DIR`, `HOST`, `PORT`, `SQLITE_PATH`, `RUNTIME_SOCKET`, `SKILLS_DIR`), and fail-fast exit behavior on malformed TOML.

## Sub-features

- `cli-help` prints subcommands `core` and `supervisor` and exits `0`.
- `cli-supervisor` reads `RUNTIME_SOCKET` / `SKILLS_DIR` and logs supervisor startup cleanly.
- `config-malformed-exit` exits with nonzero status `1` when `system.toml` is malformed.

## How to get to it (user POV)

- Run `./target/debug/xiaohe --help`.
- Run `./target/debug/xiaohe supervisor`.
- Run `CONFIG_DIR=<dir> ./target/debug/xiaohe core` against a directory containing malformed `system.toml`.

## Driving it with xiaohe CLI

Preconditions:

- Binary `./target/debug/xiaohe` is built (`cargo build`) and `$EVIDENCE_DIR` exists.

- **Help output.** Run `./target/debug/xiaohe --help | tee "${EVIDENCE_DIR}/cli-help.txt"`. Exit code is `0` and stdout lists `core` and `supervisor`.
- **Supervisor subcommand.** Run `RUNTIME_SOCKET=/tmp/test-rpc.sock SKILLS_DIR=/tmp/test-skills ./target/debug/xiaohe supervisor 2>&1 | tee "${EVIDENCE_DIR}/cli-supervisor.txt"`. Exit code is `0` and log shows `socket=/tmp/test-rpc.sock` and `skills_dir=/tmp/test-skills`.
- **Malformed `system.toml` fails fast.** Run `BAD_DIR=$(mktemp -d); printf 'bad = [toml\n' > "$BAD_DIR/system.toml"; set +e; CONFIG_DIR="$BAD_DIR" ./target/debug/xiaohe core >"${EVIDENCE_DIR}/cli-bad-config.txt" 2>&1; rc=$?; set -e; rm -rf "$BAD_DIR"; echo "EXIT_CODE=$rc" | tee -a "${EVIDENCE_DIR}/cli-bad-config.txt"`. Output contains `EXIT_CODE=1` and a TOML parse error.

## Gotchas

- `xiaohe supervisor` is currently a Phase 1 scaffold that logs initialization and exits `0` (long-running UDS supervision arrives in Phase 3).
- Always run the malformed-config check in its own temp directory so it never mutates `$SCRATCH_DIR/config/system.toml`.
