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

if [[ $# -gt 0 && "${1}" != -* ]]; then
  OWNER_USER_ID="$1"
  shift
fi

SOURCE_ROOT="${SCRIPT_DIR}/rootfs/bin/buckyos_jarvis"
TARGET_ROOT="${BUCKYOS_ROOT}/data/home/${OWNER_USER_ID}/.local/share/${APP_ID}"
SERVICE_DEBUG_SCRIPT="${SCRIPT_DIR}/rootfs/bin/service_debug.tsx"

if [[ ! -d "${SOURCE_ROOT}" ]]; then
  echo "jarvis source directory not found: ${SOURCE_ROOT}" >&2
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

mkdir -p "${TARGET_ROOT}/behaviors"

install -m 0644 "${SOURCE_ROOT}/role.md" "${TARGET_ROOT}/role.md"
install -m 0644 "${SOURCE_ROOT}/self.md" "${TARGET_ROOT}/self.md"
cp -R "${SOURCE_ROOT}/behaviors/." "${TARGET_ROOT}/behaviors/"

echo "[debug_jarvis] synced jarvis assets to ${TARGET_ROOT}"
echo "[debug_jarvis] launching service_debug for ${APP_ID}/${OWNER_USER_ID}"

exec deno run --allow-env --allow-read --allow-net --allow-run \
  "${SERVICE_DEBUG_SCRIPT}" \
  "${APP_ID}" \
  "${OWNER_USER_ID}" \
  "$@"
