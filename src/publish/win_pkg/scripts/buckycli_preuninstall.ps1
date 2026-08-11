$ErrorActionPreference = "Stop"

$HookDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ScriptsDir = Split-Path -Parent $HookDir
$InstallDir = Split-Path -Parent $ScriptsDir
$NormalizedInstallDir = $InstallDir.TrimEnd("\")

$CurrentPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ([string]::IsNullOrWhiteSpace($CurrentPath)) {
  exit 0
}

$PathItems = $CurrentPath -split ";" | Where-Object {
  -not [string]::IsNullOrWhiteSpace($_) -and
  (-not $_.TrimEnd("\").Equals($NormalizedInstallDir, [StringComparison]::OrdinalIgnoreCase))
}

[Environment]::SetEnvironmentVariable("Path", ($PathItems -join ";"), "User")
