#!/usr/bin/env bash
# Build the Linux payloads of ext-core (doc/pikg/format.md §5) into payload/<platform>/, then package them:
#
#   ./build.sh
#   buckyos pikg doctor dapp_meta --catalog ..
#   buckyos pikg build dapp_meta && buckyos pikg pack dapp_dist
#   buckyos app publish "$PWD" --app-doc dapp_dist/APPDOC.json     # the control-panel node's path
#
# pandoc is carried in the payload (pinned version + sha256 in ../install_pandoc.sh, shared with the paios/exttool
# seed image). The other commands are still apt packages of the aios image (build_aios RUNTIME_PACKAGES, Node, uv,
# Deno), so this payload only names them: its launcher runs the next one of that name on PATH and exits 127 when the
# machine has none. A bare Linux host therefore gets no working ffmpeg from this payload; carrying the tools themselves
# is T2 of notepads/opendan-exttools-cross-platform-todo.md, and nothing installs the package before X3.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
manifest="$here/ext-manifest.json"

python_bin="${PYTHON:-python3}"
commands="$("$python_bin" -c 'import json, sys; print(" ".join(p["command"] for p in json.load(open(sys.argv[1]))["provides"]))' "$manifest")"

write_pandoc_launcher() {
  cat > "$1/bin/pandoc" <<'SH'
#!/bin/sh
exec "$(cd "$(dirname "$0")" && pwd -P)/../store/ext-core/pandoc/bin/pandoc" "$@"
SH
  chmod 755 "$1/bin/pandoc"
}

write_delegate_launcher() {
  cat > "$1/bin/$2" <<SH
#!/bin/sh
# ext-core does not carry $2 on Linux yet: the next one on PATH serves.
self="\$(cd "\$(dirname "\$0")" && pwd -P)"
IFS=:
for dir in \$PATH; do
  [ "\$(cd "\$dir" 2>/dev/null && pwd -P)" = "\$self" ] && continue
  [ -x "\$dir/$2" ] && exec "\$dir/$2" "\$@"
done
echo "$2: not on this machine, and ext-core's Linux payload does not carry it yet" >&2
exit 127
SH
  chmod 755 "$1/bin/$2"
}

for pair in linux-x86_64:amd64 linux-aarch64:aarch64; do
  platform="${pair%%:*}"
  payload="$here/payload/$platform"
  rm -rf "$payload"
  mkdir -p "$payload/bin" "$payload/smoke" "$payload/store/ext-core"
  cp "$manifest" "$payload/ext-manifest.json"
  cp "$here"/smoke/*.sh "$payload/smoke/"
  BUCKYOS_TOOLS_DIR="$payload" PANDOC_STORE_REL=store/ext-core/pandoc PANDOC_LINKS="" TARGET_ARCH="${pair##*:}" \
    bash "$here/../install_pandoc.sh"
  for command in $commands; do
    if [ "$command" = pandoc ]; then
      write_pandoc_launcher "$payload"
    else
      write_delegate_launcher "$payload" "$command"
    fi
  done
  echo "[ext-core] $platform: $payload"
done
