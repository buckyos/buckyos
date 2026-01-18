$ErrorActionPreference = "Stop"

# BEGIN AUTO-GENERATED: data_paths

$src = Join-Path $DefaultsDir "bin\applist.json"

$dst = Join-Path $Root "bin\applist.json"

if (-not (Test-Path $dst)) {

  if (Test-Path $src) {

    New-Item -ItemType Directory -Force -Path (Split-Path $dst -Parent) | Out-Null

    Copy-Item -Force -Path $src -Destination $dst

  }

}

$src = Join-Path $DefaultsDir "bin\cyfs-gateway"

$dst = Join-Path $Root "bin\cyfs-gateway"

$shouldCopy = $false

if (-not (Test-Path $dst)) { $shouldCopy = $true }

elseif (-not (Get-ChildItem -LiteralPath $dst -Force -ErrorAction SilentlyContinue | Select-Object -First 1)) { $shouldCopy = $true }

if ($shouldCopy -and (Test-Path $src)) {

  New-Item -ItemType Directory -Force -Path $dst | Out-Null

  Copy-Item -Recurse -Force -Path (Join-Path $src "*") -Destination $dst

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

$src = Join-Path $DefaultsDir "cache"

$dst = Join-Path $Root "cache"

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

$src = Join-Path $DefaultsDir "tmp"

$dst = Join-Path $Root "tmp"

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

$src = Join-Path $DefaultsDir "home"

$dst = Join-Path $Root "home"

$shouldCopy = $false

if (-not (Test-Path $dst)) { $shouldCopy = $true }

elseif (-not (Get-ChildItem -LiteralPath $dst -Force -ErrorAction SilentlyContinue | Select-Object -First 1)) { $shouldCopy = $true }

if ($shouldCopy -and (Test-Path $src)) {

  New-Item -ItemType Directory -Force -Path $dst | Out-Null

  Copy-Item -Recurse -Force -Path (Join-Path $src "*") -Destination $dst

}

$src = Join-Path $DefaultsDir "library"

$dst = Join-Path $Root "library"

$shouldCopy = $false

if (-not (Test-Path $dst)) { $shouldCopy = $true }

elseif (-not (Get-ChildItem -LiteralPath $dst -Force -ErrorAction SilentlyContinue | Select-Object -First 1)) { $shouldCopy = $true }

if ($shouldCopy -and (Test-Path $src)) {

  New-Item -ItemType Directory -Force -Path $dst | Out-Null

  Copy-Item -Recurse -Force -Path (Join-Path $src "*") -Destination $dst

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

# END AUTO-GENERATED: data_paths

