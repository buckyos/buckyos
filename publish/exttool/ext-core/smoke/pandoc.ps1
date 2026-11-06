$ErrorActionPreference = 'Continue'
$html = '# title' | & pandoc -f markdown -t html
if ($LASTEXITCODE -ne 0 -or ($html -notmatch '<h1')) { exit 1 }
