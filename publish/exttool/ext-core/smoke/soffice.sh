#!/bin/sh
# A docx with Chinese and Latin text and a picture, and an xlsx, to PDF: the text comes back, set in the CJK fonts of the
# payload (a machine's own fonts are not looked at), and the picture is in the page.
set -eu
dir="$(mktemp -d)"
trap 'rm -rf "$dir"' EXIT
cd "$dir"
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
soffice "-env:UserInstallation=file://$dir/profile" --headless --convert-to pdf --outdir out a.docx b.xlsx >/dev/null 2>&1
pdftotext out/a.pdf - | grep -q '中英混排 mixed text'
pdftotext out/b.pdf - | grep -q '表格 sheet'
grep -a -q 'NotoS[a-z]*CJK' out/a.pdf
grep -a -q '/Subtype */Image' out/a.pdf
