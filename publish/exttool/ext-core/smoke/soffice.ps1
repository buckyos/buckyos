# The same as soffice.sh. Windows PowerShell reads a script without a BOM in the ANSI code page, so the Chinese text is
# written as escapes and python3 does the comparing.
$ErrorActionPreference = 'Continue'
$dir = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid())
New-Item -ItemType Directory -Path $dir | Out-Null
Push-Location $dir
function Require($ok) { if (-not $ok) { throw "failed at line $($MyInvocation.ScriptLineNumber)" } }
try {
  & magick -size 32x32 xc:red a.png; Require ($LASTEXITCODE -eq 0)
  $make = @'
import zipfile
open("a.md", "w", encoding="utf-8").write("# 标题 Title\n\n中英混排 mixed text\n\n![](a.png)\n")
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
'@
  [IO.File]::WriteAllText("$dir\make.py", $make)
  & python3 make.py; Require ($LASTEXITCODE -eq 0)
  & pandoc a.md -o a.docx; Require ($LASTEXITCODE -eq 0)
  $profile_url = 'file:///' + ("$dir\profile" -replace '\\', '/' -replace ' ', '%20')
  & soffice "-env:UserInstallation=$profile_url" --headless --convert-to pdf --outdir out a.docx b.xlsx 2>$null | Out-Null
  Require (Test-Path out\a.pdf); Require (Test-Path out\b.pdf)
  & pdftotext -enc UTF-8 out\a.pdf a.txt 2>$null; Require ($LASTEXITCODE -eq 0)
  & pdftotext -enc UTF-8 out\b.pdf b.txt 2>$null; Require ($LASTEXITCODE -eq 0)
  $check = @'
import re
assert "中英混排 mixed text" in open("a.txt", encoding="utf-8").read()
assert "表格 sheet" in open("b.txt", encoding="utf-8").read()
pdf = open("out/a.pdf", "rb").read()
assert re.search(rb"NotoS[a-z]*CJK", pdf)
assert re.search(rb"/Subtype */Image", pdf)
'@
  [IO.File]::WriteAllText("$dir\check.py", $check)
  & python3 check.py; Require ($LASTEXITCODE -eq 0)
} finally { Pop-Location; Remove-Item -Recurse -Force $dir }
