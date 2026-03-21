#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  create_tmux_debug_session.sh <agent_tool_binary> [session_name] [agent_env_root]

Example:
  ./create_tmux_debug_session.sh /opt/buckyos/bin/opendan/agent_tool od-debug /tmp/od-agent-env

This creates a tmux session with:
  - PATH prefixed by a temp tool alias directory
  - aliases pointing to the single `agent_tool` binary
  - OPENDAN_* context variables similar to exec_bash
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

AGENT_TOOL_BIN="${1:-${OPENDAN_AGENT_TOOL:-}}"
SESSION_NAME="${2:-od-agent-tool-debug}"
AGENT_ENV_ROOT="${3:-${OPENDAN_AGENT_ENV:-$(mktemp -d /tmp/opendan-agent-env.XXXXXX)}}"

if [[ -z "${AGENT_TOOL_BIN}" ]]; then
  echo "missing agent_tool binary path" >&2
  usage >&2
  exit 2
fi

if [[ ! -x "${AGENT_TOOL_BIN}" ]]; then
  echo "agent_tool binary is not executable: ${AGENT_TOOL_BIN}" >&2
  exit 2
fi

mkdir -p "${AGENT_ENV_ROOT}"
TOOL_DIR="${AGENT_ENV_ROOT}/debug-tools"
mkdir -p "${TOOL_DIR}"

for tool_name in \
  agent_tool \
  read_file \
  write_file \
  edit_file \
  get_session \
  todo \
  create_workspace \
  bind_workspace \
  check_task \
  cancel_task
do
  ln -sfn "${AGENT_TOOL_BIN}" "${TOOL_DIR}/${tool_name}"
done

export_cmds=(
  "export PATH='${TOOL_DIR}:\$PATH'"
  "export OPENDAN_AGENT_TOOL='${AGENT_TOOL_BIN}'"
  "export OPENDAN_AGENT_ENV='${AGENT_ENV_ROOT}'"
  "export OPENDAN_AGENT_ID='did:opendan:debug'"
  "export OPENDAN_BEHAVIOR='debug'"
  "export OPENDAN_STEP_IDX='0'"
  "export OPENDAN_WAKEUP_ID='debug-wakeup'"
  "export OPENDAN_SESSION_ID='debug-session'"
  "export OPENDAN_TRACE_ID='debug-trace'"
  "cd '${PWD}'"
  "clear"
  "printf 'agent_tool=%s\nagent_env=%s\ntool_dir=%s\n' '${AGENT_TOOL_BIN}' '${AGENT_ENV_ROOT}' '${TOOL_DIR}'"
)

if tmux has-session -t "${SESSION_NAME}" 2>/dev/null; then
  tmux attach-session -t "${SESSION_NAME}"
  exit 0
fi

tmux new-session -d -s "${SESSION_NAME}"
for cmd in "${export_cmds[@]}"; do
  tmux send-keys -t "${SESSION_NAME}:0.0" "${cmd}" C-m
done
tmux attach-session -t "${SESSION_NAME}"
