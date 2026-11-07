$ErrorActionPreference = 'Continue'
$dir = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid())
New-Item -ItemType Directory -Path $dir | Out-Null
try {
  & magick -size 4x4 xc:white "$dir\a.png"
  if ($LASTEXITCODE -ne 0) { exit 1 }
  $info = & magick identify "$dir\a.png"
  if ($info -notmatch '4x4') { exit 1 }
  & convert -size 4x4 xc:white "$dir\b.png" 2>$null
  if ($LASTEXITCODE -ne 0) { exit 1 }
  $info = & identify "$dir\b.png"
  if ($info -notmatch '4x4') { exit 1 }
  # `compare` alone is an alias of Compare-Object in Windows PowerShell.
  & compare.cmd -metric AE "$dir\a.png" "$dir\b.png" null: 2>$null
} finally { Remove-Item -Recurse -Force $dir }
