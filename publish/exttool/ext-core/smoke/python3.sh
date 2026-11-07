#!/bin/sh
set -eu
[ "$(python3 -c 'print(6 * 7)')" = 42 ]
uv --version >/dev/null
