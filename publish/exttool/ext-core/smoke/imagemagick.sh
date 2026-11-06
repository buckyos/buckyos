#!/bin/sh
# ImageMagick 7 (`magick`) where a payload has it, the ImageMagick 6 commands otherwise.
set -eu
dir="$(mktemp -d)"
trap 'rm -rf "$dir"' EXIT
if command -v magick >/dev/null 2>&1; then
  magick -size 4x4 xc:white "$dir/a.png"
  magick identify "$dir/a.png" | grep -q '4x4'
else
  convert -size 4x4 xc:white "$dir/a.png"
  identify "$dir/a.png" | grep -q '4x4'
  compare -metric AE "$dir/a.png" "$dir/a.png" null: 2>/dev/null
fi
