#!/bin/sh
# Slot launcher for the macOS desktop acceptance run. Start it once from a
# Terminal window in the logged-in GUI session; the daemon then inherits that
# session (and Terminal's TCC attribution) exactly as a user-launched copy does.
# Each ~/bk-e2e/launch-<slot>.env (shell assignments: binary, url_file, extra)
# names the binary, URL file and extra arguments; the launcher runs the daemon once and records its exit code.
base="$HOME/bk-e2e"
mkdir -p "$base"
echo "launcher ready at $base (pid $$)"
while :; do
  for spec in "$base"/launch-*.env; do
    [ -f "$spec" ] || continue
    slot="${spec##*/launch-}"; slot="${slot%.env}"
    [ -f "$base/daemon-$slot.exit" ] && continue
    [ -f "$base/daemon-$slot.started" ] && continue
    : > "$base/daemon-$slot.started"
    binary=''; url_file=''; extra=''
    # shellcheck disable=SC1090
    . "$spec"
    (
      cd "$HOME" || exit 1
      # shellcheck disable=SC2086
      "$binary" $extra run --url-file "$url_file" > "$base/daemon-$slot.log" 2>&1
      echo "$?" > "$base/daemon-$slot.exit"
    ) &
  done
  sleep 1
done
