#!/bin/sh
# A docx explicitly using the payload's Noto CJK font, with Chinese and Latin text and a picture, and an xlsx, to PDF:
# the text comes back, the requested font is embedded, and the picture is in the page. Then a two-page Visio drawing
# (make_visio.py): Draw opens it through the Visio import filter, both pages, their text and the picture come back; and
# a Visio binary (visio-fdo57117.vsd, from LibreOffice's tests, see THIRD_PARTY.md): one page with its text.
set -eu
smoke_dir="$(cd "$(dirname "$0")" && pwd)"
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
mixed_text="$(python3 -c 'print("\u4e2d\u82f1\u6df7\u6392 mixed text")')"
sheet_text="$(python3 -c 'print("\u8868\u683c sheet")')"
python3 - <<'PYTHON'
from pathlib import Path
Path("a.md").write_text("# \u6807\u9898 Title\n\n\u4e2d\u82f1\u6df7\u6392 mixed text\n\n![](a.png)\n", encoding="utf-8")
PYTHON
pandoc a.md -o a.docx
python3 "$smoke_dir/soffice_docx.py" a.docx
python3 - <<'PY'
import zipfile
parts = {
    "[Content_Types].xml": '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>',
    "_rels/.rels": '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>',
    "xl/workbook.xml": '<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="s" sheetId="1" r:id="rId1"/></sheets></workbook>',
    "xl/_rels/workbook.xml.rels": '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>',
    "xl/worksheets/sheet1.xml": '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>\u8868\u683c sheet</t></is></c><c r="B1"><v>42</v></c></row></sheetData></worksheet>',
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
require_text a.txt "$mixed_text"
require_text b.txt "$sheet_text"
echo '[soffice] checking embedded Noto CJK font and image'
if ! grep -a -q 'NotoS[a-z]*CJK' out/a.pdf; then
  echo '[soffice] DOCX PDF has no embedded Noto CJK font marker' >&2
  exit 1
fi
if ! grep -a -q '/Subtype */Image' out/a.pdf; then
  echo '[soffice] DOCX PDF has no image marker' >&2
  exit 1
fi
echo '[soffice] converting a VSDX drawing to PDF'
flow_text="$(python3 -c 'print("\u5f00\u59cb Start")')"
caption_text="$(python3 -c 'print("\u56fe\u7247\u8bf4\u660e Picture caption")')"
python3 "$smoke_dir/make_visio.py" c.vsdx
if ! soffice "-env:UserInstallation=file://$dir/profile" --headless --convert-to pdf --outdir out c.vsdx >visio.log 2>&1 || [ ! -s out/c.pdf ]; then
  cat visio.log >&2
  echo '[soffice] VSDX conversion failed' >&2
  exit 1
fi
cat visio.log
if ! grep -q 'as a Draw document' visio.log; then
  echo '[soffice] the VSDX drawing was not opened by Draw' >&2
  exit 1
fi
pages="$(pdfinfo out/c.pdf | sed -n 's/^Pages: *//p')"
if [ "$pages" != 2 ]; then
  printf '[soffice] VSDX PDF has %s pages, expected 2\n' "$pages" >&2
  exit 1
fi
pdftotext out/c.pdf c.txt
require_text c.txt "$flow_text"
require_text c.txt "$caption_text"
if ! grep -a -q '/Subtype */Image' out/c.pdf; then
  echo '[soffice] VSDX PDF has no picture' >&2
  exit 1
fi
echo '[soffice] converting a VSD drawing to PDF'
cp "$smoke_dir/visio-fdo57117.vsd" d.vsd
if ! soffice "-env:UserInstallation=file://$dir/profile" --headless --convert-to pdf --outdir out d.vsd >vsd.log 2>&1 || [ ! -s out/d.pdf ]; then
  cat vsd.log >&2
  echo '[soffice] VSD conversion failed' >&2
  exit 1
fi
cat vsd.log
if ! grep -q 'as a Draw document' vsd.log; then
  echo '[soffice] the VSD drawing was not opened by Draw' >&2
  exit 1
fi
pages="$(pdfinfo out/d.pdf | sed -n 's/^Pages: *//p')"
if [ "$pages" != 1 ]; then
  printf '[soffice] VSD PDF has %s pages, expected 1\n' "$pages" >&2
  exit 1
fi
pdftotext out/d.pdf d.txt
require_text d.txt 'test'
if ! cmp -s "$smoke_dir/visio-fdo57117.vsd" d.vsd; then
  echo '[soffice] the VSD input was changed by the conversion' >&2
  exit 1
fi
echo '[soffice] passed'
