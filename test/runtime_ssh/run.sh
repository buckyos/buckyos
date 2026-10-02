#!/usr/bin/env bash
set -euo pipefail
repo_dir=$(cd -- "$(dirname -- "$0")/../.." && pwd)
fixture_dir=$(mktemp -d)
cleanup() {
  if [[ -f "$fixture_dir/sshd.pid" ]]; then kill "$(cat "$fixture_dir/sshd.pid")" 2>/dev/null || true; fi
  if [[ -f "$fixture_dir/sshd2.pid" ]]; then kill "$(cat "$fixture_dir/sshd2.pid")" 2>/dev/null || true; fi
  rm -rf -- "$fixture_dir"
}
trap cleanup EXIT
mkdir -p "$fixture_dir/bin" "$fixture_dir/work space 中文"
ssh-keygen -q -t ed25519 -N '' -f "$fixture_dir/host_key"
ssh-keygen -q -t ed25519 -N '' -f "$fixture_dir/client_key"
ssh-keygen -q -t ed25519 -N '' -f "$fixture_dir/denied_key"
fixture_port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')
fixture_user=$(id -un)
cat > "$fixture_dir/sshd_config" <<EOF
ListenAddress 127.0.0.1
Port $fixture_port
HostKey $fixture_dir/host_key
PidFile $fixture_dir/sshd.pid
AuthorizedKeysFile $fixture_dir/client_key.pub
StrictModes no
PasswordAuthentication no
KbdInteractiveAuthentication no
PermitRootLogin prohibit-password
UsePAM no
Subsystem sftp internal-sftp
AllowUsers $fixture_user
EOF
sshd_bin=$(command -v sshd)
"$sshd_bin" -f "$fixture_dir/sshd_config" -E "$fixture_dir/sshd.log"
redirect_port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')
sed -e "s/Port $fixture_port/Port $redirect_port/" -e "s|sshd.pid|sshd2.pid|" "$fixture_dir/sshd_config" > "$fixture_dir/sshd2_config"
"$sshd_bin" -f "$fixture_dir/sshd2_config" -E "$fixture_dir/sshd2.log"
printf '[127.0.0.1]:%s %s\n' "$fixture_port" "$(cat "$fixture_dir/host_key.pub")" > "$fixture_dir/known_hosts"
printf '[127.0.0.1]:%s %s\n' "$redirect_port" "$(cat "$fixture_dir/host_key.pub")" >> "$fixture_dir/known_hosts"
cat > "$fixture_dir/client_config" <<EOF
Host runtime-test
  HostName 127.0.0.1
  Port $fixture_port
  User $fixture_user
  IdentityFile $fixture_dir/client_key
  IdentitiesOnly yes
  UserKnownHostsFile $fixture_dir/known_hosts
  StrictHostKeyChecking yes
EOF
cat >> "$fixture_dir/client_config" <<EOF
Host runtime-denied
  HostName 127.0.0.1
  Port $fixture_port
  User $fixture_user
  IdentityFile $fixture_dir/denied_key
  IdentitiesOnly yes
  UserKnownHostsFile $fixture_dir/known_hosts
  StrictHostKeyChecking yes
EOF
for binary in ssh sftp; do
  binary_path=$(command -v "$binary")
  printf '#!/bin/sh\nexec %q -F %q "$@"\n' "$binary_path" "$fixture_dir/client_config" > "$fixture_dir/bin/$binary"
  chmod 700 "$fixture_dir/bin/$binary"
done
export PATH="$fixture_dir/bin:$PATH"
export LLM_RUNTIME_SSH_HOST=runtime-test
export LLM_RUNTIME_SSH_WORKDIR="$fixture_dir/work space 中文"
export LLM_RUNTIME_SSH_PID_FILE="$fixture_dir/sshd.pid"
export LLM_RUNTIME_SSH_SERVER="$sshd_bin"
export LLM_RUNTIME_SSH_SERVER_CONFIG="$fixture_dir/sshd_config"
export LLM_RUNTIME_SSH_CLIENT_CONFIG="$fixture_dir/client_config"
export LLM_RUNTIME_SSH_REDIRECT_PORT="$redirect_port"
cargo test --manifest-path "$repo_dir/src/Cargo.toml" -p agent_tool --lib runtime::tests::ssh -- --ignored --test-threads=1 --nocapture
