#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")/test_control_panel"
deno run --config ../deno.json --allow-net --allow-read --allow-env \
  --unsafely-ignore-certificate-errors \
  test_user_groups.ts
