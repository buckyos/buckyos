#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")/../src/frame/desktop"
pnpm exec playwright test --config=playwright.dv.config.ts
