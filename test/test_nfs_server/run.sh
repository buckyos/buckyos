#!/usr/bin/env bash
#
# End-to-end verification of nfs_server through the TS clients
# (src/frame/desktop/src/api/nfsp_client.ts et al.):
#   - nfsp:       verify_nfsp.ts           (protocol client, 18 cases)
#   - browser:    verify_browser_client.ts (caching client, 11 cases)
#   - persistent: verify_persistent.ts     (stable-dir suite)
#
# nfsp/browser get their own freshly-started nfs_server over an empty local
# export directory (the scripts assert against a fresh tree, and dir
# revisions change on every server restart by design).
#
# persistent runs against a STABLE work dir, $PERSIST_ROOT (default
# /tmp/nfs_server_test), which is NEVER deleted: each run verifies the
# previous run's files/collection/view survived the restart, exercises
# fs + view + collection over the persistent tree, a 20MB large-file
# upload/resume/dedup flow, and the obj_id → manual named-store read path.
#
# Usage:
#   ./run.sh              # run all three suites
#   ./run.sh nfsp         # protocol suite only
#   ./run.sh browser      # caching suite only
#   ./run.sh persistent   # stable-dir suite only (state kept across runs)
#
# Requires: cargo, node >= 23 (TS type stripping; persistent also needs
# node:sqlite for the manual named-store read). Runs everything locally;
# the verify scripts read/write the export dir directly (bypass-write tests).
set -euo pipefail

SUITE="${1:-all}"
case "$SUITE" in nfsp|browser|persistent|all) ;; *) echo "usage: $0 [nfsp|browser|persistent|all]" >&2; exit 2 ;; esac

PERSIST_ROOT="${PERSIST_ROOT:-/tmp/nfs_server_test}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

NODE_MAJOR="$(node -p 'process.versions.node.split(".")[0]' 2>/dev/null || echo 0)"
if [ "$NODE_MAJOR" -lt 23 ]; then
  echo "node >= 23 required (found: $(node --version 2>/dev/null || echo 'none'))" >&2
  exit 1
fi

echo "==> building nfs_server"
cargo build -p nfs_server --manifest-path "$REPO_ROOT/src/frame/nfs_server/Cargo.toml"
TARGET_DIR="$(cargo metadata --format-version 1 --no-deps \
  --manifest-path "$REPO_ROOT/src/frame/nfs_server/Cargo.toml" \
  | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
BIN="$TARGET_DIR/debug/nfs_server"
[ -x "$BIN" ] || { echo "nfs_server binary not found at $BIN" >&2; exit 1; }

WORK="$(mktemp -d -t nfs_server_test.XXXXXX)"
SERVER_PID=""
FAILED=0

cleanup() {
  [ -n "$SERVER_PID" ] && kill "$SERVER_PID" 2>/dev/null || true
  if [ "$FAILED" -ne 0 ]; then
    echo "==> kept work dir for inspection: $WORK"
  else
    rm -rf "$WORK"
  fi
}
trap cleanup EXIT

stop_server() {
  if [ -n "$SERVER_PID" ]; then
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
    SERVER_PID=""
  fi
}

# suite_base <name> — persistent lives in the stable root, others under $WORK
suite_base() {
  if [ "$1" = persistent ]; then echo "$PERSIST_ROOT/persistent"; else echo "$WORK/$1"; fi
}

# start_server <name> <port> [extra flags...] — home/data dirs under suite_base
start_server() {
  local name="$1" port="$2"; shift 2
  local base; base="$(suite_base "$name")"
  mkdir -p "$base/home" "$base/data"
  "$BIN" --listen "127.0.0.1:$port" \
         --data-dir "$base/data" \
         --export "home=$base/home" \
         --scan-interval-secs 0 \
         "$@" > "$base/server.log" 2>&1 &
  SERVER_PID=$!
  for _ in $(seq 1 50); do
    if (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null; then exec 3>&- 3<&-; return 0; fi
    kill -0 "$SERVER_PID" 2>/dev/null || { echo "server died, log:" >&2; cat "$base/server.log" >&2; return 1; }
    sleep 0.1
  done
  echo "server did not start listening on port $port" >&2
  return 1
}

run_suite() {
  local name="$1" port="$2" script="$3"; shift 3
  local base; base="$(suite_base "$name")"
  echo "==> suite '$name': nfs_server on 127.0.0.1:$port, export=$base/home"
  start_server "$name" "$port" "$@"
  if NFSP_BASE="http://127.0.0.1:$port" NFSP_HOME="$base/home" NFSP_DATA="$base/data" \
      node "$SCRIPT_DIR/$script"; then
    echo "==> suite '$name' PASSED"
  else
    echo "==> suite '$name' FAILED (server log: $base/server.log)" >&2
    FAILED=1
  fi
  stop_server
}

# verify_nfsp exercises the debug reconcile/create_view doors → --debug-api.
if [ "$SUITE" = nfsp ] || [ "$SUITE" = all ]; then
  run_suite nfsp 3261 verify_nfsp.ts --debug-api
fi
if [ "$SUITE" = browser ] || [ "$SUITE" = all ]; then
  run_suite browser 3262 verify_browser_client.ts
fi
# verify_persistent seeds views through the debug door → --debug-api.
# Its work dir ($PERSIST_ROOT) is deliberately kept after the run.
if [ "$SUITE" = persistent ] || [ "$SUITE" = all ]; then
  run_suite persistent 3263 verify_persistent.ts --debug-api
  echo "==> persistent state kept at: $PERSIST_ROOT"
fi

if [ "$FAILED" -ne 0 ]; then
  echo "==> RESULT: FAILED"
  exit 1
fi
echo "==> RESULT: all suites passed"
