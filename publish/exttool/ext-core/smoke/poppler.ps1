$ErrorActionPreference = 'Continue'
$dir = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid())
New-Item -ItemType Directory -Path $dir | Out-Null
try {
  $pdf = "%PDF-1.4`n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj`n2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj`n3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 72 72]>>endobj`ntrailer<</Root 1 0 R>>`n"
  [IO.File]::WriteAllText("$dir\a.pdf", $pdf)
  $info = & pdfinfo "$dir\a.pdf" 2>$null
  if (-not ($info -match '^Pages:\s*1')) { exit 1 }
  & pdftotext "$dir\a.pdf" "$dir\a.txt" 2>$null
  & pdftoppm -r 10 -png "$dir\a.pdf" "$dir\page" 2>$null
  if (-not (Test-Path "$dir\page*.png")) { exit 1 }
} finally { Remove-Item -Recurse -Force $dir }
