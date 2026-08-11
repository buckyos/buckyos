$ErrorActionPreference = "Stop"

$TaskName = "BuckyOSNodeDaemonKeepAlive"

& schtasks.exe /Delete /TN $TaskName /F *> $null

$RunKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
Remove-ItemProperty -Path $RunKey -Name "BuckyOSDaemon" -ErrorAction SilentlyContinue
