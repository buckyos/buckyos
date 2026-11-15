#!/bin/sh
# Headless, with no display, no network and a small /dev/shm, as root too (hence --no-sandbox): a page with Chinese and
# Latin text to PDF, and the text comes back, so the browser found a font for it.
set -eu
dir="$(mktemp -d)"
trap 'rm -rf "$dir" 2>/dev/null || true' EXIT
chromium --version | grep -q '[0-9][0-9]*\.[0-9]'
printf '<!doctype html><meta charset="utf-8"><title>smoke</title><p>中英混排 mixed text</p>\n' > "$dir/a.html"
if ! chromium --headless=new --no-sandbox --disable-gpu --disable-dev-shm-usage --no-first-run --user-data-dir="$dir/profile" \
  --no-pdf-header-footer --print-to-pdf="$dir/a.pdf" "file://$dir/a.html" > "$dir/chromium.log" 2>&1; then
  cat "$dir/chromium.log" >&2
  exit 1
fi
pdftotext "$dir/a.pdf" "$dir/a.txt"
if ! grep -q '中英混排 mixed text' "$dir/a.txt"; then
  cat "$dir/chromium.log" >&2
  echo '[chromium] the PDF does not have the page text' >&2
  exit 1
fi
