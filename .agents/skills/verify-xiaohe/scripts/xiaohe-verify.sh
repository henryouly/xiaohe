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
sup_pid_file="${scratch_dir}/supervisor.pid"
sock_file="${scratch_dir}/rpc.sock"
port_file="${scratch_dir}/port"
log_file="${scratch_dir}/core.log"
sup_log_file="${scratch_dir}/supervisor.log"
bin_path="${repo_root}/target/debug/xiaohe"

stop_pid_file() {
  local pf="$1"
  if [[ -f "$pf" ]]; then
    local p
    p="$(cat "$pf")"
    if kill -0 "$p" 2>/dev/null; then
      kill -TERM "$p" 2>/dev/null || true
      for _ in $(seq 1 30); do
        if ! kill -0 "$p" 2>/dev/null; then
          break
        fi
        sleep 0.1
      done
      if kill -0 "$p" 2>/dev/null; then
        kill -KILL "$p" 2>/dev/null || true
      fi
    fi
  fi
}

case "$cmd" in
  launch)
    port="${3:-14000}"
    if [[ -f "$pid_file" ]] && kill -0 "$(cat "$pid_file")" 2>/dev/null; then
      echo "Instance for run_id=${run_id} is already running (PID $(cat "$pid_file"))" >&2
      exit 1
    fi

    mkdir -p "${scratch_dir}/config" "${scratch_dir}/data" "${evidence_dir}"

    # Seed verification scaffolding: system.toml with a known webhook secret ("verify-secret").
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

[runtime]
socket_path = "${sock_file}"

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

    # Enable reference miio-airpurifier device skill in hardware-free demo_mode.
    cat > "${scratch_dir}/config/devices.toml" <<EOF
[skills.miio-airpurifier]
enabled = true

[skills.miio-airpurifier.config]
demo_mode = true
poll_interval_secs = 1
area_id = "living_room"
EOF

    # Seed a Phase 4 declarative TOML automation triggered by verify_hook.
    mkdir -p "${scratch_dir}/config/automations"
    cat > "${scratch_dir}/config/automations/webhook_purifier.toml" <<EOF
id = "verify_webhook_purifier"
name = "Verification Webhook Purifier Boost"
enabled = true
mode = "single"

[[triggers]]
type = "webhook"
webhook_id = "verify_hook"

[[actions]]
type = "entity.call"
entity_id = "switch.purifier_power"
action = "turn_on"

[[actions]]
type = "skill.invoke"
skill_id = "climate-optimizer"
action = "evaluate_comfort"
context_entities = ["sensor.purifier_temp", "sensor.purifier_humidity"]
[actions.params]
temp_entity = "sensor.purifier_temp"
humidity_entity = "sensor.purifier_humidity"
threshold_c = 24.0
EOF

    (cd "$repo_root" && cargo build --quiet)

    # 1. Start xiaohe supervisor on isolated UDS socket
    RUNTIME_SOCKET="${sock_file}" \
      SKILLS_DIR="${repo_root}/skills" \
      RUST_LOG="info" \
      "$bin_path" supervisor >"$sup_log_file" 2>&1 &
    sup_pid=$!
    echo "$sup_pid" > "$sup_pid_file"

    sock_ready=0
    for _ in $(seq 1 40); do
      if ! kill -0 "$sup_pid" 2>/dev/null; then
        echo "xiaohe supervisor exited prematurely; log:" >&2
        cat "$sup_log_file" >&2
        exit 1
      fi
      if [[ -S "$sock_file" ]]; then
        sock_ready=1
        break
      fi
      sleep 0.1
    done
    if [[ "$sock_ready" -ne 1 ]]; then
      echo "xiaohe supervisor did not create UDS socket ${sock_file}; log:" >&2
      cat "$sup_log_file" >&2
      exit 1
    fi

    # 2. Start xiaohe core connected to supervisor UDS socket
    echo "$port" > "$port_file"
    CONFIG_DIR="${scratch_dir}/config" \
      SQLITE_PATH="${scratch_dir}/data/xiaohe.db" \
      RUNTIME_SOCKET="${sock_file}" \
      HOST="127.0.0.1" \
      PORT="${port}" \
      RUST_LOG="info" \
      "$bin_path" core >"$log_file" 2>&1 &
    pid=$!
    echo "$pid" > "$pid_file"

    ready=0
    for _ in $(seq 1 50); do
      if ! kill -0 "$pid" 2>/dev/null; then
        echo "xiaohe core exited prematurely; log:" >&2
        cat "$log_file" >&2
        exit 1
      fi
      if curl -sf "http://127.0.0.1:${port}/health/ready" | grep -q '"runtime":"ok"'; then
        ready=1
        break
      fi
      sleep 0.2
    done

    if [[ "$ready" -ne 1 ]]; then
      echo "xiaohe core did not become ready with runtime=ok on port ${port}; core log:" >&2
      cat "$log_file" >&2
      echo "supervisor log:" >&2
      cat "$sup_log_file" >&2
      exit 1
    fi

    echo "READY run_id=${run_id} core_pid=${pid} sup_pid=${sup_pid} url=http://127.0.0.1:${port} socket=${sock_file} scratch=${scratch_dir} evidence=${evidence_dir}"
    ;;

  doctor)
    if [[ ! -f "$pid_file" || ! -f "$sup_pid_file" || ! -f "$port_file" ]]; then
      echo "UNHEALTHY: missing pid or port metadata in ${scratch_dir}" >&2
      exit 1
    fi
    pid="$(cat "$pid_file")"
    sup_pid="$(cat "$sup_pid_file")"
    port="$(cat "$port_file")"
    if ! kill -0 "$pid" 2>/dev/null; then
      echo "UNHEALTHY: core PID ${pid} is not running" >&2
      exit 1
    fi
    if ! kill -0 "$sup_pid" 2>/dev/null; then
      echo "UNHEALTHY: supervisor PID ${sup_pid} is not running" >&2
      exit 1
    fi
    if [[ ! -S "$sock_file" ]]; then
      echo "UNHEALTHY: UDS socket missing at ${sock_file}" >&2
      exit 1
    fi
    ready_body="$(curl -sf "http://127.0.0.1:${port}/health/ready")" || {
      echo "UNHEALTHY: GET /health/ready failed on port ${port}" >&2
      exit 1
    }
    if ! printf '%s' "$ready_body" | grep -q '"runtime":"ok"'; then
      echo "UNHEALTHY: expected runtime=ok in /health/ready, got: ${ready_body}" >&2
      exit 1
    fi
    if [[ ! -f "${scratch_dir}/data/xiaohe.db" ]]; then
      echo "UNHEALTHY: expected SQLite DB missing at ${scratch_dir}/data/xiaohe.db" >&2
      exit 1
    fi
    echo "DOCTOR_OK core_pid=${pid} sup_pid=${sup_pid} url=http://127.0.0.1:${port} socket=${sock_file} db=${scratch_dir}/data/xiaohe.db ready=${ready_body}"
    ;;

  cleanup)
    stop_pid_file "$pid_file"
    stop_pid_file "$sup_pid_file"
    rm -rf "$scratch_dir"
    echo "CLEANED scratch=${scratch_dir} (evidence preserved at ${evidence_dir})"
    ;;

  *)
    echo "Unknown command: $cmd (expected launch|doctor|cleanup)" >&2
    exit 2
    ;;
esac
