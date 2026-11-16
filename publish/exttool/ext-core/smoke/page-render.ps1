# The same as page-render.sh. Windows PowerShell reads a script without a BOM in the ANSI code page, so the Chinese text is
# written as escapes and python3 does the writing and the comparing. The fonts are the system's here.
$ErrorActionPreference = 'Continue'
$root = [IO.Path]::GetTempPath()
if ($env:EXT_CORE_SMOKE_ARTIFACTS) { $root = $env:EXT_CORE_SMOKE_ARTIFACTS }
$root = [IO.Path]::GetFullPath($root)
$dir = Join-Path $root ('page-render-' + [Guid]::NewGuid())
New-Item -ItemType Directory -Path $dir -ErrorAction Stop | Out-Null
Push-Location $dir -ErrorAction Stop
function Require($ok) { if (-not $ok) { throw "failed at line $($MyInvocation.ScriptLineNumber)" } }
$passed = $false
$stage = 'read the version'
try {
  $version = (& page-render --version) -join "`n"; Require ($LASTEXITCODE -eq 0)
  Require ($version -match '^page-render [0-9][0-9.]* chromium [0-9][0-9.]* playwright-core [0-9][0-9.]*$')
  $stage = 'create the page'
  New-Item -ItemType Directory -Path doc | Out-Null
  & magick -size 32x32 xc:red doc\a.png; Require ($LASTEXITCODE -eq 0)
  $make = @'
page = """<!doctype html>
<html lang="zh"><head><meta charset="utf-8"><title>\u6807\u9898 Title</title>
<style>section { break-after: page; } .cover { background: #cde; }</style></head>
<body>
<section class="cover"><h1>\u6807\u9898 Title</h1><p>\u4e2d\u82f1\u6df7\u6392 mixed text</p><img src="a.png"><img src="http://ext-core.invalid/b.png"></section>
<section><p>\u7b2c\u4e8c\u9875 page two</p></section>
<section><p>\u7b2c\u4e09\u9875 page three</p></section>
</body></html>
"""
open("doc/index.html", "w", encoding="utf-8").write(page)
open("footer.html", "w", encoding="utf-8").write('<div style="font-size: 9px; width: 100%; text-align: center"><span class="pageNumber"></span>/<span class="totalPages"></span></div>\n')
'@
  [IO.File]::WriteAllText("$dir\make.py", $make)
  & python3 make.py; Require ($LASTEXITCODE -eq 0)
  $stage = 'render to PDF'
  Write-Output "[page-render] $stage"
  $pdf_json = (& page-render --html doc --pdf out\a.pdf --paper A4 --margin 15 --footer-html footer.html --background) -join "`n"
  Require ($LASTEXITCODE -eq 0)
  Write-Output $pdf_json
  [IO.File]::WriteAllText("$dir\pdf.json", $pdf_json)
  $stage = 'render to PNG'
  Write-Output "[page-render] $stage"
  $png_json = (& page-render --html doc\index.html --png out\a.png --viewport 400x300 --scale 2) -join "`n"
  Require ($LASTEXITCODE -eq 0)
  [IO.File]::WriteAllText("$dir\png.json", $png_json)
  $stage = 'extract the PDF text'
  & pdftotext -enc UTF-8 out\a.pdf a.txt; Require ($LASTEXITCODE -eq 0)
  $pages = (& pdfinfo out\a.pdf) -join "`n"; Require ($LASTEXITCODE -eq 0)
  Require ($pages -match '(?m)^Pages: *3')
  Require ($pages -match '(?m)^Page size:.*A4')
  $width = & magick identify -format '%w' out\a.png; Require ($LASTEXITCODE -eq 0)
  Require ($width -eq '800')
  $stage = 'verify the results, the PDF text and the picture'
  $check = @'
import json, re
pdf = json.load(open("pdf.json", encoding="utf-8"))
png = json.load(open("png.json", encoding="utf-8"))
assert pdf["pages"] == 3 and pdf["bytes"] > 0 and isinstance(pdf["sandbox"], bool), pdf
assert pdf["warnings"] == ["blocked: http://ext-core.invalid/b.png"], pdf
assert png["pages"] == 1 and png["bytes"] > 0, png
text = open("a.txt", encoding="utf-8").read()
print("[page-render] PDF text:", ascii(text), flush=True)
for expected in ("\u4e2d\u82f1\u6df7\u6392 mixed text", "\u7b2c\u4e09\u9875 page three", "2/3"):
    assert expected in text, "the PDF is missing expected text: " + ascii(expected)
assert re.search(rb"/Subtype */Image", open("out/a.pdf", "rb").read()), "the PDF has no picture"
'@
  [IO.File]::WriteAllText("$dir\check.py", $check)
  & python3 check.py; Require ($LASTEXITCODE -eq 0)
  $passed = $true
} catch {
  Write-Output "[page-render] failed during: $stage"
  throw
} finally {
  Pop-Location
  if ($passed) {
    if ([IO.Path]::GetFullPath($dir).StartsWith($root.TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
      Remove-Item -LiteralPath $dir -Recurse -Force
    } else { throw "Refusing to remove a path outside $root" }
  } else { Write-Output "[page-render] failure files: $dir" }
}
