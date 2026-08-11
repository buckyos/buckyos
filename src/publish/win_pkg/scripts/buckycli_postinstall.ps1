$ErrorActionPreference = "Stop"

$HookDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ScriptsDir = Split-Path -Parent $HookDir
$InstallDir = Split-Path -Parent $ScriptsDir
$NormalizedInstallDir = $InstallDir.TrimEnd("\")

$CurrentPath = [Environment]::GetEnvironmentVariable("Path", "User")
$PathItems = @()
if (-not [string]::IsNullOrWhiteSpace($CurrentPath)) {
  $PathItems = $CurrentPath -split ";" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
}

$AlreadyPresent = $false
foreach ($Item in $PathItems) {
  if ($Item.TrimEnd("\").Equals($NormalizedInstallDir, [StringComparison]::OrdinalIgnoreCase)) {
    $AlreadyPresent = $true
    break
  }
}

if (-not $AlreadyPresent) {
  if ([string]::IsNullOrWhiteSpace($CurrentPath)) {
    [Environment]::SetEnvironmentVariable("Path", $InstallDir, "User")
  } else {
    [Environment]::SetEnvironmentVariable("Path", "$CurrentPath;$InstallDir", "User")
  }
}
