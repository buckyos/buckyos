$ErrorActionPreference = 'Continue'
if ((& python3 -c 'print(6 * 7)') -ne '42') { exit 1 }
& uv --version | Out-Null
if ($LASTEXITCODE -ne 0) { exit 1 }
