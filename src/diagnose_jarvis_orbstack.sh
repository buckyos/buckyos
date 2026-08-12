#!/usr/bin/env bash
set -u
set -o pipefail

SCRIPT_NAME="$(basename "$0")"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DEFAULT_BUCKYOS_ROOT="/opt/buckyos"
DEFAULT_AIOS_IMAGE_REPO="paios/aios"
OWNER_HINT=""
BUNDLE_DIR=""
SINCE_MINUTES="60"

usage() {
  cat <<EOF
Usage:
  $SCRIPT_NAME [--owner <owner_user_id>] [--bundle-dir <dir>] [--since-minutes <n>]

Examples:
  bash src/diagnose_jarvis_orbstack.sh
  bash src/diagnose_jarvis_orbstack.sh --owner alice
  BUCKYOS_ROOT=/opt/buckyos bash src/diagnose_jarvis_orbstack.sh --bundle-dir /tmp/jarvis-diag

What it collects:
  - macOS / OrbStack / Docker basic status
  - BuckyOS runtime paths, logs, and Jarvis data directories
  - recent node_daemon and agent log tails
  - Jarvis container/image state if still present
  - a foreground repro command rebuilt from the last node_daemon docker run log
EOF
}

log() {
  echo "[$SCRIPT_NAME] $*"
}

resolve_aios_image_repo() {
  local devenv_path="$PROJECT_ROOT/devenv.json"
  local python_bin=""

  if [[ ! -f "$devenv_path" ]]; then
    printf '%s\n' "$DEFAULT_AIOS_IMAGE_REPO"
    return
  fi

  if command -v python3 >/dev/null 2>&1; then
    python_bin="$(command -v python3)"
  elif command -v python >/dev/null 2>&1; then
    python_bin="$(command -v python)"
  fi

  if [[ -z "$python_bin" ]]; then
    printf '%s\n' "$DEFAULT_AIOS_IMAGE_REPO"
    return
  fi

  "$python_bin" - "$devenv_path" "$DEFAULT_AIOS_IMAGE_REPO" <<'PY'
import json
import sys

config_path = sys.argv[1]
default_image = sys.argv[2]

try:
    with open(config_path, "r", encoding="utf-8") as fh:
        config = json.load(fh)
except Exception:
    print(default_image)
    raise SystemExit(0)

image = config.get("aios")
if isinstance(image, str) and image.strip():
    print(image.strip())
else:
    print(default_image)
PY
}

append_line() {
  local file="$1"
  shift
  printf '%s\n' "$*" >>"$file"
}

capture_cmd() {
  local outfile="$1"
  shift
  {
    printf '$'
    local arg
    for arg in "$@"; do
      printf ' %q' "$arg"
    done
    printf '\n'
    "$@"
  } >"$outfile" 2>&1
}

append_cmd() {
  local outfile="$1"
  shift
  {
    printf '$'
    local arg
    for arg in "$@"; do
      printf ' %q' "$arg"
    done
    printf '\n'
    "$@"
    printf '\n'
  } >>"$outfile" 2>&1
}

resolve_buckyos_root() {
  if [[ -n "${BUCKYOS_ROOT:-}" && -d "${BUCKYOS_ROOT}" ]]; then
    printf '%s\n' "${BUCKYOS_ROOT}"
    return
  fi

  local proc_line proc_path proc_root
  proc_line="$(ps ax -o command= 2>/dev/null | grep -m 1 -E '/node[-_]daemon/node_daemon([[:space:]]|$)|/node-daemon/node_daemon([[:space:]]|$)' || true)"
  if [[ -n "$proc_line" ]]; then
    proc_path="$(printf '%s\n' "$proc_line" | awk '{print $1}')"
    if [[ -n "$proc_path" && -e "$proc_path" ]]; then
      proc_root="$(cd "$(dirname "$proc_path")/../.." 2>/dev/null && pwd || true)"
      if [[ -n "$proc_root" && -d "$proc_root" ]]; then
        printf '%s\n' "$proc_root"
        return
      fi
    fi
  fi

  printf '%s\n' "$DEFAULT_BUCKYOS_ROOT"
}

discover_owners() {
  local root="$1"
  local owner_file="$2"
  : >"$owner_file"

  if [[ -n "$OWNER_HINT" ]]; then
    printf '%s\n' "$OWNER_HINT" >"$owner_file"
    return
  fi

  local jarvis_dirs
  jarvis_dirs="$(find "$root/data/home" -type d -path '*/.local/share/jarvis' 2>/dev/null || true)"
  if [[ -n "$jarvis_dirs" ]]; then
    printf '%s\n' "$jarvis_dirs" | while IFS= read -r dir; do
      [[ -z "$dir" ]] && continue
      basename "$(dirname "$(dirname "$(dirname "$dir")")")"
    done | sort -u >"$owner_file"
    return
  fi

  find "$root/data/home" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | while IFS= read -r dir; do
    basename "$dir"
  done | sort -u >"$owner_file"
}

collect_recent_logs() {
  local log_root="$1"
  local output="$2"
  local name1="$3"
  local name2="$4"
  local limit="$5"
  : >"$output"

  if [[ ! -d "$log_root" ]]; then
    append_line "$output" "log root not found: $log_root"
    return
  fi

  local files
  files="$(find "$log_root" -type f \( -name "$name1" -o -name "$name2" \) -exec stat -f '%m %N' {} \; 2>/dev/null | sort -nr | head -n "$limit" | sed 's/^[0-9][0-9]* //')"
  if [[ -z "$files" ]]; then
    append_line "$output" "no matching logs under $log_root for $name1 / $name2"
    return
  fi

  printf '%s\n' "$files" | while IFS= read -r file; do
    [[ -z "$file" ]] && continue
    append_line "$output" "===== $file ====="
    tail -n 200 "$file" >>"$output" 2>&1 || true
    append_line "$output" ""
  done
}

extract_last_docker_run() {
  local log_root="$1"
  local out_file="$2"
  : >"$out_file"

  if [[ ! -d "$log_root" ]]; then
    return
  fi

  local line
  line="$(
    find "$log_root" -type f \( -name '*node_daemon*' -o -name '*node-daemon*' \) -print 2>/dev/null \
      | while IFS= read -r file; do
          grep -h 'executing docker run:' "$file" 2>/dev/null || true
        done \
      | grep 'jarvis' \
      | tail -n 1 \
      | sed 's/.*executing docker run: //'
  )"

  if [[ -n "$line" ]]; then
    sanitize_docker_run_line "$line" >"$out_file"
  fi
}

sanitize_docker_run_line() {
  local input="$1"
  printf '%s\n' "$input" | /usr/bin/perl -0pe "s/'-e'\\s+'([^'=]*(?:TOKEN|SECRET|PASSWORD|PRIVATE_KEY|SESSION|API_KEY|BUCKYOS_ZONE_CONFIG|AI_PROVIDER_CONFIG)[^'=]*)=[^']*'/'-e' '\$1=<REDACTED>'/g"
}

build_repro_script() {
  local last_run_file="$1"
  local output="$2"

  if [[ ! -s "$last_run_file" ]]; then
    return
  fi

  local run_line sanitized_line repro_line
  run_line="$(cat "$last_run_file")"
  sanitized_line="$(sanitize_docker_run_line "$run_line")"
  repro_line="$(printf '%s\n' "$sanitized_line" | /usr/bin/perl -pe "s/(?:'--rm'|--rm)\\s+//; s/(?:'-d'|-d)/'-it'/")"

  cat >"$output" <<EOF
#!/usr/bin/env bash
set -euo pipefail

${repro_line}
EOF
  chmod +x "$output"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --owner)
      OWNER_HINT="${2:-}"
      shift 2
      ;;
    --bundle-dir)
      BUNDLE_DIR="${2:-}"
      shift 2
      ;;
    --since-minutes)
      SINCE_MINUTES="${2:-}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if ! [[ "$SINCE_MINUTES" =~ ^[0-9]+$ ]]; then
  echo "--since-minutes must be an integer" >&2
  exit 2
fi

RUN_TS="$(date '+%Y%m%d-%H%M%S')"
if [[ -z "$BUNDLE_DIR" ]]; then
  BUNDLE_DIR="/tmp/jarvis-orbstack-diag-${RUN_TS}"
fi
mkdir -p "$BUNDLE_DIR"

SUMMARY_FILE="$BUNDLE_DIR/summary.txt"
SYSTEM_FILE="$BUNDLE_DIR/system.txt"
DOCKER_FILE="$BUNDLE_DIR/docker.txt"
CONTAINERS_FILE="$BUNDLE_DIR/jarvis_containers.txt"
BUCKYOS_FILE="$BUNDLE_DIR/buckyos.txt"
NODE_LOG_FILE="$BUNDLE_DIR/node_daemon_tail.txt"
AGENT_LOG_FILE="$BUNDLE_DIR/agent_logs_tail.txt"
PATTERN_FILE="$BUNDLE_DIR/recent_log_matches.txt"
LAST_RUN_FILE="$BUNDLE_DIR/last_jarvis_docker_run.txt"
REPRO_FILE="$BUNDLE_DIR/repro_jarvis_foreground.sh"
OWNERS_FILE="$BUNDLE_DIR/owners.txt"

BUCKYOS_ROOT_RESOLVED="$(resolve_buckyos_root)"
AIOS_IMAGE_REPO="$(resolve_aios_image_repo)"
LOG_ROOT="$BUCKYOS_ROOT_RESOLVED/logs"
AGENT_LOG_ROOT="$LOG_ROOT/agents"

discover_owners "$BUCKYOS_ROOT_RESOLVED" "$OWNERS_FILE"

{
  echo "jarvis + OrbStack diagnosis bundle"
  echo "generated_at: $(date -u '+%Y-%m-%dT%H:%M:%SZ')"
  echo "host_timezone: $(date '+%Z %z')"
  echo "buckyos_root: $BUCKYOS_ROOT_RESOLVED"
  echo "bundle_dir: $BUNDLE_DIR"
  echo "since_minutes: $SINCE_MINUTES"
  echo ""
} >"$SUMMARY_FILE"

append_line "$SUMMARY_FILE" "quick_findings:"

if [[ ! -d /Applications/OrbStack.app ]]; then
  append_line "$SUMMARY_FILE" "- OrbStack.app not found at /Applications/OrbStack.app"
else
  append_line "$SUMMARY_FILE" "- OrbStack.app exists"
fi

if command -v docker >/dev/null 2>&1; then
  append_line "$SUMMARY_FILE" "- docker CLI found at $(command -v docker)"
else
  append_line "$SUMMARY_FILE" "- docker CLI not found in PATH"
fi

if docker info >/dev/null 2>&1; then
  append_line "$SUMMARY_FILE" "- docker info works for current user"
else
  append_line "$SUMMARY_FILE" "- docker info failed for current user"
fi

if pgrep -f 'node_daemon|node-daemon' >/dev/null 2>&1; then
  append_line "$SUMMARY_FILE" "- node_daemon process is running"
else
  append_line "$SUMMARY_FILE" "- node_daemon process not found"
fi

if [[ -s "$OWNERS_FILE" ]]; then
  append_line "$SUMMARY_FILE" "- detected owners: $(paste -sd ', ' "$OWNERS_FILE")"
else
  append_line "$SUMMARY_FILE" "- no owner directories detected under $BUCKYOS_ROOT_RESOLVED/data/home"
fi

capture_cmd "$SYSTEM_FILE" /usr/bin/sw_vers
append_cmd "$SYSTEM_FILE" /usr/bin/uname -a
append_cmd "$SYSTEM_FILE" /usr/bin/arch
append_cmd "$SYSTEM_FILE" /usr/bin/id
append_cmd "$SYSTEM_FILE" /usr/bin/whoami
append_cmd "$SYSTEM_FILE" /bin/date
append_cmd "$SYSTEM_FILE" /usr/sbin/sysctl -n hw.optional.arm64
append_cmd "$SYSTEM_FILE" /usr/sbin/sysctl -n machdep.cpu.brand_string
append_cmd "$SYSTEM_FILE" /bin/ls -ld /Applications/OrbStack.app
append_cmd "$SYSTEM_FILE" /bin/ps ax -o pid,ppid,user,etime,command

capture_cmd "$DOCKER_FILE" /usr/bin/env
append_cmd "$DOCKER_FILE" /usr/bin/which docker
append_cmd "$DOCKER_FILE" docker --version
append_cmd "$DOCKER_FILE" docker context show
append_cmd "$DOCKER_FILE" docker context ls
append_cmd "$DOCKER_FILE" docker version
append_cmd "$DOCKER_FILE" docker info
append_cmd "$DOCKER_FILE" docker ps -a --no-trunc
append_cmd "$DOCKER_FILE" docker images --digests
append_cmd "$DOCKER_FILE" /bin/ls -l /var/run/docker.sock
append_cmd "$DOCKER_FILE" /usr/bin/stat -f '%N %z bytes %Sp %Su:%Sg' /var/run/docker.sock
append_cmd "$DOCKER_FILE" /bin/ps ax -o pid,ppid,user,etime,command | /usr/bin/grep -E 'OrbStack|node_daemon|node-daemon|docker|containerd'

{
  echo "buckyos_root: $BUCKYOS_ROOT_RESOLVED"
  echo ""
  printf '$ %q %q\n' /bin/ls -ld "$BUCKYOS_ROOT_RESOLVED"
  /bin/ls -ld "$BUCKYOS_ROOT_RESOLVED" 2>&1 || true
  echo ""
  printf '$ %q %q\n' /bin/ls -ld "$BUCKYOS_ROOT_RESOLVED/data"
  /bin/ls -ld "$BUCKYOS_ROOT_RESOLVED/data" 2>&1 || true
  echo ""
  printf '$ %q %q\n' /bin/ls -ld "$BUCKYOS_ROOT_RESOLVED/logs"
  /bin/ls -ld "$BUCKYOS_ROOT_RESOLVED/logs" 2>&1 || true
  echo ""
  printf '$ %q %q\n' /bin/ls -ld "$BUCKYOS_ROOT_RESOLVED/storage"
  /bin/ls -ld "$BUCKYOS_ROOT_RESOLVED/storage" 2>&1 || true
  echo ""
  printf '$ %q %q\n' /usr/bin/find "$BUCKYOS_ROOT_RESOLVED/data/home" -maxdepth 4 -type d
  /usr/bin/find "$BUCKYOS_ROOT_RESOLVED/data/home" -maxdepth 4 -type d 2>&1 || true
  echo ""
} >"$BUCKYOS_FILE"

while IFS= read -r owner; do
  [[ -z "$owner" ]] && continue
  owner_safe="$(printf '%s' "$owner" | tr '/' '_')"
  app_dir="$BUCKYOS_ROOT_RESOLVED/data/home/$owner/.local/share/jarvis"
  log_dir="$AGENT_LOG_ROOT/${owner}-jarvis"
  {
    echo "===== owner: $owner ====="
    printf '$ %q %q\n' /bin/ls -ld "$app_dir"
    /bin/ls -ld "$app_dir" 2>&1 || true
    printf '$ %q %q\n' /usr/bin/find "$app_dir" -maxdepth 3 -type f
    /usr/bin/find "$app_dir" -maxdepth 3 -type f 2>&1 || true
    printf '$ %q %q\n' /bin/ls -ld "$log_dir"
    /bin/ls -ld "$log_dir" 2>&1 || true
    printf '$ %q %q\n' /usr/bin/find "$log_dir" -maxdepth 2 -type f
    /usr/bin/find "$log_dir" -maxdepth 2 -type f 2>&1 || true
    echo ""
  } >>"$BUCKYOS_FILE"

  owner_container_file="$BUNDLE_DIR/container_${owner_safe}.txt"
  capture_cmd "$owner_container_file" docker ps -a --no-trunc --filter "name=^${owner}-jarvis$"
  append_cmd "$owner_container_file" docker container inspect "${owner}-jarvis"
  append_cmd "$owner_container_file" docker logs --tail 200 "${owner}-jarvis"

  owner_label_file="$BUNDLE_DIR/container_label_${owner_safe}.txt"
  capture_cmd "$owner_label_file" docker ps -a --no-trunc --filter "label=buckyos.app_id=jarvis"
  append_cmd "$owner_label_file" docker ps -a --no-trunc --filter "label=buckyos.owner_user_id=${owner}"
done <"$OWNERS_FILE"

capture_cmd "$CONTAINERS_FILE" docker ps -a --no-trunc --filter "label=buckyos.app_id=jarvis"
append_cmd "$CONTAINERS_FILE" docker ps --no-trunc --filter "label=buckyos.app_id=jarvis"
append_cmd "$CONTAINERS_FILE" docker image inspect "${AIOS_IMAGE_REPO}:latest-aarch64"
append_cmd "$CONTAINERS_FILE" docker image inspect "${AIOS_IMAGE_REPO}:latest-amd64"

collect_recent_logs "$LOG_ROOT" "$NODE_LOG_FILE" '*node_daemon*' '*node-daemon*' 5
collect_recent_logs "$AGENT_LOG_ROOT" "$AGENT_LOG_FILE" '*jarvis*' '*.log' 8

{
  echo "patterns:"
  echo "jarvis|docker run|docker rm|overlay|fuse|permission denied|Mounts denied|No such file|exec format error|panic|fatal|error"
  echo ""
  if [[ -d "$LOG_ROOT" ]]; then
    /usr/bin/grep -RInE 'jarvis|docker run|docker rm|overlay|fuse|permission denied|Mounts denied|No such file|exec format error|panic|fatal|error' "$LOG_ROOT" 2>/dev/null | /usr/bin/tail -n 400
  else
    echo "log root not found: $LOG_ROOT"
  fi
} >"$PATTERN_FILE"

extract_last_docker_run "$LOG_ROOT" "$LAST_RUN_FILE"
build_repro_script "$LAST_RUN_FILE" "$REPRO_FILE"

if [[ -s "$LAST_RUN_FILE" ]]; then
  append_line "$SUMMARY_FILE" "- extracted last jarvis docker run command to $(basename "$LAST_RUN_FILE")"
  append_line "$SUMMARY_FILE" "- generated foreground repro script $(basename "$REPRO_FILE")"
else
  append_line "$SUMMARY_FILE" "- could not find a recent jarvis docker run line in node_daemon logs"
fi

ARCHIVE_PATH="${BUNDLE_DIR}.tar.gz"
if tar -czf "$ARCHIVE_PATH" -C "$(dirname "$BUNDLE_DIR")" "$(basename "$BUNDLE_DIR")" >/dev/null 2>&1; then
  append_line "$SUMMARY_FILE" "- packed bundle: $ARCHIVE_PATH"
fi

log "bundle ready: $BUNDLE_DIR"
if [[ -f "$ARCHIVE_PATH" ]]; then
  log "archive ready: $ARCHIVE_PATH"
fi
log "send summary.txt plus the tar.gz bundle back for analysis"
