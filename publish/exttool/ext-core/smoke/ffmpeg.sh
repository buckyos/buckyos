#!/bin/sh
# Generate a one-second clip, then read it back.
set -eu
dir="$(mktemp -d)"
trap 'rm -rf "$dir"' EXIT
ffmpeg -hide_banner -loglevel error -f lavfi -i testsrc=size=16x16:rate=1:duration=1 -y "$dir/clip.mkv"
ffprobe -hide_banner -loglevel error -show_entries stream=codec_type -of csv=p=0 "$dir/clip.mkv" | grep -q video
