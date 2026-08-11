$ErrorActionPreference = "Stop"

$HookDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ScriptsDir = Split-Path -Parent $HookDir
$Root = Split-Path -Parent $ScriptsDir

$TaskName = "BuckyOSNodeDaemonKeepAlive"
$Loader = Join-Path $ScriptsDir "node_daemon_loader.vbs"
$NodeDaemon = Join-Path $Root "bin\node-daemon\node_daemon.exe"
$RunCommand = 'wscript.exe //B //NoLogo "{0}" "{1}"' -f $Loader, $NodeDaemon

& schtasks.exe /Delete /TN $TaskName /F *> $null

& schtasks.exe /Create /TN $TaskName /SC MINUTE /MO 1 /F /TR $RunCommand
if ($LASTEXITCODE -ne 0) {
  exit $LASTEXITCODE
}

& schtasks.exe /Run /TN $TaskName
if ($LASTEXITCODE -ne 0) {
  exit $LASTEXITCODE
}

$RunKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
New-Item -Path $RunKey -Force | Out-Null
Set-ItemProperty -Path $RunKey -Name "BuckyOSDaemon" -Value $RunCommand
