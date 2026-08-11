#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  ./build_host_buckycli.sh [install_dir]

Build buckycli for the current host target and install it to:
  ~/buckycli/buckycli

Arguments:
  install_dir  Optional install directory. Defaults to ~/buckycli.

Environment:
  BUCKYCLI_INSTALL_DIR  Optional install directory override.
  BUCKYCLI_HOST_TARGET  Optional Rust host target override.
  BUCKYCLI_TARGET_DIR   Optional Cargo target directory override.
  CARGO                 Cargo executable. Defaults to cargo.
  RUSTC                 Rustc executable. Defaults to rustc.
  PYTHON                Python executable for cargo metadata parsing. Defaults to python3.
  CARGO_TARGET_DIR      Standard Cargo target directory override.
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INSTALL_DIR="${1:-${BUCKYCLI_INSTALL_DIR:-${HOME}/buckycli}}"
CARGO_BIN="${CARGO:-cargo}"
RUSTC_BIN="${RUSTC:-rustc}"
PYTHON_BIN="${PYTHON:-python3}"

if ! command -v "${CARGO_BIN}" >/dev/null 2>&1; then
  echo "cargo is required but was not found in PATH: ${CARGO_BIN}" >&2
  exit 2
fi

if ! command -v "${RUSTC_BIN}" >/dev/null 2>&1; then
  echo "rustc is required but was not found in PATH: ${RUSTC_BIN}" >&2
  exit 2
fi

if ! command -v "${PYTHON_BIN}" >/dev/null 2>&1; then
  echo "python is required but was not found in PATH: ${PYTHON_BIN}" >&2
  exit 2
fi

HOST_TARGET="${BUCKYCLI_HOST_TARGET:-$("${RUSTC_BIN}" -vV | awk '/^host: / { print $2 }')}"
if [[ -z "${HOST_TARGET}" ]]; then
  echo "failed to detect rust host target" >&2
  exit 2
fi

cd "${SCRIPT_DIR}"

echo "[build_host_buckycli] host target: ${HOST_TARGET}"
echo "[build_host_buckycli] install dir: ${INSTALL_DIR}"
echo "[build_host_buckycli] cargo: ${CARGO_BIN}"
echo "[build_host_buckycli] rustc: ${RUSTC_BIN}"

"${CARGO_BIN}" build -p buckycli --release --target "${HOST_TARGET}"

if [[ -n "${BUCKYCLI_TARGET_DIR:-}" ]]; then
  TARGET_DIR="${BUCKYCLI_TARGET_DIR}"
elif [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
  TARGET_DIR="${CARGO_TARGET_DIR}"
else
  TARGET_DIR="$("${CARGO_BIN}" metadata --no-deps --format-version 1 | "${PYTHON_BIN}" -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
fi
TARGET_DIR="$(cd "${TARGET_DIR}" && pwd)"
BUCKYCLI_BIN=""
for candidate in \
  "${TARGET_DIR}/${HOST_TARGET}/release/buckycli" \
  "${TARGET_DIR}/release/buckycli" \
  "${SCRIPT_DIR}/target/${HOST_TARGET}/release/buckycli" \
  "${SCRIPT_DIR}/target/release/buckycli"
do
  if [[ -f "${candidate}" ]]; then
    BUCKYCLI_BIN="${candidate}"
    break
  fi
done

if [[ -z "${BUCKYCLI_BIN}" ]]; then
  echo "built buckycli not found. Checked:" >&2
  echo "  - ${TARGET_DIR}/${HOST_TARGET}/release/buckycli" >&2
  echo "  - ${TARGET_DIR}/release/buckycli" >&2
  echo "  - ${SCRIPT_DIR}/target/${HOST_TARGET}/release/buckycli" >&2
  echo "  - ${SCRIPT_DIR}/target/release/buckycli" >&2
  exit 1
fi

mkdir -p "${INSTALL_DIR}"
install -m 0755 "${BUCKYCLI_BIN}" "${INSTALL_DIR}/buckycli"

echo "[build_host_buckycli] installed: ${INSTALL_DIR}/buckycli"
if command -v file >/dev/null 2>&1; then
  file "${INSTALL_DIR}/buckycli"
fi

if [[ "$(uname -s)" == "Darwin" ]] && command -v file >/dev/null 2>&1; then
  if file "${INSTALL_DIR}/buckycli" | grep -q "ELF"; then
    echo "installed buckycli is ELF, expected a Darwin Mach-O executable" >&2
    exit 1
  fi
fi
