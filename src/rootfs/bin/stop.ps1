$ErrorActionPreference = "Continue"

function Stop-BuckyProcess {
  param([string]$Name)

  try {
    $processes = Get-Process -Name $Name -ErrorAction SilentlyContinue
    if ($null -eq $processes) {
      Write-Host "$Name not running"
      return
    }

    $processes | Stop-Process -Force -ErrorAction SilentlyContinue
    Write-Host "$Name killed"
  } catch {
    Write-Host "$Name not running"
  }
}

$processNames = @(
  "node-daemon",
  "node_daemon",
  "scheduler",
  "verify-hub",
  "verify_hub",
  "system-config",
  "system_config",
  "cyfs-gateway",
  "cyfs_gateway",
  "filebrowser",
  "smb-service",
  "smb_service",
  "repo-service",
  "repo_service",
  "control-panel",
  "control_panel",
  "aicc",
  "task_manager",
  "kmsg",
  "msg_center",
  "opendan"
)

foreach ($name in $processNames) {
  Stop-BuckyProcess -Name $name
}
