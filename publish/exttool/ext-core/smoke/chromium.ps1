# The same as chromium.sh. Windows PowerShell reads a script without a BOM in the ANSI code page, so the Chinese text is
# written as escapes and python3 does the comparing.
$ErrorActionPreference = 'Continue'
$dir = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid())
New-Item -ItemType Directory -Path $dir | Out-Null
Push-Location $dir
function Require($ok) { if (-not $ok) { throw "failed at line $($MyInvocation.ScriptLineNumber)" } }
try {
  Require ((& chromium --version) -match '\d+\.\d+')
  $make = @'
open("a.html", "w", encoding="utf-8").write('<!doctype html><meta charset="utf-8"><title>smoke</title><p>\u4e2d\u82f1\u6df7\u6392 mixed text</p>\n')
'@
  [IO.File]::WriteAllText("$dir\make.py", $make)
  & python3 make.py; Require ($LASTEXITCODE -eq 0)
  $url = 'file:///' + ("$dir\a.html" -replace '\\', '/' -replace ' ', '%20')
  & chromium --headless=new --no-sandbox --disable-gpu --no-first-run "--user-data-dir=$dir\profile" --no-pdf-header-footer "--print-to-pdf=$dir\a.pdf" $url > chromium.log 2>&1
  $chromium_exit = $LASTEXITCODE
  Get-Content -LiteralPath chromium.log
  Require ($chromium_exit -eq 0)
  Require (Test-Path a.pdf)
  & pdftotext -enc UTF-8 a.pdf a.txt; Require ($LASTEXITCODE -eq 0)
  $check = @'
text = open("a.txt", encoding="utf-8").read()
assert "\u4e2d\u82f1\u6df7\u6392 mixed text" in text, ascii(text)
'@
  [IO.File]::WriteAllText("$dir\check.py", $check)
  & python3 check.py; Require ($LASTEXITCODE -eq 0)
} finally { Pop-Location; Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue }
