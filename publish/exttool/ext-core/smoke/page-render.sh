#!/bin/sh
# A local page with Chinese and Latin text, a picture and a request to the network, to a three-page A4 PDF with page
# numbers and to a PNG: the text comes back, the picture is in the page, the request was refused.
set -eu
if [ -n "${EXT_CORE_SMOKE_ARTIFACTS:-}" ]; then
  mkdir -p "$EXT_CORE_SMOKE_ARTIFACTS"
  dir="$(mktemp -d "$EXT_CORE_SMOKE_ARTIFACTS/page-render.XXXXXX")"
else
  dir="$(mktemp -d)"
fi
cleanup() {
  status=$?
  trap - EXIT
  if [ "$status" -eq 0 ]; then
    rm -rf "$dir"
  else
    printf '[page-render] failed (exit %s); files preserved at %s\n' "$status" "$dir" >&2
  fi
  exit "$status"
}
trap cleanup EXIT
cd "$dir"
page-render --version | grep -q '^page-render [0-9][0-9.]* chromium [0-9][0-9.]* playwright-core [0-9][0-9.]*$'
echo '[page-render] preparing the page'
mkdir doc
magick -size 32x32 xc:red doc/a.png
cat > doc/index.html <<'HTML'
<!doctype html>
<html lang="zh"><head><meta charset="utf-8"><title>&#x6807;&#x9898; Title</title>
<style>section { break-after: page; } .cover { background: #cde; }</style></head>
<body>
<section class="cover"><h1>&#x6807;&#x9898; Title</h1><p>&#x4e2d;&#x82f1;&#x6df7;&#x6392; mixed text</p><img src="a.png"><img src="http://ext-core.invalid/b.png"></section>
<section><p>&#x7b2c;&#x4e8c;&#x9875; page two</p></section>
<section><p>&#x7b2c;&#x4e09;&#x9875; page three</p></section>
</body></html>
HTML
printf '%s\n' '<div style="font-size: 9px; width: 100%; text-align: center"><span class="pageNumber"></span>/<span class="totalPages"></span></div>' > footer.html
echo '[page-render] rendering to PDF'
page-render --html doc --pdf out/a.pdf --paper A4 --margin 15 --footer-html footer.html --background > pdf.json
cat pdf.json
jq -e '.pages == 3 and .bytes > 0 and (.sandbox | type) == "boolean" and .warnings == ["blocked: http://ext-core.invalid/b.png"]' pdf.json >/dev/null
pdfinfo out/a.pdf | grep -q '^Pages: *3'
pdfinfo out/a.pdf | grep -q '^Page size:.*A4'
echo '[page-render] checking the PDF text, font and picture'
pdftotext out/a.pdf a.txt
mixed_text="$(python3 -c 'print("\u4e2d\u82f1\u6df7\u6392 mixed text")')"
third_page="$(python3 -c 'print("\u7b2c\u4e09\u9875 page three")')"
for expected in "$mixed_text" "$third_page" '2/3'; do
  if ! grep -q "$expected" a.txt; then
    printf '[page-render] the PDF is missing expected text: %s\n' "$expected" >&2
    exit 1
  fi
done
if ! grep -a -q '/Subtype */Image' out/a.pdf; then
  echo '[page-render] the PDF has no picture' >&2
  exit 1
fi
# Linux: the browser's fonts are the payload's. Elsewhere they are the system's.
if [ "$(uname -s)" = Linux ] && ! grep -a -q 'NotoS[a-z]*CJK' out/a.pdf; then
  echo '[page-render] the PDF has no embedded Noto CJK font marker' >&2
  exit 1
fi
echo '[page-render] rendering to PNG'
page-render --html doc/index.html --png out/a.png --viewport 400x300 --scale 2 > png.json
jq -e '.pages == 1 and .bytes > 0' png.json >/dev/null
[ "$(magick identify -format '%w' out/a.png)" = 800 ]
echo '[page-render] passed'
