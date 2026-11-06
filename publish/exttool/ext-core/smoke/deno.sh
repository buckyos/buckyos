#!/bin/sh
set -eu
dir="$(mktemp -d)"
trap 'rm -rf "$dir"' EXIT
[ "$(DENO_DIR="$dir" deno eval 'console.log(6 * 7)')" = 42 ]
