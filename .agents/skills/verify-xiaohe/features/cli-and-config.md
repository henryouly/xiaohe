# CLI & Configuration

CLI & Configuration covers the unified `xiaohe` binary's subcommands (`core`, `supervisor`, `--help`), unprefixed environment overrides (`CONFIG_DIR`, `HOST`, `PORT`, `STORAGE_BACKEND`, `SQLITE_PATH`, `RUNTIME_SOCKET`, `SKILLS_DIR`), and fail-fast exit behavior on malformed TOML.

## Sub-features

- `cli-help` prints subcommands `core` and `supervisor` and exits `0`.
- `cli-supervisor` reads `RUNTIME_SOCKET` / `SKILLS_DIR`, scans manifests, binds the UDS socket, and exits `0` cleanly on `SIGTERM`.
- `config-malformed-exit` exits with nonzero status `1` when `system.toml` is malformed.

## How to get to it (user POV)

- Run `./target/debug/xiaohe --help`.
- Run `RUNTIME_SOCKET=<sock> SKILLS_DIR=<dir> ./target/debug/xiaohe supervisor` and stop it with `SIGTERM`.
- Run `CONFIG_DIR=<dir> ./target/debug/xiaohe core` against a directory containing malformed `system.toml`.

## Driving it with xiaohe CLI

Preconditions:

- Binary `./target/debug/xiaohe` is built (`cargo build`) and `$EVIDENCE_DIR` exists.

- **Help output.** Run `./target/debug/xiaohe --help | tee "${EVIDENCE_DIR}/cli-help.txt"`. Exit code is `0` and stdout lists `core` and `supervisor`.
- **Supervisor standalone startup & SIGTERM.** Run `SUP_TMP=$(mktemp -d); RUNTIME_SOCKET="$SUP_TMP/rpc.sock" SKILLS_DIR=./skills ./target/debug/xiaohe supervisor >"${EVIDENCE_DIR}/cli-supervisor.txt" 2>&1 & spid=$!; for _ in $(seq 1 30); do [ -S "$SUP_TMP/rpc.sock" ] && break; sleep 0.1; done; test -S "$SUP_TMP/rpc.sock"; kill -TERM "$spid"; wait "$spid"; echo "EXIT_CODE=$?" >> "${EVIDENCE_DIR}/cli-supervisor.txt"; test ! -e "$SUP_TMP/rpc.sock"; rm -rf "$SUP_TMP"`. Output contains `discovered_skills=3`, `supervisor UDS RPC server listening`, `supervisor UDS RPC server stopped`, and `EXIT_CODE=0`.
- **Malformed `system.toml` fails fast.** Run `BAD_DIR=$(mktemp -d); printf 'bad = [toml\n' > "$BAD_DIR/system.toml"; set +e; CONFIG_DIR="$BAD_DIR" ./target/debug/xiaohe core >"${EVIDENCE_DIR}/cli-bad-config.txt" 2>&1; rc=$?; set -e; rm -rf "$BAD_DIR"; echo "EXIT_CODE=$rc" | tee -a "${EVIDENCE_DIR}/cli-bad-config.txt"`. Output contains `EXIT_CODE=1` and a TOML parse error.

## Gotchas

- `xiaohe supervisor` is a long-running UDS server as of Phase 3; always send `SIGTERM` (do not expect it to exit immediately on its own).
- Always run the malformed-config check in its own temp directory so it never mutates `$SCRATCH_DIR/config/system.toml`.
