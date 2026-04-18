#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${SRC_DIR}/.." && pwd)"

export BUCKYOS_ROOT="${BUCKYOS_ROOT:-${REPO_ROOT}/.dev_buckyos_klog}"

ZONE_HOST="${KLOG_ZONE_HOST:-test.buckyos.io}"
KLOG_NODE_ID="${KLOG_NODE_ID:-1}"
KLOG_RAFT_PORT="${KLOG_RAFT_PORT:-21001}"
KLOG_INTER_NODE_PORT="${KLOG_INTER_NODE_PORT:-21002}"
KLOG_ADMIN_PORT="${KLOG_ADMIN_PORT:-21003}"
KLOG_RPC_PORT="${KLOG_RPC_PORT:-4070}"
KLOG_GATEWAY_CONTAINER="${KLOG_GATEWAY_CONTAINER:-dev-kapi-gateway}"

log() {
  printf '[klog-dev] %s\n' "$*"
}

fail() {
  printf '[klog-dev][error] %s\n' "$*" >&2
  exit 1
}

require_cmd() {
  local cmd="$1"
  command -v "$cmd" >/dev/null 2>&1 || fail "missing required command: ${cmd}"
}

wait_for_port() {
  local name="$1"
  local host="$2"
  local port="$3"
  local retries="${4:-60}"

  for ((i = 1; i <= retries; i++)); do
    if bash -lc "</dev/tcp/${host}/${port}" >/dev/null 2>&1; then
      log "ready: ${name} on ${host}:${port}"
      return 0
    fi
    sleep 1
  done

  fail "timeout waiting ${name} on ${host}:${port}"
}

ensure_host_resolution() {
  python3 - "$ZONE_HOST" <<'PY'
import socket
import sys

host = sys.argv[1]
try:
    print(socket.gethostbyname(host))
except OSError as err:
    raise SystemExit(f"failed to resolve {host}: {err}")
PY
}

ensure_artifact() {
  local path="$1"
  [[ -f "$path" ]] || fail "missing build artifact: ${path}"
}

ensure_placeholder_dir() {
  local dir="$1"
  local title="$2"
  mkdir -p "$dir"
  if [[ ! -f "${dir}/index.html" ]]; then
    printf '<!doctype html><title>%s</title>\n' "$title" > "${dir}/index.html"
  fi
}

ensure_local_buckycli() {
  mkdir -p "${HOME}/buckycli"
  cp "${SRC_DIR}/rootfs/bin/buckycli/buckycli" "${HOME}/buckycli/buckycli"
  chmod +x "${HOME}/buckycli/buckycli"
}

ensure_klog_bundle() {
  local bundle_dir="${BUCKYOS_ROOT}/bin/klog-service"
  local config_file="${BUCKYOS_ROOT}/etc/klog-service.toml"

  mkdir -p "${bundle_dir}" "${BUCKYOS_ROOT}/etc"
  cp "${SRC_DIR}/rootfs/bin/klog-service/klog_daemon" "${bundle_dir}/klog_daemon"
  chmod +x "${bundle_dir}/klog_daemon"

  cat > "${bundle_dir}/kernel_pkg.toml" <<'EOF'
service_name = "klog-service"
service_type = "kernel"
EOF

  cat > "${bundle_dir}/start" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$DIR/../.." && pwd)"
BIN="$DIR/klog_daemon"
PIDFILE="$DIR/klog-service.pid"
LOGDIR="$ROOT/logs/klog-service"
LOGFILE="$LOGDIR/klog-service.out.log"
mkdir -p "$LOGDIR"
export BUCKYOS_ROOT="$ROOT"
export KLOG_CONFIG_FILE="$ROOT/etc/klog-service.toml"
if [ -f "$PIDFILE" ] && kill -0 "$(cat "$PIDFILE")" 2>/dev/null; then
  exit 0
fi
nohup "$BIN" >> "$LOGFILE" 2>&1 &
echo $! > "$PIDFILE"
sleep 1
kill -0 "$(cat "$PIDFILE")"
EOF

  cat > "${bundle_dir}/stop" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
DIR="$(cd "$(dirname "$0")" && pwd)"
PIDFILE="$DIR/klog-service.pid"
if [ -f "$PIDFILE" ]; then
  PID="$(cat "$PIDFILE")"
  kill "$PID" 2>/dev/null || true
  sleep 1
  kill -9 "$PID" 2>/dev/null || true
  rm -f "$PIDFILE"
fi
exit 0
EOF

  cat > "${bundle_dir}/status" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
DIR="$(cd "$(dirname "$0")" && pwd)"
PIDFILE="$DIR/klog-service.pid"
if [ -f "$PIDFILE" ] && kill -0 "$(cat "$PIDFILE")" 2>/dev/null; then
  exit 0
fi
exit 1
EOF

  chmod +x "${bundle_dir}/start" "${bundle_dir}/stop" "${bundle_dir}/status"

  cat > "${config_file}" <<EOF
node_id = ${KLOG_NODE_ID}

[cluster]
name = "${ZONE_HOST}"
id = "${ZONE_HOST}"
auto_bootstrap = true

[network]
listen_addr = "127.0.0.1:${KLOG_RAFT_PORT}"
inter_node_listen_addr = "127.0.0.1:${KLOG_INTER_NODE_PORT}"
admin_listen_addr = "127.0.0.1:${KLOG_ADMIN_PORT}"
rpc_listen_addr = "0.0.0.0:${KLOG_RPC_PORT}"
advertise_addr = "127.0.0.1"
advertise_port = ${KLOG_RAFT_PORT}
advertise_inter_port = ${KLOG_INTER_NODE_PORT}
advertise_admin_port = ${KLOG_ADMIN_PORT}
rpc_advertise_port = ${KLOG_RPC_PORT}
enable_rpc_server = true

[admin]
local_only = true
EOF
}

start_local_buckyos() {
  log "starting local buckyos runtime at ${BUCKYOS_ROOT}"
  (
    cd "${SRC_DIR}"
    uv run start.py --all
  )

  wait_for_port "system_config" 127.0.0.1 3200
  wait_for_port "verify_hub" 127.0.0.1 3300
  wait_for_port "control_panel" 127.0.0.1 4020
}

start_klog_service() {
  local bundle_dir="${BUCKYOS_ROOT}/bin/klog-service"
  "${bundle_dir}/stop" || true
  "${bundle_dir}/start"
  wait_for_port "klog-service" 127.0.0.1 "${KLOG_RPC_PORT}"
}

ensure_port_free() {
  local port="$1"
  if ss -ltn "sport = :${port}" | tail -n +2 | grep -q .; then
    fail "port ${port} is already in use; stop the conflicting process before running this script"
  fi
}

start_min_gateway() {
  local gateway_conf="${BUCKYOS_ROOT}/etc/min_kapi_gateway.conf"

  cat > "${gateway_conf}" <<EOF
server {
    listen 80 default_server;
    listen 3180;
    server_name _;

    client_max_body_size 16m;

    location /kapi/system_config {
        proxy_pass http://127.0.0.1:3200;
    }

    location /kapi/verify-hub {
        proxy_pass http://127.0.0.1:3300;
    }

    location /kapi/control-panel {
        proxy_pass http://127.0.0.1:4020;
    }

    location /kapi/klog-service {
        proxy_pass http://127.0.0.1:${KLOG_RPC_PORT};
    }

    location / {
        return 404;
    }
}
EOF

  docker rm -f "${KLOG_GATEWAY_CONTAINER}" >/dev/null 2>&1 || true
  ensure_port_free 80
  ensure_port_free 3180

  docker run -d \
    --name "${KLOG_GATEWAY_CONTAINER}" \
    --network host \
    -v "${gateway_conf}:/etc/nginx/conf.d/default.conf:ro" \
    nginx:1.27-alpine >/dev/null

  sleep 2
  wait_for_port "zone gateway http" 127.0.0.1 80
  wait_for_port "node gateway http" 127.0.0.1 3180
}

probe_endpoints() {
  log "probing ${ZONE_HOST} service endpoints"
  curl -fsS -o /dev/null "http://${ZONE_HOST}/kapi/control-panel" || true
  curl -fsS -o /dev/null -X POST \
    "http://${ZONE_HOST}/kapi/klog-service" \
    -H 'content-type: application/json' \
    -d '{"jsonrpc":"2.0","id":1,"method":"klog.meta.query","params":[{"key":"health","strong_read":false}]}' || true
}

run_remote_tests() {
  log "running klog remote integration tests"
  (
    cd "${SRC_DIR}"
    cargo test -p buckyos-api --test klog_remote_tests -- --ignored --nocapture --test-threads=1
  )
}

main() {
  require_cmd uv
  require_cmd cargo
  require_cmd docker
  require_cmd python3
  require_cmd curl

  ensure_host_resolution
  ensure_artifact "${SRC_DIR}/rootfs/bin/buckycli/buckycli"
  ensure_artifact "${SRC_DIR}/rootfs/bin/node-daemon/node_daemon"
  ensure_artifact "${SRC_DIR}/rootfs/bin/system-config/system_config"
  ensure_artifact "${SRC_DIR}/rootfs/bin/verify-hub/verify_hub"
  ensure_artifact "${SRC_DIR}/rootfs/bin/control-panel/control_panel"
  ensure_artifact "${SRC_DIR}/rootfs/bin/klog-service/klog_daemon"

  ensure_placeholder_dir "${SRC_DIR}/rootfs/bin/node-active" "node-active placeholder"
  ensure_placeholder_dir "${SRC_DIR}/rootfs/bin/buckyos_systest" "buckyos_systest placeholder"
  ensure_placeholder_dir "${SRC_DIR}/rootfs/bin/control-panel/web" "control-panel web placeholder"
  ensure_local_buckycli
  start_local_buckyos
  ensure_klog_bundle
  start_klog_service
  start_min_gateway
  probe_endpoints
  run_remote_tests

  log "single-node klog validation passed"
}

main "$@"
