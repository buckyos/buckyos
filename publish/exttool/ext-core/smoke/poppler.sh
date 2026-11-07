#!/bin/sh
# A one-page PDF with no text: every poppler command must parse it.
set -eu
dir="$(mktemp -d)"
trap 'rm -rf "$dir"' EXIT
cat > "$dir/a.pdf" <<'PDF'
%PDF-1.4
1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj
2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj
3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 72 72]>>endobj
trailer<</Root 1 0 R>>
PDF
pdfinfo "$dir/a.pdf" 2>/dev/null | grep -q '^Pages: *1'
pdftotext "$dir/a.pdf" "$dir/a.txt" 2>/dev/null
pdftoppm -r 10 -png "$dir/a.pdf" "$dir/page" 2>/dev/null
ls "$dir"/page*.png >/dev/null
