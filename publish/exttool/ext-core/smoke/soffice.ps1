# The same as soffice.sh. Windows PowerShell reads a script without a BOM in the ANSI code page, so the Chinese text is
# written as escapes and python3 does the comparing.
$ErrorActionPreference = 'Continue'
$root = [IO.Path]::GetTempPath()
if ($env:EXT_CORE_SMOKE_ARTIFACTS) { $root = $env:EXT_CORE_SMOKE_ARTIFACTS }
$root = [IO.Path]::GetFullPath($root)
$dir = Join-Path $root ('soffice-' + [Guid]::NewGuid())
New-Item -ItemType Directory -Path $dir -ErrorAction Stop | Out-Null
Push-Location $dir -ErrorAction Stop
function Require($ok) { if (-not $ok) { throw "failed at line $($MyInvocation.ScriptLineNumber)" } }
$passed = $false
$stage = 'create fixtures'
try {
  Write-Output "[soffice] PowerShell $($PSVersionTable.PSVersion), ANSI code page $([Text.Encoding]::Default.CodePage)"
  & magick -size 32x32 xc:red a.png; Require ($LASTEXITCODE -eq 0)
  $make = @'
import zipfile
open("a.md", "w", encoding="utf-8").write("# \u6807\u9898 Title\n\n\u4e2d\u82f1\u6df7\u6392 mixed text\n\n![](a.png)\n")
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
'@
  [IO.File]::WriteAllText("$dir\make.py", $make)
  & python3 make.py; Require ($LASTEXITCODE -eq 0)
  & pandoc a.md -o a.docx; Require ($LASTEXITCODE -eq 0)
  & python3 (Join-Path $PSScriptRoot 'soffice_docx.py') a.docx; Require ($LASTEXITCODE -eq 0)
  $profile_url = 'file:///' + ("$dir\profile" -replace '\\', '/' -replace ' ', '%20')
  $stage = 'convert DOCX and XLSX to PDF'
  Write-Output "[soffice] $stage"
  & soffice "-env:UserInstallation=$profile_url" --headless --convert-to pdf --outdir out a.docx b.xlsx > convert.log 2>&1
  $convert_exit = $LASTEXITCODE
  Get-Content -LiteralPath convert.log
  Require ($convert_exit -eq 0)
  Require (Test-Path out\a.pdf); Require (Test-Path out\b.pdf)
  $stage = 'extract PDF text'
  & pdftotext -enc UTF-8 out\a.pdf a.txt; Require ($LASTEXITCODE -eq 0)
  & pdftotext -enc UTF-8 out\b.pdf b.txt; Require ($LASTEXITCODE -eq 0)
  $stage = 'verify PDF text, fonts and image'
  $check = @'
import re
a = open("a.txt", encoding="utf-8").read()
b = open("b.txt", encoding="utf-8").read()
print("[soffice] DOCX PDF text:", ascii(a), flush=True)
print("[soffice] XLSX PDF text:", ascii(b), flush=True)
assert "\u4e2d\u82f1\u6df7\u6392 mixed text" in a, "DOCX PDF text mismatch"
assert "\u8868\u683c sheet" in b, "XLSX PDF text mismatch"
pdf = open("out/a.pdf", "rb").read()
fonts = re.findall(rb"/BaseFont\s*/([^\s/<>()]+)", pdf)
print("[soffice] PDF fonts:", fonts, flush=True)
assert re.search(rb"NotoS[a-z]*CJK", pdf), "DOCX PDF does not embed a Noto CJK font"
assert re.search(rb"/Subtype */Image", pdf), "DOCX PDF does not contain an image"
'@
  [IO.File]::WriteAllText("$dir\check.py", $check)
  & python3 check.py; Require ($LASTEXITCODE -eq 0)
  $passed = $true
} catch {
  Write-Output "[soffice] failed during: $stage"
  throw
} finally {
  Pop-Location
  if ($passed) {
    if ([IO.Path]::GetFullPath($dir).StartsWith($root.TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
      Remove-Item -LiteralPath $dir -Recurse -Force
    } else { throw "Refusing to remove a path outside $root" }
  } else { Write-Output "[soffice] failure files: $dir" }
}
