#!/usr/bin/env bash
# Regenerate the Rust run-directory fixtures of xllm (xllm TS runtime TODO
# Q9 (e)): real `agent_tool xllm` runs against a scripted OpenAI mock
# (mock_openai.mjs), kept as they are so the TS runtime's takeover tests run
# without a Rust binary.
#
#   paused_transient/  one `write_file`, then the model fails with HTTP 503
#                      (Transient): status `paused`, notes.txt in work/
#   shell_sigkill/     SIGKILLed while `shell` runs (its process group too, so
#                      no `exit` was written): the latest snapshot is the
#                      `before_tool_call` checkpoint of that call, no result;
#                      expected.json holds what Rust gives the model for it,
#                      captured by resuming copies with Rust: `no_exit` is the
#                      `describe_interrupted` text (Unresolved); `exit_0` (an
#                      `exit` file added as if the command ended after the
#                      crash) is Rust's exec-dir readback of task
#                      `local:shell:<call_id>`, which a host without a task
#                      resolver (the Deno host) does not do
#
# Each fixture: fixture.json, runs/<run_id>/{run.json,snapshots,exec}, work/.
# Paths inside are under fixture.json `root`; the websdk copy
# (tests/fixtures/xllm/runs/) has rewrite_paths.ts to move a fixture to
# another root / uid / mock port. `AGENT_TOOL_HOST_ID=xllm-fixture` fixes the
# runtime's `host`.
#
# Usage (from anywhere): bash generate.sh   (XLLM_RUST_BIN=… to pick the
# binary; default src/target/debug/agent_tool from `cargo build -p
# agent_tool_cli_dev`). Then copy paused_transient/ and shell_sigkill/ to
# buckyos-websdk/tests/fixtures/xllm/runs/.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
SRC=$(cd "$HERE/../../../../.." && pwd)
BIN=${XLLM_RUST_BIN:-}
if [ -z "$BIN" ]; then
  for c in "$SRC/target/debug/agent_tool" /tmp/dev-cache-root/cargo-target/debug/agent_tool; do
    if [ -x "$c" ]; then BIN=$c; break; fi
  done
fi
[ -n "$BIN" ] && [ -x "$BIN" ] || { echo "agent_tool binary not found (cargo build -p agent_tool_cli_dev, or set XLLM_RUST_BIN)" >&2; exit 2; }

ROOT=/tmp/xllm-fixture-root
OFFSET_SECS=7200
CALL_SHELL=call_shell_1
MARKER=xllm-fixture-shell
export AGENT_TOOL_HOST_ID=xllm-fixture
export XLLM_FIXTURE_KEY=fixture-key

rm -rf "$ROOT"
mkdir -p "$ROOT"
cat > "$ROOT/script.json" <<EOF
{
  "paused": [
    { "tool_calls": [ { "id": "call_write_1", "name": "write_file", "arguments": { "path": "notes.txt", "content": "written by rust\n" } } ] },
    { "status": 503, "body": { "error": { "message": "mock overloaded", "type": "server_error" } } }
  ],
  "shell": [
    { "tool_calls": [ { "id": "$CALL_SHELL", "name": "shell", "arguments": { "command": "printf 'started\\\\n'; sleep 60; printf 'finished\\\\n' # $MARKER" } } ] }
  ],
  "capture_no_exit": [ { "content": "done" } ],
  "capture_exit_0": [ { "content": "done" } ]
}
EOF

node "$HERE/mock_openai.mjs" --script "$ROOT/script.json" --log "$ROOT/requests.jsonl" > "$ROOT/mock.out" 2>&1 &
MOCK=$!
trap 'kill $MOCK 2>/dev/null || true' EXIT
for _ in $(seq 50); do grep -q listening "$ROOT/mock.out" 2>/dev/null && break; sleep 0.1; done
PORT=$(awk '/listening/ {print $2}' "$ROOT/mock.out")
[ -n "$PORT" ] || { cat "$ROOT/mock.out" >&2; exit 1; }

# One fixture's workdir, runs dir and .llm_context (tools on; a wait budget
# not below the shell timeout, so `shell` waits in the call and the bash group
# has no task tools — the shape a host without a task backend can take over).
setup() {
  local dir=$ROOT/$1 scenario=$2
  mkdir -p "$dir/work" "$dir/runs"
  cat > "$dir/work/.llm_context" <<EOF
provider:
  type: openai
  base_url: http://127.0.0.1:$PORT/s/$scenario/v1
  api_key_env: XLLM_FIXTURE_KEY
model: mock-model
runtime:
  task_wait_ms: 1800000
tools:
  enabled: true
EOF
}

only_run_id() {
  local ids
  ids=$(ls "$1")
  [ "$(echo "$ids" | wc -l)" = 1 ] || { echo "expected one run in $1, got: $ids" >&2; exit 1; }
  echo "$ids"
}

# --- paused_transient --------------------------------------------------------
setup paused paused
set +e
"$BIN" xllm --dir "$ROOT/paused/work" --runs-dir "$ROOT/paused/runs" --run-logs warn \
  "Write notes.txt with one line, then finish." </dev/null
code=$?
set -e
[ "$code" = 3 ] || { echo "paused run: expected exit 3 (paused), got $code" >&2; exit 1; }
PAUSED_ID=$(only_run_id "$ROOT/paused/runs")

# --- shell_sigkill -----------------------------------------------------------
setup shell shell
"$BIN" xllm --dir "$ROOT/shell/work" --runs-dir "$ROOT/shell/runs" --run-logs warn \
  "Run the long command." </dev/null &
RUST=$!
for _ in $(seq 300); do
  if grep -qs started "$ROOT"/shell/runs/*/exec/$CALL_SHELL/stdout; then break; fi
  sleep 0.1
done
grep -qs started "$ROOT"/shell/runs/*/exec/$CALL_SHELL/stdout || { echo "shell never started" >&2; exit 1; }
kill -9 "$RUST"
wait "$RUST" 2>/dev/null || true
# The command's process group outlived the executor (kill -9): kill it as a
# whole, before it ends, so no `exit` is written.
PGID=$(ps -o pgid= -p "$(pgrep -o -f "$MARKER")" | tr -d ' ')
kill -9 -- "-$PGID"
sleep 0.3
SHELL_ID=$(only_run_id "$ROOT/shell/runs")
EXEC=$ROOT/shell/runs/$SHELL_ID/exec/$CALL_SHELL
[ ! -e "$EXEC/exit" ] || { echo "the shell command wrote exit" >&2; exit 1; }

# Rust's own `describe_interrupted` for the call: resume a copy (the `command`
# mtime set OFFSET_SECS back, as the TS tests do) and read the tool message
# the mock received. `exit_0` also writes the exit file the command would
# have written had it finished after the crash.
capture() {
  local name=$1 dir=$ROOT/$1
  cp -a "$ROOT/shell" "$dir"
  find "$dir" -type f \( -name '*.json' -o -name .llm_context \) -exec sed -i \
    -e "s#$ROOT/shell#$dir#g" -e "s#/s/shell/v1#/s/$name/v1#g" {} +
  local exec_dir=$dir/runs/$SHELL_ID/exec/$CALL_SHELL
  if [ "$name" = capture_exit_0 ]; then
    printf 'finished\n' >> "$exec_dir/stdout"
    printf '0\n' > "$exec_dir/exit"
  fi
  local secs=$(( $(date +%s) - OFFSET_SECS ))
  touch -d "@$secs" "$exec_dir/command"
  "$BIN" xllm --resume --run "$SHELL_ID" --runs-dir "$dir/runs" --run-logs warn </dev/null >/dev/null
  node -e '
    const [log, scenario, callId, execDir, ms] = process.argv.slice(1)
    const fs = require("fs")
    const req = fs.readFileSync(log, "utf8").trim().split("\n").map(JSON.parse).find(r => r.scenario === scenario && r.n === 1)
    const tool = req.body.messages.find(m => m.role === "tool" && m.tool_call_id === callId)
    const tpl = s => s.split(execDir).join("{{exec_dir}}").split(ms).join("{{command_mtime_ms}}")
    const prefix = "[unresolved: result unknown] "
    const out = { tool_message: tpl(tool.content) }
    if (tool.content.startsWith(prefix)) out.reason = tpl(tool.content.slice(prefix.length))
    process.stdout.write(JSON.stringify(out))
  ' "$ROOT/requests.jsonl" "$name" "$CALL_SHELL" "$exec_dir" "$((secs * 1000))" > "$ROOT/$name.json"
}
capture capture_no_exit
capture capture_exit_0

# --- publish -----------------------------------------------------------------
publish() {
  local name=$1 dir=$ROOT/$2 run_id=$3
  rm -rf "${HERE:?}/$name"
  mkdir -p "$HERE/$name"
  cp -a "$dir/runs" "$dir/work" "$HERE/$name/"
  find "$HERE/$name/runs" -name .lock -delete
  cat > "$HERE/$name/fixture.json" <<EOF
{
  "root": "$dir",
  "run_id": "$run_id",
  "uid": "$(id -u)",
  "host_id": "$AGENT_TOOL_HOST_ID",
  "base_url": "http://127.0.0.1:$PORT/s/$2/v1",
  "api_key_env": "XLLM_FIXTURE_KEY"
}
EOF
}
publish paused_transient paused "$PAUSED_ID"
publish shell_sigkill shell "$SHELL_ID"
node -e '
  const fs = require("fs")
  const [out, call, offset, a, b] = process.argv.slice(1)
  fs.writeFileSync(out, JSON.stringify({
    call_id: call,
    command_mtime_offset_secs: Number(offset),
    no_exit: JSON.parse(fs.readFileSync(a, "utf8")),
    exit_0: { exit_file: "0\n", stdout_append: "finished\n", ...JSON.parse(fs.readFileSync(b, "utf8")) },
  }, null, 2) + "\n")
' "$HERE/shell_sigkill/expected.json" "$CALL_SHELL" "$OFFSET_SECS" "$ROOT/capture_no_exit.json" "$ROOT/capture_exit_0.json"
echo "fixtures written: $HERE/paused_transient ($PAUSED_ID), $HERE/shell_sigkill ($SHELL_ID)"
