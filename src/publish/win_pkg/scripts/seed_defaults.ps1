$ErrorActionPreference = "Stop"































































$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path































































$Root = Split-Path -Parent $ScriptDir































































$DefaultsDir = Join-Path $Root ".buckyos_installer_defaults"































































































































# BEGIN AUTO-GENERATED: data_paths
$src = Join-Path $DefaultsDir "bin\applist.json"
$dst = Join-Path $Root "bin\applist.json"
if (-not (Test-Path $dst)) {
  if (Test-Path $src) {
    New-Item -ItemType Directory -Force -Path (Split-Path $dst -Parent) | Out-Null
    Copy-Item -Force -Path $src -Destination $dst
  }
}
$src = Join-Path $DefaultsDir "etc\node_gateway_info.json"
$dst = Join-Path $Root "etc\node_gateway_info.json"
if (-not (Test-Path $dst)) {
  if (Test-Path $src) {
    New-Item -ItemType Directory -Force -Path (Split-Path $dst -Parent) | Out-Null
    Copy-Item -Force -Path $src -Destination $dst
  }
}
$src = Join-Path $DefaultsDir "etc\user_gateway.yaml"
$dst = Join-Path $Root "etc\user_gateway.yaml"
if (-not (Test-Path $dst)) {
  if (Test-Path $src) {
    New-Item -ItemType Directory -Force -Path (Split-Path $dst -Parent) | Out-Null
    Copy-Item -Force -Path $src -Destination $dst
  }
}
$src = Join-Path $DefaultsDir "etc\post_gateway.yaml"
$dst = Join-Path $Root "etc\post_gateway.yaml"
if (-not (Test-Path $dst)) {
  if (Test-Path $src) {
    New-Item -ItemType Directory -Force -Path (Split-Path $dst -Parent) | Out-Null
    Copy-Item -Force -Path $src -Destination $dst
  }
}
$src = Join-Path $DefaultsDir "etc\node_gateway.json"
$dst = Join-Path $Root "etc\node_gateway.json"
if (-not (Test-Path $dst)) {
  if (Test-Path $src) {
    New-Item -ItemType Directory -Force -Path (Split-Path $dst -Parent) | Out-Null
    Copy-Item -Force -Path $src -Destination $dst
  }
}
$src = Join-Path $DefaultsDir "etc\cyfs_gateway.yaml"
$dst = Join-Path $Root "etc\cyfs_gateway.yaml"
if (-not (Test-Path $dst)) {
  if (Test-Path $src) {
    New-Item -ItemType Directory -Force -Path (Split-Path $dst -Parent) | Out-Null
    Copy-Item -Force -Path $src -Destination $dst
  }
}
$src = Join-Path $DefaultsDir "data"
$dst = Join-Path $Root "data"
$shouldCopy = $false
if (-not (Test-Path $dst)) { $shouldCopy = $true }
elseif (-not (Get-ChildItem -LiteralPath $dst -Force -ErrorAction SilentlyContinue | Select-Object -First 1)) { $shouldCopy = $true }
if ($shouldCopy -and (Test-Path $src)) {
  New-Item -ItemType Directory -Force -Path $dst | Out-Null
  Copy-Item -Recurse -Force -Path (Join-Path $src "*") -Destination $dst
}
$src = Join-Path $DefaultsDir "storage"
$dst = Join-Path $Root "storage"
$shouldCopy = $false
if (-not (Test-Path $dst)) { $shouldCopy = $true }
elseif (-not (Get-ChildItem -LiteralPath $dst -Force -ErrorAction SilentlyContinue | Select-Object -First 1)) { $shouldCopy = $true }
if ($shouldCopy -and (Test-Path $src)) {
  New-Item -ItemType Directory -Force -Path $dst | Out-Null
  Copy-Item -Recurse -Force -Path (Join-Path $src "*") -Destination $dst
}
$src = Join-Path $DefaultsDir "local"
$dst = Join-Path $Root "local"
$shouldCopy = $false
if (-not (Test-Path $dst)) { $shouldCopy = $true }
elseif (-not (Get-ChildItem -LiteralPath $dst -Force -ErrorAction SilentlyContinue | Select-Object -First 1)) { $shouldCopy = $true }
if ($shouldCopy -and (Test-Path $src)) {
  New-Item -ItemType Directory -Force -Path $dst | Out-Null
  Copy-Item -Recurse -Force -Path (Join-Path $src "*") -Destination $dst
}
$src = Join-Path $DefaultsDir "logs"
$dst = Join-Path $Root "logs"
$shouldCopy = $false
if (-not (Test-Path $dst)) { $shouldCopy = $true }
elseif (-not (Get-ChildItem -LiteralPath $dst -Force -ErrorAction SilentlyContinue | Select-Object -First 1)) { $shouldCopy = $true }
if ($shouldCopy -and (Test-Path $src)) {
  New-Item -ItemType Directory -Force -Path $dst | Out-Null
  Copy-Item -Recurse -Force -Path (Join-Path $src "*") -Destination $dst
}
# END AUTO-GENERATED: data_paths
