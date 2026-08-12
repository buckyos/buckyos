#!/usr/bin/env bash
set -euo pipefail

DEFAULT_CERT_PATH="${HOME}/.buckycli/buckyos_test_ca_ca_cert.pem"
SYSTEM_KEYCHAIN="/Library/Keychains/System.keychain"

usage() {
  cat <<EOF
Usage:
  ./install_test_ca.sh [certificate_path]

Install the BuckyOS test CA certificate into the macOS system keychain.

Arguments:
  certificate_path  Optional CA certificate path.
                    Defaults to ${DEFAULT_CERT_PATH}
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

if [[ $# -gt 1 ]]; then
  usage >&2
  exit 2
fi

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "this script only supports macOS" >&2
  exit 2
fi

CERT_PATH="${1:-${DEFAULT_CERT_PATH}}"

if [[ ! -f "${CERT_PATH}" ]]; then
  echo "CA certificate not found: ${CERT_PATH}" >&2
  exit 2
fi

for command_name in openssl security sudo; do
  if ! command -v "${command_name}" >/dev/null 2>&1; then
    echo "required command not found: ${command_name}" >&2
    exit 2
  fi
done

if ! openssl x509 -in "${CERT_PATH}" -noout >/dev/null 2>&1; then
  echo "invalid X.509 PEM certificate: ${CERT_PATH}" >&2
  exit 2
fi

if ! openssl x509 -in "${CERT_PATH}" -noout -text \
  | grep -E 'CA:[[:space:]]*TRUE' >/dev/null; then
  echo "certificate is not a CA certificate (CA:TRUE is missing): ${CERT_PATH}" >&2
  exit 2
fi

if ! openssl verify -CAfile "${CERT_PATH}" "${CERT_PATH}" >/dev/null 2>&1; then
  echo "certificate is expired, not yet valid, or not a valid self-signed root CA: ${CERT_PATH}" >&2
  exit 2
fi

CERT_FINGERPRINT="$(
  openssl x509 -in "${CERT_PATH}" -noout -fingerprint -sha256 \
    | sed 's/.*=//; s/://g' \
    | tr '[:lower:]' '[:upper:]'
)"

if security verify-cert -q -L -l -c "${CERT_PATH}" -k "${SYSTEM_KEYCHAIN}" >/dev/null 2>&1; then
  echo "BuckyOS test CA is already trusted."
  echo "SHA-256: ${CERT_FINGERPRINT}"
  exit 0
fi

echo "Installing BuckyOS test CA into ${SYSTEM_KEYCHAIN}"
echo "SHA-256: ${CERT_FINGERPRINT}"
sudo security add-trusted-cert \
  -d \
  -r trustRoot \
  -k "${SYSTEM_KEYCHAIN}" \
  "${CERT_PATH}"

if ! security verify-cert -q -L -l -c "${CERT_PATH}" -k "${SYSTEM_KEYCHAIN}" >/dev/null 2>&1; then
  echo "certificate was imported, but macOS trust verification failed" >&2
  exit 1
fi

echo "BuckyOS test CA installed and trusted successfully."
