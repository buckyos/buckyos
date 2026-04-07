#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  debug_systest.sh [owner_user_id] [service_debug_args...]

Examples:
  ./debug_systest.sh
  ./debug_systest.sh devtest
  ./debug_systest.sh devtest --port 10176
  ./debug_systest.sh --port 10176
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUCKYOS_ROOT="${BUCKYOS_ROOT:-/opt/buckyos}"
APP_ID="buckyos_systest"
OWNER_USER_ID="devtest"

if [[ $# -gt 0 && "${1}" != -* ]]; then
  OWNER_USER_ID="$1"
  shift
fi

SOURCE_ROOT="${SCRIPT_DIR}/apps/sys_test"
TARGET_ROOT="${BUCKYOS_ROOT}/bin/${APP_ID}"

find_service_debug_script() {
  local candidates=(
    "${SERVICE_DEBUG_SCRIPT:-}"
    "${SCRIPT_DIR}/rootfs/bin/service_debug.tsx"
    "${BUCKYOS_ROOT}/bin/service_debug.tsx"
    "/Users/liuzhicong/project/buckyos/src/rootfs/bin/service_debug.tsx"
  )

  local candidate
  for candidate in "${candidates[@]}"; do
    if [[ -n "${candidate}" && -f "${candidate}" ]]; then
      printf '%s\n' "${candidate}"
      return 0
    fi
  done

  return 1
}

if [[ ! -d "${SOURCE_ROOT}" ]]; then
  echo "sys_test source directory not found: ${SOURCE_ROOT}" >&2
  exit 2
fi

SERVICE_DEBUG_SCRIPT="$(find_service_debug_script || true)"
if [[ -z "${SERVICE_DEBUG_SCRIPT}" ]]; then
  echo "service_debug.tsx not found. Checked:" >&2
  echo "  - \${SERVICE_DEBUG_SCRIPT}" >&2
  echo "  - ${SCRIPT_DIR}/rootfs/bin/service_debug.tsx" >&2
  echo "  - ${BUCKYOS_ROOT}/bin/service_debug.tsx" >&2
  echo "  - /Users/liuzhicong/project/buckyos/src/rootfs/bin/service_debug.tsx" >&2
  exit 2
fi

if ! command -v deno >/dev/null 2>&1; then
  echo "deno is required but was not found in PATH" >&2
  exit 2
fi

mkdir -p "${TARGET_ROOT}"
install -m 0644 "${SOURCE_ROOT}/deno.json" "${TARGET_ROOT}/deno.json"
install -m 0644 "${SOURCE_ROOT}/main.ts" "${TARGET_ROOT}/main.ts"
rm -rf "${TARGET_ROOT}/dist"
cp -R "${SOURCE_ROOT}/dist" "${TARGET_ROOT}/dist"

echo "[debug_systest] synced sys_test assets to ${TARGET_ROOT}"
echo "[debug_systest] using service_debug: ${SERVICE_DEBUG_SCRIPT}"
echo "[debug_systest] launching service_debug for ${APP_ID}/${OWNER_USER_ID}"

exec deno run --quiet -A \
  "${SERVICE_DEBUG_SCRIPT}" \
  "${APP_ID}" \
  "${OWNER_USER_ID}" \
  "$@"
