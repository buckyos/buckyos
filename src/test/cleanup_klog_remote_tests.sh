#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${SRC_DIR}/.." && pwd)"

export BUCKYOS_ROOT="${BUCKYOS_ROOT:-${REPO_ROOT}/.dev_buckyos_klog}"
KLOG_GATEWAY_CONTAINER="${KLOG_GATEWAY_CONTAINER:-dev-kapi-gateway}"
PURGE_RUNTIME=1

log() {
  printf '[klog-dev-cleanup] %s\n' "$*"
}

fail() {
  printf '[klog-dev-cleanup][error] %s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<'EOF'
Usage:
  src/test/cleanup_klog_remote_tests.sh [--keep-root]

Options:
  --keep-root   Stop processes and remove the helper gateway container, but keep BUCKYOS_ROOT on disk.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --keep-root)
      PURGE_RUNTIME=0
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      fail "unknown argument: $1"
      ;;
  esac
done

safe_to_purge() {
  local runtime_root="$1"
  local base
  base="$(basename "$runtime_root")"
  [[ "$base" == .dev_buckyos* ]]
}

stop_klog_wrapper() {
  local stop_script="${BUCKYOS_ROOT}/bin/klog-service/stop"
  if [[ -x "$stop_script" ]]; then
    log "stopping local klog-service wrapper"
    "$stop_script" || true
  fi
}

stop_buckyos() {
  log "stopping local buckyos runtime"
  (
    cd "${SRC_DIR}"
    uv run stop.py || true
  )
}

remove_gateway_container() {
  if command -v docker >/dev/null 2>&1; then
    log "removing helper gateway container ${KLOG_GATEWAY_CONTAINER}"
    docker rm -f "${KLOG_GATEWAY_CONTAINER}" >/dev/null 2>&1 || true
  fi
}

purge_runtime_root() {
  if [[ ! -e "${BUCKYOS_ROOT}" ]]; then
    log "runtime root already absent: ${BUCKYOS_ROOT}"
    return 0
  fi

  safe_to_purge "${BUCKYOS_ROOT}" || fail "refuse to purge unsafe BUCKYOS_ROOT: ${BUCKYOS_ROOT}"

  log "removing runtime root ${BUCKYOS_ROOT}"
  if rm -rf "${BUCKYOS_ROOT}" 2>/dev/null; then
    return 0
  fi

  command -v docker >/dev/null 2>&1 || fail "failed to remove ${BUCKYOS_ROOT} and docker is unavailable for privileged cleanup"

  log "falling back to docker-assisted cleanup for ${BUCKYOS_ROOT}"
  docker run --rm \
    -v "${REPO_ROOT}:/repo" \
    alpine:3.20 \
    sh -lc "rm -rf '/repo/$(basename "${BUCKYOS_ROOT}")'"

  [[ ! -e "${BUCKYOS_ROOT}" ]] || fail "runtime root still exists after docker-assisted cleanup: ${BUCKYOS_ROOT}"
}

main() {
  stop_klog_wrapper
  stop_buckyos
  remove_gateway_container

  if [[ "${PURGE_RUNTIME}" -eq 1 ]]; then
    purge_runtime_root
  else
    log "keeping runtime root ${BUCKYOS_ROOT}"
  fi

  log "cleanup finished"
}

main
