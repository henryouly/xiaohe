# Offline Validation CLI, InfluxDB Backend & Starter Skill Templates

`xiaohe validate` statically validates `config/{system,areas,devices,secrets,users}.toml`, `config/automations/*.toml`, and optional `skills/*/skill.toml` manifests offline (exiting `0` on valid and `1` on errors, with optional `--json` output). Xiaohe also supports `STORAGE_BACKEND=influxdb` via `InfluxDbStore` (InfluxDB v2 HTTP API + optional `compose.yml` `influxdb` profile) and bundles runnable starter Skill templates under `templates/skills/`.

## Sub-features

- **`xiaohe validate` CLI**: validates clean workspaces (`exit 0`), rejects broken TOML/schemas/InfluxDB configs/nonexistent `--skills-dir` (`exit 1`), and emits machine-readable JSON via `--json`.
- **InfluxDB v2 `HistoryStore` backend**: `STORAGE_BACKEND=influxdb` with `INFLUXDB_URL`, `INFLUXDB_ORG`, `INFLUXDB_BUCKET`, `INFLUXDB_TOKEN` (or `token_secret` in `secrets.toml`), plus optional `compose.yml --profile influxdb`.
- **Starter Skill templates**: `templates/skills/device-starter` and `templates/skills/action-starter` pass manifest validation and execute end-to-end JSON-RPC 2.0 over stdin/stdout via `xiaohe_sdk.SkillApp`.

## How to get to it (user POV)

Run `./target/debug/xiaohe validate --config-dir <DIR> [--skills-dir <DIR>] [--json]` against any config directory, or run `python3 -m unittest discover -s sdk/python/tests -v` to exercise the starter templates.

## Driving it with `xiaohe` CLI + Python unittest

Preconditions: Binary built via `cargo build`.

```bash
TMP_CFG=$(mktemp -d /tmp/xiaohe-validate-XXXXXX)
trap 'rm -rf "$TMP_CFG"' EXIT
mkdir -p "$TMP_CFG/automations"

# 1. Valid config + starter templates -> exit 0
./target/debug/xiaohe validate --config-dir "$TMP_CFG" --skills-dir ./templates/skills --json \
  | tee "$EVIDENCE_DIR/validate-ok.json" | jq -e '.skills_checked == 2'

# 2. Missing explicit --skills-dir -> exit 1
! ./target/debug/xiaohe validate --config-dir "$TMP_CFG" --skills-dir "$TMP_CFG/missing" \
  | tee "$EVIDENCE_DIR/validate-missing-skills.txt"

# 3. Invalid InfluxDB config (missing [storage.influxdb]) -> exit 1
printf '[storage]\nbackend = "influxdb"\n' > "$TMP_CFG/system.toml"
! ./target/debug/xiaohe validate --config-dir "$TMP_CFG" \
  | tee "$EVIDENCE_DIR/validate-bad-influx.txt"

# 4. End-to-end JSON-RPC 2.0 tests for SDK + starter Skill templates
python3 -m unittest discover -s sdk/python/tests -v | tee "$EVIDENCE_DIR/templates-test.txt"
```

## Gotchas

- Testing `InfluxDbStore` against a live InfluxDB v2 server requires starting `docker compose --profile influxdb up -d influxdb`; offline CI proves all HTTP endpoints, Line Protocol/Flux escaping, RFC-4180 CSV parsing, and `/api/v2/delete` pruning against an in-process `axum` mock InfluxDB v2 server (`cargo test storage::influxdb`).
- `xiaohe validate` warns (non-fatal) when `secrets.toml` or `users.toml` is readable by group/others (`mode & 0o077 != 0`), or when an automation references an entity that is only discovered at runtime.
