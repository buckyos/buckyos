$ErrorActionPreference = 'Continue'
$env:DENO_DIR = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid())
try {
  if ((& deno eval 'console.log(6 * 7)') -ne '42') { exit 1 }
} finally { Remove-Item -Recurse -Force $env:DENO_DIR -ErrorAction SilentlyContinue }
