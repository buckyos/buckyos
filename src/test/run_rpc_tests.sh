#!/usr/bin/env bash

set -euo pipefail

wait_for_port() {
  local name="$1"
  local host="$2"
  local port="$3"
  local retries="${4:-60}"

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

wait_for_port "node gateway" 127.0.0.1 3180
wait_for_port "system_config" 127.0.0.1 3200
wait_for_port "verify_hub" 127.0.0.1 3300
wait_for_port "control_panel" 127.0.0.1 4020

echo "[run] cargo run -p test_rbac"
cargo run -p test_rbac

echo "[run] cargo run -p test_control_panel_rpc"
cargo run -p test_control_panel_rpc
