#!/bin/zsh
set -u

export PATH="/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:${PATH:-}"

run_with_timeout() {
  local seconds="$1"
  shift
  /usr/bin/perl -e '$t=shift; alarm $t; exec @ARGV' "$seconds" "$@"
}

if ! command -v docker >/dev/null 2>&1; then
  echo "[buckyos] installation-check: docker CLI not found" >&2
  exit 10
fi

if ! run_with_timeout 15 docker info >/dev/null 2>&1; then
  echo "[buckyos] installation-check: docker daemon unavailable for current user" >&2
  exit 11
fi

exit 0
