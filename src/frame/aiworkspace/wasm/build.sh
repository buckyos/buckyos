#!/usr/bin/env bash
# Build the WASM facade of aiworkspace-core for the Desktop app.
# Needs: rustup target add wasm32-unknown-unknown; cargo install wasm-bindgen-cli --version <same as Cargo.lock>
set -euo pipefail
cd "$(dirname "$0")/../../.."          # buckyos/src
OUT="${1:-frame/desktop/src/app/aiworkspace/wasm}"
TARGET_DIR="$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
cargo build -p aiworkspace-wasm --target wasm32-unknown-unknown --release
wasm-bindgen --target web --out-dir "$OUT" "$TARGET_DIR/wasm32-unknown-unknown/release/aiworkspace_wasm.wasm"
ls -la "$OUT"
