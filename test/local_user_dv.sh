#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")/test_control_panel"
deno run --config ../deno.json --allow-net --allow-read --allow-env --allow-run=uv \
  --unsafely-ignore-certificate-errors \
  test_local_user.ts
