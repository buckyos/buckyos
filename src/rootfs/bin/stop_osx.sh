#!/bin/sh

# Stop this installation of the BuckyOS Desktop Runtime: the bound user's
# LaunchAgent, that user's processes whose executable lives under this root,
# and the BuckyOS app containers in that user's Docker context. Processes are
# never matched by a machine-wide name. Usable as root (installer) or as the
# bound user.

set +e

export PATH="/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:${PATH:-}"

ROOT="${BUCKYOS_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
REAL_ROOT="$(cd "$ROOT" 2>/dev/null && pwd -P || printf '%s\n' "$ROOT")"
LABEL="com.buckyos.runtime"
USER_NAME=""
if [ -f "$ROOT/etc/desktop_runtime.json" ]; then
  USER_NAME="$(/usr/bin/plutil -extract user raw -o - "$ROOT/etc/desktop_runtime.json" 2>/dev/null)" || USER_NAME=""
fi
[ -n "$USER_NAME" ] || USER_NAME="$(id -un)"
USER_ID="$(id -u "$USER_NAME" 2>/dev/null)"
if [ -z "$USER_ID" ]; then
  echo "unknown runtime user: $USER_NAME"
  exit 1
fi

as_user() {
  if [ "$(id -u)" = "$USER_ID" ]; then
    "$@"
  else
    /usr/bin/sudo -u "$USER_NAME" -H "$@"
  fi
}

instance_pids() {
  ps -axo pid=,uid=,comm= | while read -r pid uid comm; do
    [ "$uid" = "$USER_ID" ] || continue
    [ "$pid" = "$$" ] && continue
    case "$comm" in
      "$ROOT"/*|"$REAL_ROOT"/*) echo "$pid" ;;
    esac
  done
}

stop_launch_agent() {
  if launchctl print "gui/$USER_ID/$LABEL" >/dev/null 2>&1; then
    if launchctl bootout "gui/$USER_ID/$LABEL" >/dev/null 2>&1; then
      echo "$LABEL booted out of gui/$USER_ID"
    else
      echo "failed to boot out $LABEL from gui/$USER_ID"
    fi
  else
    echo "$LABEL not loaded in gui/$USER_ID"
  fi
}

stop_instance_processes() {
  pids="$(instance_pids)"
  if [ -z "$pids" ]; then
    echo "no processes of $USER_NAME running from $ROOT"
    return
  fi
  for pid in $pids; do
    kill "$pid" >/dev/null 2>&1 && echo "stopped pid $pid"
  done
  attempt=0
  while [ "$attempt" -lt 10 ] && [ -n "$(instance_pids)" ]; do
    sleep 1
    attempt=$((attempt + 1))
  done
  for pid in $(instance_pids); do
    kill -9 "$pid" >/dev/null 2>&1 && echo "killed pid $pid"
  done
}

remove_buckyos_containers() {
  if ! as_user /bin/sh -c 'command -v docker' >/dev/null 2>&1; then
    echo "docker not found for $USER_NAME"
    return
  fi

  container_ids="$(as_user docker ps -aq --filter "label=buckyos.app_instance_id" 2>/dev/null || true)"
  if [ -z "$container_ids" ]; then
    echo "No buckyos docker containers found"
    return
  fi

  echo "$container_ids" | while IFS= read -r container_id; do
    if [ -n "$container_id" ]; then
      if as_user docker rm -f "$container_id" >/dev/null 2>&1; then
        echo "$container_id container removed"
      else
        echo "Failed to remove $container_id"
      fi
    fi
  done
}

stop_launch_agent
stop_instance_processes
remove_buckyos_containers
