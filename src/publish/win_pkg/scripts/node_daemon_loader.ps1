param(
  [string]$NodeDaemonPath
)

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$Root = Split-Path -Parent $ScriptDir

if ([string]::IsNullOrWhiteSpace($NodeDaemonPath)) {
  $NodeDaemonPath = Join-Path $Root "bin\node-daemon\node_daemon.exe"
}

try {
  $existing = Get-Process -Name "node_daemon" -ErrorAction SilentlyContinue
  if ($existing) {
    exit 0
  }

  if (-not (Test-Path -LiteralPath $NodeDaemonPath)) {
    exit 1
  }

  $startInfo = New-Object System.Diagnostics.ProcessStartInfo
  $startInfo.FileName = $NodeDaemonPath
  $startInfo.Arguments = "--enable_active"
  $startInfo.WorkingDirectory = Split-Path -Parent $NodeDaemonPath
  $startInfo.UseShellExecute = $false
  $startInfo.CreateNoWindow = $true

  [System.Diagnostics.Process]::Start($startInfo) | Out-Null
  exit 0
}
catch {
  exit 1
}
