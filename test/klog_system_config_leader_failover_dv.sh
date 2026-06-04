#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
TEST_TARGET_DIR="${REPO_ROOT}/src/target/test_klog_system_config_leader_failover_dv"

cd "${REPO_ROOT}"

export BUCKYOS_ROOT="${BUCKYOS_ROOT:-${REPO_ROOT}/.dev_buckyos}"
export KLOG_CLUSTER_DV_MODE="local-gateway-system-config-leader-failover"

if [[ -z "${SYSTEM_CONFIG_BIN:-}" || -z "${KLOG_DAEMON_BIN:-}" ]]; then
  echo "[build] cargo build -p system_config -p klog_daemon"
  (cd "${REPO_ROOT}/src" && OPENSSL_NO_VENDOR="${OPENSSL_NO_VENDOR:-1}" cargo build -p system_config -p klog_daemon)
  export SYSTEM_CONFIG_BIN="${SYSTEM_CONFIG_BIN:-${REPO_ROOT}/src/target/debug/system_config}"
  export KLOG_DAEMON_BIN="${KLOG_DAEMON_BIN:-${REPO_ROOT}/src/target/debug/klog_daemon}"
fi

echo "[diag] BUCKYOS_ROOT=${BUCKYOS_ROOT}"
echo "[diag] CYFS_GATEWAY_BIN=${CYFS_GATEWAY_BIN:-<auto>}"
echo "[diag] SYSTEM_CONFIG_BIN=${SYSTEM_CONFIG_BIN:-<auto>}"
echo "[diag] KLOG_DAEMON_BIN=${KLOG_DAEMON_BIN:-<auto>}"
echo "[run] cargo run --manifest-path test/test_klog_cluster_dv/Cargo.toml"

CARGO_TARGET_DIR="${TEST_TARGET_DIR}" \
  cargo run --manifest-path test/test_klog_cluster_dv/Cargo.toml
