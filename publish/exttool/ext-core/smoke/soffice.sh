#!/bin/sh
# A docx with Chinese and Latin text and a picture, and an xlsx, to PDF: the text comes back, set in the CJK fonts of the
# payload (a machine's own fonts are not looked at), and the picture is in the page.
set -eu
if [ -n "${EXT_CORE_SMOKE_ARTIFACTS:-}" ]; then
  mkdir -p "$EXT_CORE_SMOKE_ARTIFACTS"
  dir="$(mktemp -d "$EXT_CORE_SMOKE_ARTIFACTS/soffice.XXXXXX")"
else
  dir="$(mktemp -d)"
fi
cleanup() {
  status=$?
  trap - EXIT
  if [ "$status" -eq 0 ]; then
    rm -rf "$dir"
  else
    printf '[soffice] failed (exit %s); files preserved at %s\n' "$status" "$dir" >&2
  fi
  exit "$status"
}
trap cleanup EXIT
cd "$dir"
echo '[soffice] preparing DOCX and XLSX fixtures'
magick -size 32x32 xc:red a.png
printf '# 标题 Title\n\n中英混排 mixed text\n\n![](a.png)\n' > a.md
pandoc a.md -o a.docx
python3 - <<'PY'
import zipfile
parts = {
    "[Content_Types].xml": '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>',
    "_rels/.rels": '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>',
    "xl/workbook.xml": '<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="s" sheetId="1" r:id="rId1"/></sheets></workbook>',
    "xl/_rels/workbook.xml.rels": '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>',
    "xl/worksheets/sheet1.xml": '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>表格 sheet</t></is></c><c r="B1"><v>42</v></c></row></sheetData></worksheet>',
}
with zipfile.ZipFile("b.xlsx", "w") as z:
    for name, text in parts.items():
        z.writestr(name, '<?xml version="1.0" encoding="UTF-8"?>' + text)
PY
echo '[soffice] converting DOCX and XLSX to PDF'
if soffice "-env:UserInstallation=file://$dir/profile" --headless --convert-to pdf --outdir out a.docx b.xlsx >soffice.log 2>&1; then
  cat soffice.log
else
  status=$?
  cat soffice.log >&2
  printf '[soffice] conversion failed (exit %s)\n' "$status" >&2
  exit "$status"
fi
echo '[soffice] extracting PDF text'
for name in a b; do
  if ! pdftotext "out/$name.pdf" "$name.txt"; then
    printf '[soffice] could not extract text from out/%s.pdf\n' "$name" >&2
    exit 1
  fi
done
require_text() {
  if ! grep -q "$2" "$1"; then
    printf '[soffice] %s is missing expected text: %s\n' "$1" "$2" >&2
    python3 - "$1" <<'PY'
from pathlib import Path
import sys
print(ascii(Path(sys.argv[1]).read_text(encoding="utf-8", errors="replace")), file=sys.stderr)
PY
    exit 1
  fi
}
echo '[soffice] checking DOCX and XLSX text'
require_text a.txt '中英混排 mixed text'
require_text b.txt '表格 sheet'
echo '[soffice] checking embedded Noto CJK font and image'
if ! grep -a -q 'NotoS[a-z]*CJK' out/a.pdf; then
  echo '[soffice] DOCX PDF has no embedded Noto CJK font marker' >&2
  exit 1
fi
if ! grep -a -q '/Subtype */Image' out/a.pdf; then
  echo '[soffice] DOCX PDF has no image marker' >&2
  exit 1
fi
echo '[soffice] passed'
