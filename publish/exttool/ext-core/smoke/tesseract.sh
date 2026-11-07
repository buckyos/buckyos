#!/bin/sh
# The language data the package guarantees: eng and osd.
set -eu
langs="$(tesseract --list-langs 2>&1)"
printf '%s\n' "$langs" | grep -qx eng
printf '%s\n' "$langs" | grep -qx osd
