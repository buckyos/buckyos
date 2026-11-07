$ErrorActionPreference = 'Continue'
$dir = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid())
New-Item -ItemType Directory -Path $dir | Out-Null
Push-Location $dir
function Require($ok) { if (-not $ok) { throw "failed at line $($MyInvocation.ScriptLineNumber)" } }
try {
  & git init -q repo; Require ($LASTEXITCODE -eq 0)
  & git -C repo -c user.name=t -c user.email=t@t commit -q --allow-empty -m first; Require ($LASTEXITCODE -eq 0)
  Require ((& git -C repo log --oneline) -match 'first')
  # `curl` alone is an alias of Invoke-WebRequest in Windows PowerShell.
  Require ((& curl.cmd --version) -match 'curl')
  [IO.File]::WriteAllText("$dir\a.json", '{"a":[1,2]}')
  & jq -e '.a[1] == 2' "$dir\a.json" | Out-Null; Require ($LASTEXITCODE -eq 0)
  New-Item -ItemType Directory -Path tree\sub | Out-Null
  [IO.File]::WriteAllText("$dir\tree\sub\x", '')
  Require ((& tree tree) -match 'x')
  & python3 -c "import zipfile; zipfile.ZipFile('a.zip','w').writestr('inside.txt','hello')"; Require ($LASTEXITCODE -eq 0)
  & unzip -q a.zip -d out; Require ($LASTEXITCODE -eq 0)
  Require ((Get-Content out\inside.txt) -match 'hello')
  [IO.File]::WriteAllText("$dir\f", 'hello')
  & xz -k f; Require ($LASTEXITCODE -eq 0)
  Require ((& xz -dc f.xz) -match 'hello')
} finally { Pop-Location; Remove-Item -Recurse -Force $dir }
