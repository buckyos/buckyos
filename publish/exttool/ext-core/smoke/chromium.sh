#!/bin/sh
# Headless, with no display, no network and a small /dev/shm, as root too (hence --no-sandbox): a page with Chinese and
# Latin text to PDF, and the text comes back, so the browser found a font for it.
set -eu
dir="$(mktemp -d)"
trap 'rm -rf "$dir" 2>/dev/null || true' EXIT
chromium --version | grep -q '[0-9][0-9]*\.[0-9]'
mixed_text="$(python3 -c 'print("\u4e2d\u82f1\u6df7\u6392 mixed text")')"
printf '<!doctype html><meta charset="utf-8"><title>smoke</title><p>%s</p>\n' "$mixed_text" > "$dir/a.html"
if ! chromium --headless=new --no-sandbox --disable-gpu --disable-dev-shm-usage --no-first-run --user-data-dir="$dir/profile" \
  --no-pdf-header-footer --print-to-pdf="$dir/a.pdf" "file://$dir/a.html" > "$dir/chromium.log" 2>&1; then
  cat "$dir/chromium.log" >&2
  exit 1
fi
pdftotext "$dir/a.pdf" "$dir/a.txt"
if ! grep -q "$mixed_text" "$dir/a.txt"; then
  cat "$dir/chromium.log" >&2
  echo '[chromium] the PDF does not have the page text' >&2
  exit 1
fi
