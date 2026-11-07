$ErrorActionPreference = 'Continue'
if ((& node -e 'console.log(6 * 7)') -ne '42') { exit 1 }
& npm --version | Out-Null
if ($LASTEXITCODE -ne 0) { exit 1 }
& npx --version | Out-Null
if ($LASTEXITCODE -ne 0) { exit 1 }
