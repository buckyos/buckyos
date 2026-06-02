#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
TEST_TARGET_DIR="${REPO_ROOT}/src/target/test_klog_raft_snapshot_install_crash_dv"

cd "${REPO_ROOT}"

export BUCKYOS_ROOT="${BUCKYOS_ROOT:-${REPO_ROOT}/.dev_buckyos}"
export KLOG_CLUSTER_DV_MODE="local-gateway-raft-snapshot-install-crash"

if [[ -z "${KLOG_DAEMON_BIN:-}" ]]; then
  echo "[build] cargo build -p klog_daemon --bin klog_daemon"
  (cd "${REPO_ROOT}/src" && cargo build -p klog_daemon --bin klog_daemon)
  export KLOG_DAEMON_BIN="${REPO_ROOT}/src/target/debug/klog_daemon"
fi

echo "[diag] BUCKYOS_ROOT=${BUCKYOS_ROOT}"
echo "[diag] CYFS_GATEWAY_BIN=${CYFS_GATEWAY_BIN:-<auto>}"
echo "[diag] KLOG_DAEMON_BIN=${KLOG_DAEMON_BIN:-<auto>}"
echo "[diag] KLOG_RAFT_SNAPSHOT_INSTALL_CRASH_ITEMS=${KLOG_RAFT_SNAPSHOT_INSTALL_CRASH_ITEMS:-<default>}"
echo "[diag] KLOG_RAFT_SNAPSHOT_INSTALL_CRASH_VALUE_BYTES=${KLOG_RAFT_SNAPSHOT_INSTALL_CRASH_VALUE_BYTES:-<default>}"
echo "[diag] KLOG_RAFT_SNAPSHOT_INSTALL_CRASH_CHUNK_BYTES=${KLOG_RAFT_SNAPSHOT_INSTALL_CRASH_CHUNK_BYTES:-<default>}"
echo "[run] cargo run --manifest-path test/test_klog_cluster_dv/Cargo.toml"

CARGO_TARGET_DIR="${TEST_TARGET_DIR}" \
  cargo run --manifest-path test/test_klog_cluster_dv/Cargo.toml
