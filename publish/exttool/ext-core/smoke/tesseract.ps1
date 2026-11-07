$ErrorActionPreference = 'Continue'
$langs = & tesseract --list-langs 2>&1 | ForEach-Object { "$_".Trim() }
if ($langs -notcontains 'eng' -or $langs -notcontains 'osd') { exit 1 }
