#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  create_tmux_debug_session.sh <agent_tool_binary> [session_name] [agent_root]

Example:
  ./create_tmux_debug_session.sh /opt/buckyos/bin/opendan/agent_tool od-debug /tmp/od-agent-root

This creates a tmux session with:
  - PATH prefixed by a temp tool alias directory
  - aliases pointing to the single `agent_tool` binary
  - the minimal AgentTool environment contract
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

AGENT_TOOL_BIN="${1:-}"
SESSION_NAME="${2:-od-agent-tool-debug}"
AGENT_ROOT="${3:-${OPENDAN_AGENT_ROOT:-$(mktemp -d /tmp/opendan-agent-root.XXXXXX)}}"

if [[ -z "${AGENT_TOOL_BIN}" ]]; then
  echo "missing agent_tool binary path" >&2
  usage >&2
  exit 2
fi

if [[ ! -x "${AGENT_TOOL_BIN}" ]]; then
  echo "agent_tool binary is not executable: ${AGENT_TOOL_BIN}" >&2
  exit 2
fi

mkdir -p "${AGENT_ROOT}"
if [[ ! -f "${AGENT_ROOT}/agent.toml" ]]; then
  cat >"${AGENT_ROOT}/agent.toml" <<'EOF'
[identity]
owner_user_id = "debug"
agent_id = "did:opendan:debug"
EOF
fi
TOOL_DIR="${AGENT_ROOT}/debug-tools"
mkdir -p "${TOOL_DIR}"

for tool_name in \
  agent_tool \
  read_file \
  write_file \
  edit_file \
  get_session \
  set_memory \
  remove_memory \
  todo \
  create_workspace \
  bind_workspace \
  check_task \
  cancel_task \
  finish_task
do
  ln -sfn "${AGENT_TOOL_BIN}" "${TOOL_DIR}/${tool_name}"
done

export_cmds=(
  "export PATH='${TOOL_DIR}:\$PATH'"
  "export OPENDAN_AGENT_ROOT='${AGENT_ROOT}'"
  "export OPENDAN_SESSION_ID='debug-session'"
  "export OPENDAN_TRACE_ID='debug-trace'"
  "export BUCKYOS_APPCLIENT_SESSION_TOKEN='${BUCKYOS_APPCLIENT_SESSION_TOKEN:-debug-token}'"
  "cd '${PWD}'"
  "clear"
  "printf 'agent_tool=%s\nagent_root=%s\ntool_dir=%s\n' '${AGENT_TOOL_BIN}' '${AGENT_ROOT}' '${TOOL_DIR}'"
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
