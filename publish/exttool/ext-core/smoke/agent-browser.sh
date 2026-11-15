#!/bin/sh
# What agent-browser does without an Agent Service: the versions, the environment for a project with another
# Playwright version, and a plain refusal when there is no browser to reach.
set -eu
unset OPENDAN_BROWSER_RPC
agent-browser version | grep -q '^chromium [0-9][0-9.]* (revision [0-9]*)$'
agent-browser version | grep -q '^playwright-core [0-9][0-9.]*$'
agent-browser --help | grep -q 'agent-browser run'
agent-browser run --help | grep -q 'export default'
vars="$(agent-browser env)"
printf '%s\n' "$vars" | grep -q '^export PLAYWRIGHT_BROWSERS_PATH='
if [ "$(uname -s)" = Linux ]; then
  (
    eval "$vars"
    [ -e "${LD_LIBRARY_PATH%%:*}/libnss3.so" ]
    [ -f "$FONTCONFIG_FILE" ]
    [ -d "$FONTCONFIG_PATH/conf.d" ]
  )
fi
status=0
refusal="$(agent-browser status 2>&1)" || status=$?
[ "$status" -eq 3 ]
printf '%s\n' "$refusal" | grep -q 'OPENDAN_BROWSER_RPC is not set'
