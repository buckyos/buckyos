# PowerShell one-click build-and-package helper (parity with make_pkg.sh intent).
# Usage:
#   ./make_pkg.ps1 -Version "0.5.1+build260115" -Arch amd64
# Notes:
#   - Assumes WiX candle/light in PATH for Windows MSI build.
#   - Uses BUCKYOS_BUILD_ROOT (default C:\opt\buckyosci) for staging/install roots.
#   - Runs cargo/buckyos-build/buckyos-install; requires dev toolchain available.

param(
    [string]$Version = "0.5.1+build-dev",
    [string]$Arch = "amd64",  # amd64|arm64
    [string]$BuildRoot = $env:BUCKYOS_BUILD_ROOT,
    [switch]$DryRun
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
if (-not $BuildRoot -or $BuildRoot.Trim() -eq "") {
    $BuildRoot = "C:\opt\buckyosci"
}
$buckyosRoot = Join-Path $BuildRoot "buckyos"
$buckycliRoot = Join-Path $BuildRoot "buckycli"

function Invoke-Step {
    param([string]$Title, [string[]]$Cmd, [string]$WorkDir = $null)
    Write-Host "==> $Title"
    if ($DryRun) {
        Write-Host "[dry-run] $($Cmd -join ' ')" -ForegroundColor Cyan
        return
    }
    if ($WorkDir) { Push-Location $WorkDir }
    try {
        if ($Cmd.Length -gt 0) {
            $cmdName = $Cmd[0]
            $cmdArgs = @()
            if ($Cmd.Length -gt 1) {
                $cmdArgs = $Cmd[1..($Cmd.Length - 1)]
            }
            & $cmdName @cmdArgs
        }
    }
    finally {
        if ($WorkDir) { Pop-Location }
    }
}

# 1) Activate venv
$venvActivate = Join-Path $repoRoot "venv\Scripts\Activate.ps1"
if (Test-Path $venvActivate) {
    if (-not $DryRun) { . $venvActivate }
    else { Write-Host "[dry-run] source $venvActivate" -ForegroundColor Cyan }
} else {
    Write-Warning "venv not found at $venvActivate; using system Python"
}

# 2) Clean previous staged roots
function Invoke-CleanPath([string]$PathToClean) {
    Write-Host "==> Clean $PathToClean"
    if ($DryRun) {
        Write-Host "[dry-run] Remove-Item -LiteralPath `"$PathToClean`" -Recurse -Force" -ForegroundColor Cyan
        return
    }
    if (Test-Path -LiteralPath $PathToClean) {
        Remove-Item -LiteralPath $PathToClean -Recurse -Force -ErrorAction SilentlyContinue
    }
}
Invoke-CleanPath $buckyosRoot
Invoke-CleanPath $buckycliRoot

# 3) Build cyfs-gateway app (if repo present)
$cyfsSrc = Join-Path (Split-Path $repoRoot) "cyfs-gateway\src"
if (Test-Path $cyfsSrc) {
    Invoke-Step -Title "cyfs-gateway: cargo update" -Cmd @("cargo", "update") -WorkDir $cyfsSrc
    Invoke-Step -Title "cyfs-gateway: buckyos-build" -Cmd @("buckyos-build") -WorkDir $cyfsSrc
    Invoke-Step -Title "cyfs-gateway: buckyos-install -> $buckyosRoot" -Cmd @("buckyos-install", "--all", "--target-rootfs=$buckyosRoot", "--app=cyfs-gateway") -WorkDir $cyfsSrc
} else {
    Write-Warning "Skip cyfs-gateway (not found at $cyfsSrc)"
}

# 4) Build buckycli/buckyos apps
$buckySrc = Join-Path $repoRoot "src"
Invoke-Step -Title "buckyos: cargo update" -Cmd @("cargo", "update") -WorkDir $buckySrc
Invoke-Step -Title "buckycli: buckyos-build" -Cmd @("buckyos-build") -WorkDir $buckySrc
Invoke-Step -Title "buckycli: install -> $buckycliRoot" -Cmd @("buckyos-install", "--all", "--target-rootfs=$buckycliRoot", "--app=buckycli") -WorkDir $buckySrc
Invoke-Step -Title "buckyos: install -> $buckyosRoot" -Cmd @("buckyos-install", "--all", "--target-rootfs=$buckyosRoot", "--app=buckyos") -WorkDir $buckySrc
Invoke-Step -Title "buckyos: make_config release" -Cmd @("python", "make_config.py", "release", "--rootfs=$buckyosRoot") -WorkDir $buckySrc

# 5) Build Windows MSI
$outDir = Join-Path $repoRoot "publish"
$workRoot = Join-Path $BuildRoot "win-msi\distbuild"
$msiScript = Join-Path $repoRoot "src\publish\make_local_win_installer.py"
#Invoke-Step -Title "Build MSI ($Arch, $Version)" -Cmd @("python", $msiScript, "build-pkg", $Arch, $Version, "--work-root", $workRoot, "--out-dir", $outDir, "--source", $buckyosRoot)

#Write-Host "Done. MSI should be in $outDir" -ForegroundColor Green
