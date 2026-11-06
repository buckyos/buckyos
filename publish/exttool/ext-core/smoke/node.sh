#!/bin/sh
set -eu
[ "$(node -e 'console.log(6 * 7)')" = 42 ]
npm --version >/dev/null
npx --version >/dev/null
