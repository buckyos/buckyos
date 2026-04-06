#!/usr/bin/env bash
set -euo pipefail

PACKAGE_ROOT="${SCRIPT_PACKAGE_ROOT:-/opt/script/package}"
DATA_ROOT="${SCRIPT_DATA_ROOT:-/opt/script/data}"
APP_ID="${SCRIPT_APP_ID:-unknown}"

detect_language() {
  if [[ -f "$PACKAGE_ROOT/deno.json" ]] || [[ -f "$PACKAGE_ROOT/deno.jsonc" ]]; then
    echo "typescript"
    return
  fi
  if [[ -f "$PACKAGE_ROOT/pyproject.toml" ]] || [[ -f "$PACKAGE_ROOT/requirements.txt" ]]; then
    echo "python"
    return
  fi
  for ext in ts tsx; do
    for name in main start index; do
      if [[ -f "$PACKAGE_ROOT/$name.$ext" ]]; then
        echo "typescript"
        return
      fi
    done
  done
  for name in main start; do
    if [[ -f "$PACKAGE_ROOT/$name.py" ]]; then
      echo "python"
      return
    fi
  done
  echo "unknown"
}

find_entry() {
  local lang="$1"
  if [[ -f "$PACKAGE_ROOT/buckyos_script.json" ]]; then
    local entry
    entry="$(python3 -c "
import json, sys
with open('$PACKAGE_ROOT/buckyos_script.json') as f:
    print(json.load(f).get('entry', ''))
" 2>/dev/null || true)"
    if [[ -n "$entry" ]] && [[ -f "$PACKAGE_ROOT/$entry" ]]; then
      echo "$PACKAGE_ROOT/$entry"
      return
    fi
  fi

  case "$lang" in
    typescript)
      for candidate in main.ts start.ts index.ts main.tsx start.tsx index.tsx; do
        if [[ -f "$PACKAGE_ROOT/$candidate" ]]; then
          echo "$PACKAGE_ROOT/$candidate"
          return
        fi
      done
      ;;
    python)
      for candidate in main.py start.py __main__.py; do
        if [[ -f "$PACKAGE_ROOT/$candidate" ]]; then
          echo "$PACKAGE_ROOT/$candidate"
          return
        fi
      done
      ;;
  esac
  echo ""
}

install_python_deps() {
  local marker="$DATA_ROOT/.deps_installed_py"
  if [[ -f "$marker" ]]; then
    return
  fi

  local venv_dir="$DATA_ROOT/.venv"
  if [[ -f "$PACKAGE_ROOT/pyproject.toml" ]]; then
    echo "[script-service] installing Python dependencies from pyproject.toml"
    uv venv "$venv_dir"
    (cd "$PACKAGE_ROOT" && uv pip install --python "$venv_dir/bin/python" .)
  elif [[ -f "$PACKAGE_ROOT/requirements.txt" ]]; then
    echo "[script-service] installing Python dependencies from requirements.txt"
    uv venv "$venv_dir"
    uv pip install --python "$venv_dir/bin/python" -r "$PACKAGE_ROOT/requirements.txt"
  fi

  touch "$marker"
}

install_ts_deps() {
  local marker="$DATA_ROOT/.deps_installed_ts"
  if [[ -f "$marker" ]]; then
    return
  fi

  if [[ -f "$PACKAGE_ROOT/deno.json" ]] || [[ -f "$PACKAGE_ROOT/deno.jsonc" ]]; then
    echo "[script-service] caching Deno dependencies"
    local entry="$1"
    DENO_DIR="$DATA_ROOT/.deno" deno cache "$entry" 2>/dev/null || true
  fi

  touch "$marker"
}

lang="$(detect_language)"
entry="$(find_entry "$lang")"

if [[ -z "$entry" ]]; then
  echo "[script-service] ERROR: no entry point found in $PACKAGE_ROOT for app=$APP_ID" >&2
  ls -la "$PACKAGE_ROOT" >&2 || true
  exit 1
fi

echo "[script-service] app=$APP_ID lang=$lang entry=$entry"
mkdir -p "$DATA_ROOT"

case "$lang" in
  python)
    install_python_deps
    if [[ -f "$DATA_ROOT/.venv/bin/python" ]]; then
      exec "$DATA_ROOT/.venv/bin/python" "$entry" "$@"
    else
      exec uv run --no-project "$entry" "$@"
    fi
    ;;
  typescript)
    install_ts_deps "$entry"
    export DENO_DIR="$DATA_ROOT/.deno"
    exec deno run --allow-all "$entry" "$@"
    ;;
  *)
    echo "[script-service] ERROR: unsupported language ($lang) for app=$APP_ID" >&2
    exit 1
    ;;
esac
