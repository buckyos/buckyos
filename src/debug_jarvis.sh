#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF_USAGE'
Usage:
  debug_jarvis.sh [owner_user_id] [--agent <agent_id>] [options] [-- <opendan args>]

Runs the Agent Loader (opendan) of one agent built from the Jarvis template on
the host, in the foreground, as the app service of the running zone: no
container, binaries built from this source tree, the agent package and the
WebUI read from the source directories. Every agent runs as its own app (app
id = AgentId, instance <agent_id>@<owner>). Press Ctrl+C to stop it;
node-daemon then starts the app container again.

The agent must exist: create it from the desktop (Add Agent) or with
  cd test/test_opendan && deno run --config ../deno.json -A --unsafely-ignore-certificate-errors agent_target.ts --create <name>

Options:
  --agent <id>      AgentId of the agent (default: $AGENT_ID, else the owner's
                    first ready agent, else the first one bound to its app)
  --no-build        Do not run cargo; use the binaries built last time
  --installed       Use $BUCKYOS_ROOT/bin/opendan/opendan instead of a cargo build
  --port <port>     Service port (default: the port the zone assigned to the app)
  -h, --help        Show this help

Examples:
  ./debug_jarvis.sh
  ./debug_jarvis.sh devtest --agent xiaobai.test.buckyos.io --no-build
  ./debug_jarvis.sh -- --poll-ms 500

Environment:
  BUCKYOS_ROOT=/opt/buckyos
  AGENT_ID=<agent_id>
  JARVIS_PACKAGE_ROOT=src/apps/jarvis_runtime/agent
  OPENDAN_WEB_ROOT=src/frame/opendan/web/dist (falls back to $BUCKYOS_ROOT/bin/opendan/web)
EOF_USAGE
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUCKYOS_ROOT="${BUCKYOS_ROOT:-/opt/buckyos}"
AGENT_ID="${AGENT_ID:-}"
OWNER_USER_ID="devtest"
BUILD=1
INSTALLED=0
SERVICE_DEBUG_ARGS=()
OPENDAN_ARGS=()

if [[ $# -gt 0 && "${1}" != -* ]]; then
  OWNER_USER_ID="$1"
  shift
fi

while [[ $# -gt 0 ]]; do
  case "$1" in
    -h|--help)
      usage
      exit 0
      ;;
    --no-build)
      BUILD=0
      shift
      ;;
    --installed)
      INSTALLED=1
      shift
      ;;
    --agent)
      AGENT_ID="${2:?--agent requires a value}"
      shift 2
      ;;
    --port)
      SERVICE_DEBUG_ARGS+=("--port" "${2:?--port requires a value}")
      shift 2
      ;;
    --)
      shift
      OPENDAN_ARGS=("$@")
      break
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

JARVIS_PACKAGE_ROOT="${JARVIS_PACKAGE_ROOT:-${SCRIPT_DIR}/apps/jarvis_runtime/agent}"
SERVICE_DEBUG_SCRIPT="${SCRIPT_DIR}/rootfs/bin/service_debug.tsx"

if [[ ! -d "${JARVIS_PACKAGE_ROOT}" ]]; then
  echo "jarvis package directory not found: ${JARVIS_PACKAGE_ROOT}" >&2
  exit 2
fi

if ! command -v deno >/dev/null 2>&1; then
  echo "deno is required but was not found in PATH" >&2
  exit 2
fi

if [[ -z "${AGENT_ID}" ]]; then
  AGENTS="$(deno run --quiet -A "${SERVICE_DEBUG_SCRIPT}" agents "${OWNER_USER_ID}")"
  AGENT_ID="$(printf '%s\n' "${AGENTS}" | awk -F '\t' '$2 == "ready" { print $1; exit }')"
  if [[ -z "${AGENT_ID}" ]]; then
    AGENT_ID="$(printf '%s\n' "${AGENTS}" | awk -F '\t' '$2 == "bound" { print $1; exit }')"
  fi
  if [[ -z "${AGENT_ID}" ]]; then
    echo "${OWNER_USER_ID} has no agent to run; create one first (see --help)" >&2
    if [[ -n "${AGENTS}" ]]; then
      printf '%s\n' "${AGENTS}" | sed 's/^/  /' >&2
    fi
    exit 2
  fi
fi
# The agent's constructed app: app id = AgentId.
APP_ID="${AGENT_ID}"
INSTANCE_ID="${APP_ID}@${OWNER_USER_ID}"

if [[ "${INSTALLED}" -eq 1 ]]; then
  OPENDAN_BIN="${BUCKYOS_ROOT}/bin/opendan/opendan"
else
  TARGET_DIR="$(cd "${SCRIPT_DIR}" && cargo metadata --format-version 1 --no-deps | deno eval 'console.log(JSON.parse(await new Response(Deno.stdin.readable).text()).target_directory)')"
  if [[ "${BUILD}" -eq 1 ]]; then
    echo "[debug_jarvis] cargo build -p opendan -p libopendan -p agent_tool_cli_dev"
    (cd "${SCRIPT_DIR}" && cargo build -p opendan -p libopendan -p agent_tool_cli_dev)
  fi
  OPENDAN_BIN="${TARGET_DIR}/debug/opendan"
fi

if [[ ! -x "${OPENDAN_BIN}" ]]; then
  echo "opendan binary not found: ${OPENDAN_BIN}" >&2
  exit 2
fi

# The session helpers (`xagent`, `agent_tool`) sit beside the opendan binary.
export PATH="$(dirname "${OPENDAN_BIN}"):${PATH}"

WEB_ROOT="${OPENDAN_WEB_ROOT:-${SCRIPT_DIR}/frame/opendan/web/dist}"
if [[ ! -f "${WEB_ROOT}/index.html" ]]; then
  WEB_ROOT="${BUCKYOS_ROOT}/bin/opendan/web"
fi
if [[ -f "${WEB_ROOT}/index.html" ]]; then
  OPENDAN_ARGS=("--web" "${WEB_ROOT}" ${OPENDAN_ARGS[@]+"${OPENDAN_ARGS[@]}"})
  echo "[debug_jarvis] WebUI: http://127.0.0.1:<service port>/ (no login from this machine)"
else
  echo "[debug_jarvis] no built WebUI found (cd frame/opendan/web && pnpm build); starting without it"
fi

# While the container is down the zone gateway has no route to the app, so the
# WebUI and kRPC are used directly on the service port, without a zone login.
OPENDAN_ARGS=("--trust-loopback" ${OPENDAN_ARGS[@]+"${OPENDAN_ARGS[@]}"})

# One process hosts the agent. node-daemon runs it in a container that
# publishes the app's service port; once this process holds that port the
# container cannot start again until this process exits. node-daemon may
# restart the container before this process has the port: opendan then
# refuses to start, and the container is stopped again.
app_container() {
  command -v docker >/dev/null 2>&1 || return 0
  local name
  for name in $(docker ps --format '{{.Names}}' | grep '^buckyos-app-' || true); do
    if docker inspect "${name}" --format '{{range .Config.Env}}{{println .}}{{end}}' | grep -qx "BUCKYOS_APP_INSTANCE_ID=${INSTANCE_ID}"; then
      printf '%s\n' "${name}"
    fi
  done
}

echo "[debug_jarvis] ${INSTANCE_ID} on the host"
echo "[debug_jarvis] opendan: ${OPENDAN_BIN}"
echo "[debug_jarvis] package: ${JARVIS_PACKAGE_ROOT}"

for attempt in 1 2 3 4 5; do
  for name in $(app_container); do
    echo "[debug_jarvis] stopping the app container ${name}"
    docker rm -f "${name}" >/dev/null
  done
  code=0
  deno run --quiet -A \
    "${SERVICE_DEBUG_SCRIPT}" \
    "${APP_ID}" \
    "${OWNER_USER_ID}" \
    --agent-package-root "${JARVIS_PACKAGE_ROOT}" \
    --opendan-bin "${OPENDAN_BIN}" \
    ${SERVICE_DEBUG_ARGS[@]+"${SERVICE_DEBUG_ARGS[@]}"} \
    -- ${OPENDAN_ARGS[@]+"${OPENDAN_ARGS[@]}"} || code=$?
  if [[ "${code}" -eq 0 || -z "$(app_container)" ]]; then
    exit "${code}"
  fi
  echo "[debug_jarvis] the app container came back first; retrying (${attempt}/5)"
done
exit "${code}"
