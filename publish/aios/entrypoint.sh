#!/usr/bin/env bash
# paios/aios Worker Image entrypoint
#
# This is the single, image-baked entrypoint for all BuckyOS apps and OpenDAN
# agents that use the paios/aios worker image. It performs three jobs:
#
#   1. Bootstrap the instance volume layout (pkg working copy, dep caches,
#      sync metadata) inside /opt/buckyos/instance and expose the writable app
#      package at /opt/buckyos/bin/$APP_ID.
#   2. File-level sync from the read-only upstream package mount
#      ($BUCKYOS_PKG_SOURCE_DIR, typically /mnt/buckyos/pkg) into the instance
#      volume's working copy ($BUCKYOS_PKG_DIR). Honours the R-15 policy:
#         - missing locally                -> copy from upstream
#         - present locally, matches meta  -> overwrite from upstream if changed
#         - present locally, diverged      -> keep local copy
#   3. Dispatch to the real program based on $BUCKYOS_APP_TYPE:
#         agent  -> /opt/buckyos/bin/opendan/opendan ...
#         script -> Deno (for TypeScript) or uv/python (for Python)
#         custom -> exec the command provided by the caller ("$@")

set -euo pipefail

log() {
  printf '[paios/aios] %s\n' "$*"
}

die() {
  printf '[paios/aios][FATAL] %s\n' "$*" >&2
  exit 1
}

: "${BUCKYOS_ROOT:=/opt/buckyos}"
: "${BUCKYOS_APP_ID:=unknown}"
: "${BUCKYOS_APP_TYPE:=}"
: "${BUCKYOS_PKG_SOURCE_DIR:=/mnt/buckyos/pkg}"
: "${BUCKYOS_INSTANCE_VOLUME:=${BUCKYOS_ROOT}/instance}"
: "${BUCKYOS_PKG_DIR:=${BUCKYOS_ROOT}/bin/${BUCKYOS_APP_ID}}"
if [[ "$BUCKYOS_APP_TYPE" == "agent" ]]; then
  : "${BUCKYOS_DATA_DIR:=${BUCKYOS_ROOT}/data/home/${BUCKYOS_OWNER_USER_ID:-default}/agents/${BUCKYOS_APP_ID}}"
else
  : "${BUCKYOS_DATA_DIR:=${BUCKYOS_ROOT}/data/home/default/.local/share/${BUCKYOS_APP_ID}}"
fi
: "${BUCKYOS_EXTTOOL_DIR:=${BUCKYOS_ROOT}/tools}"
: "${BUCKYOS_SAFE_MODE:=0}"

INSTANCE_PKG_DIR="${BUCKYOS_INSTANCE_VOLUME}/pkg"
DENO_DIR_DEFAULT="${BUCKYOS_INSTANCE_VOLUME}/deno-cache"
UV_CACHE_DEFAULT="${BUCKYOS_INSTANCE_VOLUME}/uv-cache"
NPM_CACHE_DEFAULT="${BUCKYOS_INSTANCE_VOLUME}/npm-cache"
PIP_CACHE_DEFAULT="${BUCKYOS_INSTANCE_VOLUME}/pip-cache"
SYNC_META_DIR="${BUCKYOS_INSTANCE_VOLUME}/.sync"
SYNC_META_FILE="${SYNC_META_DIR}/upstream.json"
SYNC_LOG_FILE="${SYNC_META_DIR}/last-sync.log"
AGENT_SYNC_META_DIR="${BUCKYOS_DATA_DIR}/.sync"
AGENT_SYNC_META_FILE="${AGENT_SYNC_META_DIR}/upstream.json"
AGENT_SYNC_LOG_FILE="${AGENT_SYNC_META_DIR}/last-sync.log"
EXTTOOL_SEED_MARK="${SYNC_META_DIR}/exttool-seeded"
# What the entrypoint itself runs on: the image's Deno, never an extension package's.
CORE_DENO=/opt/buckyos/bin/opendan/deno

export DENO_DIR="${DENO_DIR:-$DENO_DIR_DEFAULT}"
export UV_CACHE_DIR="${UV_CACHE_DIR:-$UV_CACHE_DEFAULT}"
export NPM_CONFIG_CACHE="${NPM_CONFIG_CACHE:-$NPM_CACHE_DEFAULT}"
export PIP_CACHE_DIR="${PIP_CACHE_DIR:-$PIP_CACHE_DEFAULT}"
export UV_PYTHON_INSTALL_DIR="${UV_PYTHON_INSTALL_DIR:-${BUCKYOS_INSTANCE_VOLUME}/uv-python}"
export NPM_CONFIG_PREFIX="${NPM_CONFIG_PREFIX:-${BUCKYOS_INSTANCE_VOLUME}/npm-prefix}"

# Extension packages come before the instance/system PATH. Always named: the
# node's reconciler may fill the volume after this container started.
if [[ ":$PATH:" != *":${BUCKYOS_EXTTOOL_DIR}/bin:"* ]]; then
  export PATH="${BUCKYOS_EXTTOOL_DIR}/bin:${PATH}"
fi

ensure_pkg_dir_alias() {
  local pkg_parent current_target
  pkg_parent="$(dirname "$BUCKYOS_PKG_DIR")"
  mkdir -p "$pkg_parent"

  if [[ -L "$BUCKYOS_PKG_DIR" ]]; then
    current_target="$(readlink "$BUCKYOS_PKG_DIR" 2>/dev/null || true)"
    if [[ "$current_target" == "$INSTANCE_PKG_DIR" ]]; then
      return
    fi
    rm -f "$BUCKYOS_PKG_DIR"
  elif [[ -e "$BUCKYOS_PKG_DIR" ]]; then
    die "pkg dir path already exists and is not the expected instance-volume alias: $BUCKYOS_PKG_DIR"
  fi

  ln -s "$INSTANCE_PKG_DIR" "$BUCKYOS_PKG_DIR"
}

mkdir -p \
  "$BUCKYOS_INSTANCE_VOLUME" \
  "$INSTANCE_PKG_DIR" \
  "$DENO_DIR" \
  "$UV_CACHE_DIR" \
  "$NPM_CONFIG_CACHE" \
  "$PIP_CACHE_DIR" \
  "$SYNC_META_DIR" \
  "$BUCKYOS_DATA_DIR"

ensure_pkg_dir_alias

# Copy-on-first-use seed from the shared ExtTool caches into the per-instance
# caches. Cheap enough on first start, and cheap enough to repeat on upgrade:
# cp -rn never overwrites locally-added entries, so apps keep their deltas.
# Once OverlayFS lower/upper is wired in (design §9), this block becomes a
# no-op and can be deleted.
seed_from_exttool() {
  local src="$1" dst="$2"
  if [[ -d "$src" ]] && [[ -n "$(ls -A "$src" 2>/dev/null)" ]]; then
    cp -rn "$src"/. "$dst"/ 2>/dev/null || true
  fi
}

if [[ ! -f "$EXTTOOL_SEED_MARK" ]]; then
  seed_from_exttool "${BUCKYOS_EXTTOOL_DIR}/uv-cache" "$UV_CACHE_DIR"
  seed_from_exttool "${BUCKYOS_EXTTOOL_DIR}/deno-cache" "$DENO_DIR"
  touch "$EXTTOOL_SEED_MARK"
fi

if [[ "$BUCKYOS_SAFE_MODE" == "1" ]]; then
  log "SAFE_MODE=1: resetting working copy and sync metadata for ${BUCKYOS_APP_ID}"
  rm -rf "$INSTANCE_PKG_DIR" "$SYNC_META_DIR"
  mkdir -p "$INSTANCE_PKG_DIR" "$SYNC_META_DIR"
  ensure_pkg_dir_alias
fi

# File-level sync is the replacement for the old overlayfs/fuse-overlayfs model.
# It runs entirely inside the container and behaves identically on Linux,
# macOS and Windows (Docker Desktop). See doc/app_service/paios容器需求.md §7.4.
sync_tree_with_upstream_policy() {
  local src="$1"
  local dst="$2"
  local meta_path="$3"
  local log_path="$4"
  local label="$5"

  if [[ ! -d "$src" ]]; then
    log "no ${label} source at $src (skipping sync)"
    return
  fi

  log "syncing ${label} $src -> $dst"
  "$CORE_DENO" run --no-config --no-prompt --allow-read --allow-write /opt/buckyos/sync_tree.ts "$src" "$dst" "$meta_path" "$log_path"
}

sync_upstream_into_instance() {
  sync_tree_with_upstream_policy \
    "$BUCKYOS_PKG_SOURCE_DIR" \
    "$INSTANCE_PKG_DIR" \
    "$SYNC_META_FILE" \
    "$SYNC_LOG_FILE" \
    "upstream pkg"
}

sync_agent_upstream_into_data_root() {
  local source_root="$BUCKYOS_PKG_SOURCE_DIR"
  if [[ ! -d "$source_root" ]]; then
    source_root="$BUCKYOS_PKG_DIR"
  fi

  sync_tree_with_upstream_policy \
    "$source_root" \
    "$BUCKYOS_DATA_DIR" \
    "$AGENT_SYNC_META_FILE" \
    "$AGENT_SYNC_LOG_FILE" \
    "agent root"
}

sync_upstream_into_instance

# ---------------------------------------------------------------------------
# Script runner (replaces the old script-service image)
# ---------------------------------------------------------------------------

script_detect_language() {
  local root="$1"
  if [[ -f "$root/deno.json" ]] || [[ -f "$root/deno.jsonc" ]]; then
    echo typescript; return
  fi
  if [[ -f "$root/pyproject.toml" ]] || [[ -f "$root/requirements.txt" ]]; then
    echo python; return
  fi
  for name in main start index; do
    for ext in ts tsx; do
      [[ -f "$root/$name.$ext" ]] && { echo typescript; return; }
    done
  done
  for name in main start __main__; do
    [[ -f "$root/$name.py" ]] && { echo python; return; }
  done
  echo unknown
}

script_find_entry() {
  local root="$1" lang="$2"
  if [[ -f "$root/buckyos_script.json" ]]; then
    local entry
    entry="$("$CORE_DENO" eval --no-config 'console.log(JSON.parse(Deno.readTextFileSync(Deno.args[0])).entry ?? "")' "$root/buckyos_script.json" 2>/dev/null || true)"
    if [[ -n "$entry" && -f "$root/$entry" ]]; then
      echo "$root/$entry"; return
    fi
  fi
  case "$lang" in
    typescript)
      for c in main.ts start.ts index.ts main.tsx start.tsx index.tsx; do
        [[ -f "$root/$c" ]] && { echo "$root/$c"; return; }
      done
      ;;
    python)
      for c in main.py start.py __main__.py; do
        [[ -f "$root/$c" ]] && { echo "$root/$c"; return; }
      done
      ;;
  esac
  echo ""
}

run_script_app() {
  local root="$BUCKYOS_PKG_DIR"
  local lang entry
  lang="$(script_detect_language "$root")"
  entry="$(script_find_entry "$root" "$lang")"

  if [[ -z "$entry" ]]; then
    die "script runner: no entry point in $root for app=$BUCKYOS_APP_ID"
  fi

  # Backward-compat env used by legacy script-service images.
  export SCRIPT_APP_ID="$BUCKYOS_APP_ID"
  export SCRIPT_PACKAGE_ROOT="$root"
  export SCRIPT_DATA_ROOT="$BUCKYOS_DATA_DIR"

  log "script app=${BUCKYOS_APP_ID} lang=${lang} entry=${entry}"

  case "$lang" in
    python)
      local venv="${BUCKYOS_INSTANCE_VOLUME}/venv"
      local dep_stamp="${BUCKYOS_INSTANCE_VOLUME}/.sync/venv-deps.sha256"
      local manifest=""
      if [[ -f "$root/pyproject.toml" ]]; then
        manifest="$root/pyproject.toml"
      elif [[ -f "$root/requirements.txt" ]]; then
        manifest="$root/requirements.txt"
      fi

      local want_hash=""
      if [[ -n "$manifest" ]]; then
        want_hash="$(sha256sum "$manifest" | awk '{print $1}')"
      fi
      local have_hash=""
      if [[ -f "$dep_stamp" ]]; then
        have_hash="$(cat "$dep_stamp" 2>/dev/null || true)"
      fi

      local need_install=0
      if [[ ! -x "$venv/bin/python" ]]; then
        log "creating venv at $venv"
        uv venv "$venv"
        need_install=1
      elif [[ "$want_hash" != "$have_hash" ]]; then
        # Upstream sync (R-15) may have replaced the manifest. Re-install so
        # that a fresh code revision doesn't run against a stale venv.
        log "venv deps manifest changed (have=${have_hash:-<none>} want=${want_hash:-<none>}), reinstalling"
        need_install=1
      fi

      if (( need_install )) && [[ -n "$manifest" ]]; then
        if [[ "$manifest" == *pyproject.toml ]]; then
          (cd "$root" && uv pip install --python "$venv/bin/python" .)
        else
          uv pip install --python "$venv/bin/python" -r "$manifest"
        fi
        mkdir -p "$(dirname "$dep_stamp")"
        printf '%s' "$want_hash" > "$dep_stamp"
      elif (( need_install )); then
        # No manifest — nothing to install, but still stamp so we don't retry
        # every start for an app that genuinely has no deps.
        mkdir -p "$(dirname "$dep_stamp")"
        : > "$dep_stamp"
      fi

      exec "$venv/bin/python" "$entry" "$@"
      ;;
    typescript)
      if [[ -f "$root/deno.json" ]] || [[ -f "$root/deno.jsonc" ]]; then
        deno cache "$entry" 2>/dev/null || true
      fi
      exec deno run --allow-all "$entry" "$@"
      ;;
    *)
      die "script runner: unsupported language ($lang) for app=$BUCKYOS_APP_ID"
      ;;
  esac
}

# ---------------------------------------------------------------------------
# Dispatch
# ---------------------------------------------------------------------------

case "$BUCKYOS_APP_TYPE" in
  agent)
    : "${BUCKYOS_SERVICE_PORT:=4060}"
    OPENDAN_BIN="${BUCKYOS_ROOT}/bin/opendan/opendan"
    if [[ ! -x "$OPENDAN_BIN" ]]; then
      die "agent dispatch: OpenDAN binary not found at $OPENDAN_BIN"
    fi
    sync_agent_upstream_into_data_root
    # §9: Agent Root lives in the data layer, not in the pkg working copy.
    # OpenDAN creates sessions/, memory/, skills/, todo/, worklog/ on demand
    # — the exact layout under agent_root isn't frozen yet, so we only
    # guarantee the data root itself exists (already mkdir'd at the top).
    log "agent=${BUCKYOS_APP_ID} port=${BUCKYOS_SERVICE_PORT} data=${BUCKYOS_DATA_DIR}"
    exec "$OPENDAN_BIN" \
      --appid "$BUCKYOS_APP_ID" \
      --agent-bin "$BUCKYOS_PKG_DIR" \
      --service-port "$BUCKYOS_SERVICE_PORT" \
      "$@"
    ;;

  script)
    run_script_app
    ;;

  custom|"")
    if [[ $# -gt 0 ]]; then
      log "custom dispatch: exec $*"
      exec "$@"
    fi
    die "no BUCKYOS_APP_TYPE set and no command provided"
    ;;

  *)
    die "unknown BUCKYOS_APP_TYPE=${BUCKYOS_APP_TYPE}"
    ;;
esac
