#!/bin/sh
set -eu
printf '# title\n' | pandoc -f markdown -t html | grep -q '<h1'
