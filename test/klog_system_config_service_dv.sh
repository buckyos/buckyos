#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

export BUCKYOS_ROOT="${BUCKYOS_ROOT:-${REPO_ROOT}/.dev_buckyos}"
export KLOG_CLUSTER_DV_MODE="local-gateway-system-config-service"

(cd "${REPO_ROOT}/src" && cargo build -p system_config -p klog_daemon)
export SYSTEM_CONFIG_BIN="${REPO_ROOT}/src/target/debug/system_config"
export KLOG_DAEMON_BIN="${REPO_ROOT}/src/target/debug/klog_daemon"

CARGO_TARGET_DIR="${REPO_ROOT}/src/target/test_klog_system_config_service_dv" \
    cargo run --manifest-path "${REPO_ROOT}/test/test_klog_cluster_dv/Cargo.toml"
