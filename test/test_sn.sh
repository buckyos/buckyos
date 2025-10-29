#!/usr/bin/env bash
set -euo pipefail

# 将此处的 pkx 替换为真实用户注册时使用的设备公钥 JWK
PUBLIC_KEY_JWK='{"crv":"Ed25519","kty":"OKP","x":"k3Dc342oNvSfZKm-Fp-IU7aRGD-JNAxoUaUIhQDWziQ"}'
DEVICE_NAME="sunxinle002" # 如需查询其他设备可更改此名称

if ! command -v jq >/dev/null 2>&1; then
  echo "jq not found, please install jq to build the request payload." >&2
  exit 1
fi

PAYLOAD="$(jq -n \
  --arg pk "$PUBLIC_KEY_JWK" \
  --arg device "$DEVICE_NAME" \
  '{method:"get_by_pk", params:{public_key:$pk, device_name:$device}, sys:[1]}')"

echo "Request payload:"
echo "$PAYLOAD"

echo "Response:"
curl --fail -sS https://sn.buckyos.ai/kapi/sn \
  -H 'Content-Type: application/json' \
  -d "$PAYLOAD" \
  | jq . || {
    echo "Failed to parse response as JSON; raw response shown above if any." >&2
    exit 1
  }
