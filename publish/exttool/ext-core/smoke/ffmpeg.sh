#!/bin/sh
# Generate a one-second clip, then read it back.
set -eu
dir="$(mktemp -d)"
trap 'rm -rf "$dir"' EXIT
ffmpeg -hide_banner -loglevel error -f lavfi -i testsrc=size=16x16:rate=1:duration=1 -y "$dir/clip.mkv"
ffprobe -hide_banner -loglevel error -show_entries stream=codec_type -of csv=p=0 "$dir/clip.mkv" | grep -q video
# Fonts: the configuration compiled in is the build prefix's, so the launcher has to say where it went. The machine may
# have no font at all, so only the configuration is checked, not whether the text is drawn.
if ffmpeg -hide_banner -loglevel warning -f lavfi -i color=size=16x16:rate=1:duration=1 -vf drawtext=text=x -f null - 2>&1 | grep -q 'Cannot load default config'; then
  exit 1
fi
