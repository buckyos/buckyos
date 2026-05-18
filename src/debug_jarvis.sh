#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  debug_jarvis.sh [owner_user_id] [service_debug_args...]

Examples:
  ./debug_jarvis.sh
  ./debug_jarvis.sh devtest
  ./debug_jarvis.sh devtest --port 14060
  ./debug_jarvis.sh --port 14060

This script always runs the Jarvis OpenDAN runtime in the foreground.
Press Ctrl+C to stop it.

Environment:
  BUCKYOS_ROOT=/opt/buckyos
  JARVIS_PACKAGE_ROOT=src/rootfs/bin/buckyos_jarvis
  DEBUG_JARVIS_REFRESH=1
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUCKYOS_ROOT="${BUCKYOS_ROOT:-/opt/buckyos}"
APP_ID="jarvis"
OWNER_USER_ID="devtest"
DEBUG_JARVIS_REFRESH="${DEBUG_JARVIS_REFRESH:-1}"

if [[ $# -gt 0 && "${1}" != -* ]]; then
  OWNER_USER_ID="$1"
  shift
fi

for arg in "$@"; do
  if [[ "${arg}" == "--detach" ]]; then
    echo "debug_jarvis.sh runs OpenDAN in the foreground; --detach is not supported" >&2
    exit 2
  fi
done

JARVIS_PACKAGE_ROOT="${JARVIS_PACKAGE_ROOT:-${SCRIPT_DIR}/rootfs/bin/buckyos_jarvis}"
TARGET_ROOT="${BUCKYOS_ROOT}/data/home/${OWNER_USER_ID}/.local/share/${APP_ID}"
SERVICE_DEBUG_SCRIPT="${SCRIPT_DIR}/rootfs/bin/service_debug.tsx"

if [[ ! -d "${JARVIS_PACKAGE_ROOT}" ]]; then
  echo "jarvis package directory not found: ${JARVIS_PACKAGE_ROOT}" >&2
  exit 2
fi

if [[ ! -f "${SERVICE_DEBUG_SCRIPT}" ]]; then
  echo "service_debug script not found: ${SERVICE_DEBUG_SCRIPT}" >&2
  exit 2
fi

if ! command -v deno >/dev/null 2>&1; then
  echo "deno is required but was not found in PATH" >&2
  exit 2
fi

if [[ "${DEBUG_JARVIS_REFRESH}" != "0" ]]; then
  mkdir -p "${TARGET_ROOT}"
  for file in agent.toml role.md self.md; do
    if [[ -f "${JARVIS_PACKAGE_ROOT}/${file}" ]]; then
      cp "${JARVIS_PACKAGE_ROOT}/${file}" "${TARGET_ROOT}/${file}"
      chmod 0644 "${TARGET_ROOT}/${file}"
    fi
  done
  for dir in behaviors tool_plans tools; do
    if [[ -d "${JARVIS_PACKAGE_ROOT}/${dir}" ]]; then
      mkdir -p "${TARGET_ROOT}/${dir}"
      cp -R "${JARVIS_PACKAGE_ROOT}/${dir}/." "${TARGET_ROOT}/${dir}/"
    fi
  done
  echo "[debug_jarvis] refreshed editable jarvis assets in ${TARGET_ROOT}"
fi

echo "[debug_jarvis] launching foreground service_debug for ${APP_ID}/${OWNER_USER_ID}"
echo "[debug_jarvis] jarvis package root: ${JARVIS_PACKAGE_ROOT}"

exec deno run --quiet -A \
  "${SERVICE_DEBUG_SCRIPT}" \
  "${APP_ID}" \
  "${OWNER_USER_ID}" \
  --agent-package-root "${JARVIS_PACKAGE_ROOT}" \
  "$@"
