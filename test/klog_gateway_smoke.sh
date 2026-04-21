#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
TEST_TARGET_DIR="${REPO_ROOT}/src/target/test_klog_gateway_rpc"

wait_for_port() {
  local name="$1"
  local host="$2"
  local port="$3"
  local retries="${4:-30}"

  for ((i = 1; i <= retries; i++)); do
    if bash -lc "</dev/tcp/${host}/${port}" >/dev/null 2>&1; then
      echo "[ready] ${name} on ${host}:${port}"
      return 0
    fi
    echo "[wait] ${name} on ${host}:${port} (${i}/${retries})"
    sleep 2
  done

  echo "[timeout] ${name} on ${host}:${port}" >&2
  return 1
}

cd "${REPO_ROOT}"

echo "[diag] uv run src/check.py"
if ! uv run src/check.py; then
  echo "[diag] check.py reported non-healthy runtime; continuing with explicit port checks" >&2
fi

wait_for_port "node gateway" 127.0.0.1 3180
wait_for_port "klog-service rpc" 127.0.0.1 4070
wait_for_port "klog-service admin" 127.0.0.1 21003

echo "[run] cargo run --manifest-path test/test_klog_gateway_rpc/Cargo.toml"
CARGO_TARGET_DIR="${TEST_TARGET_DIR}" \
  cargo run --manifest-path test/test_klog_gateway_rpc/Cargo.toml
