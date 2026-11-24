#!/usr/bin/env bash
set -euo pipefail

# Helper harness for .agents/skills/verify-xiaohe/
# Usage:
#   xiaohe-verify.sh launch <run_id> [port]
#   xiaohe-verify.sh doctor <run_id>
#   xiaohe-verify.sh cleanup <run_id>

cmd="${1:-}"
run_id="${2:-}"

if [[ -z "$cmd" || -z "$run_id" ]]; then
  echo "Usage: $0 {launch|doctor|cleanup} <run_id> [port]" >&2
  exit 2
fi

repo_root="$(cd "$(dirname "$0")/../../../.." && pwd)"
scratch_dir="/tmp/xiaohe-verify-${run_id}"
evidence_dir="/tmp/xiaohe-evidence-${run_id}"
pid_file="${scratch_dir}/core.pid"
port_file="${scratch_dir}/port"
log_file="${scratch_dir}/core.log"
bin_path="${repo_root}/target/debug/xiaohe"

case "$cmd" in
  launch)
    port="${3:-14000}"
    if [[ -f "$pid_file" ]] && kill -0 "$(cat "$pid_file")" 2>/dev/null; then
      echo "Instance for run_id=${run_id} is already running (PID $(cat "$pid_file"))" >&2
      exit 1
    fi

    mkdir -p "${scratch_dir}/config" "${scratch_dir}/data" "${evidence_dir}"

    # Seed verification scaffolding: system.toml with a known webhook secret ("verify-secret").
    # SHA-256("verify-secret") computed independently:
    secret_hash="sha256:$(printf '%s' 'verify-secret' | sha256sum | awk '{print $1}')"
    cat > "${scratch_dir}/config/system.toml" <<EOF
[server]
host = "127.0.0.1"
port = ${port}

[storage]
backend = "sqlite"

[storage.sqlite]
path = "${scratch_dir}/data/xiaohe.db"
retention_days = 30

[[webhooks]]
id = "verify_hook"
name = "Verification Webhook"
secret_hash = "${secret_hash}"
enabled = true

[[webhooks]]
id = "disabled_hook"
name = "Disabled Verification Webhook"
secret_hash = "${secret_hash}"
enabled = false
EOF

    (cd "$repo_root" && cargo build --quiet)

    echo "$port" > "$port_file"
    CONFIG_DIR="${scratch_dir}/config" \
      SQLITE_PATH="${scratch_dir}/data/xiaohe.db" \
      HOST="127.0.0.1" \
      PORT="${port}" \
      RUST_LOG="info" \
      "$bin_path" core >"$log_file" 2>&1 &
    pid=$!
    echo "$pid" > "$pid_file"

    ready=0
    for _ in $(seq 1 40); do
      if ! kill -0 "$pid" 2>/dev/null; then
        echo "xiaohe core exited prematurely; log:" >&2
        cat "$log_file" >&2
        exit 1
      fi
      if curl -sf "http://127.0.0.1:${port}/health/ready" | grep -q '"storage":"ok"'; then
        ready=1
        break
      fi
      sleep 0.2
    done

    if [[ "$ready" -ne 1 ]]; then
      echo "xiaohe core did not become ready on port ${port}; log:" >&2
      cat "$log_file" >&2
      exit 1
    fi

    echo "READY run_id=${run_id} pid=${pid} url=http://127.0.0.1:${port} scratch=${scratch_dir} evidence=${evidence_dir}"
    ;;

  doctor)
    if [[ ! -f "$pid_file" || ! -f "$port_file" ]]; then
      echo "UNHEALTHY: missing pid or port metadata in ${scratch_dir}" >&2
      exit 1
    fi
    pid="$(cat "$pid_file")"
    port="$(cat "$port_file")"
    if ! kill -0 "$pid" 2>/dev/null; then
      echo "UNHEALTHY: PID ${pid} is not running" >&2
      exit 1
    fi
    ready_body="$(curl -sf "http://127.0.0.1:${port}/health/ready")" || {
      echo "UNHEALTHY: GET /health/ready failed on port ${port}" >&2
      exit 1
    }
    if [[ ! -f "${scratch_dir}/data/xiaohe.db" ]]; then
      echo "UNHEALTHY: expected SQLite DB missing at ${scratch_dir}/data/xiaohe.db" >&2
      exit 1
    fi
    echo "DOCTOR_OK pid=${pid} url=http://127.0.0.1:${port} db=${scratch_dir}/data/xiaohe.db ready=${ready_body}"
    ;;

  cleanup)
    if [[ -f "$pid_file" ]]; then
      pid="$(cat "$pid_file")"
      if kill -0 "$pid" 2>/dev/null; then
        kill -TERM "$pid" 2>/dev/null || true
        for _ in $(seq 1 25); do
          if ! kill -0 "$pid" 2>/dev/null; then
            break
          fi
          sleep 0.1
        done
        if kill -0 "$pid" 2>/dev/null; then
          kill -KILL "$pid" 2>/dev/null || true
        fi
      fi
    fi
    rm -rf "$scratch_dir"
    echo "CLEANED scratch=${scratch_dir} (evidence preserved at ${evidence_dir})"
    ;;

  *)
    echo "Unknown command: $cmd (expected launch|doctor|cleanup)" >&2
    exit 2
    ;;
esac
