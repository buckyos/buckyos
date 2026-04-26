#!/bin/sh

set +e

export PATH="/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:${PATH:-}"

kill_process() {
  name="$1"
  if killall "$name" >/dev/null 2>&1; then
    echo "$name killed"
  else
    echo "$name not running"
  fi
}

remove_buckyos_containers() {
  if ! command -v docker >/dev/null 2>&1; then
    echo "docker not found"
    return
  fi

  container_ids="$(docker ps -aq --filter "label=buckyos.full_appid" 2>/dev/null || true)"
  if [ -z "$container_ids" ]; then
    echo "No buckyos docker containers found"
    return
  fi

  echo "$container_ids" | while IFS= read -r container_id; do
    if [ -n "$container_id" ]; then
      if docker rm -f "$container_id" >/dev/null 2>&1; then
        echo "$container_id container removed"
      else
        echo "Failed to remove $container_id"
      fi
    fi
  done
}

stop_all() {
  kill_process "node-daemon"
  kill_process "node_daemon"
  kill_process "scheduler"
  kill_process "verify-hub"
  kill_process "verify_hub"
  kill_process "system-config"
  kill_process "system_config"
  kill_process "cyfs-gateway"
  kill_process "cyfs_gateway"
  kill_process "filebrowser"
  kill_process "smb-service"
  kill_process "smb_service"
  kill_process "repo-service"
  kill_process "repo_service"
  kill_process "control-panel"
  kill_process "control_panel"
  kill_process "aicc"
  kill_process "task_manager"
  kill_process "kmsg"
  kill_process "msg_center"
  kill_process "opendan"
  remove_buckyos_containers
}

stop_all
