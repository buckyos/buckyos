# The same as agent-browser.sh.
$ErrorActionPreference = 'Continue'
function Require($ok) { if (-not $ok) { throw "failed at line $($MyInvocation.ScriptLineNumber)" } }
Remove-Item Env:OPENDAN_BROWSER_RPC -ErrorAction SilentlyContinue
$version = (& agent-browser version) -join "`n"; Require ($LASTEXITCODE -eq 0)
Require ($version -match '(?m)^chromium [0-9][0-9.]* \(revision [0-9]*\)$')
Require ($version -match '(?m)^playwright-core [0-9][0-9.]*$')
$help = (& agent-browser --help) -join "`n"; Require ($LASTEXITCODE -eq 0)
Require ($help -match 'agent-browser run')
$run_help = (& agent-browser run --help) -join "`n"; Require ($LASTEXITCODE -eq 0)
Require ($run_help -match 'export default')
$vars = (& agent-browser env) -join "`n"; Require ($LASTEXITCODE -eq 0)
Require ($vars -match '(?m)^export PLAYWRIGHT_BROWSERS_PATH=')
# The refusal is on stderr; cmd merges the streams without Windows PowerShell turning the text into error records.
$refusal = (& cmd /c 'agent-browser status 2>&1') -join "`n"
Require ($LASTEXITCODE -eq 3)
Require ($refusal -match 'OPENDAN_BROWSER_RPC is not set')
