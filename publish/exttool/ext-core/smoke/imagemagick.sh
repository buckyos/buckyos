#!/bin/sh
set -eu
dir="$(mktemp -d)"
trap 'rm -rf "$dir"' EXIT
magick -size 4x4 xc:white "$dir/a.png"
magick identify "$dir/a.png" | grep -q '4x4'
convert -size 4x4 xc:white "$dir/b.png"
identify "$dir/b.png" | grep -q '4x4'
compare -metric AE "$dir/a.png" "$dir/b.png" null: 2>/dev/null
# On Linux the prefix configuration's conf.d adds LibreOffice's font directory, Noto CJK included: it has to be read.
[ "$(uname -s)" != Linux ] || magick -list font | grep -q 'Font: Noto-Sans-CJK-SC$'
