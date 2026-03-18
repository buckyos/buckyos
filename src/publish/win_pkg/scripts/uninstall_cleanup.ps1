$ErrorActionPreference = "Continue"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$Root = Split-Path -Parent $ScriptDir
$DefaultsDir = Join-Path $Root ".buckyos_installer_defaults"

Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -Path $DefaultsDir

# BEGIN AUTO-GENERATED: clean_paths
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -Path (Join-Path $Root "data\var")
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -Path (Join-Path $Root "data\cache")
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -Path (Join-Path $Root "local")
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -Path (Join-Path $Root "logs")
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -Path (Join-Path $Root "etc")
# END AUTO-GENERATED: clean_paths
# BEGIN AUTO-GENERATED: modules
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -Path (Join-Path $Root "bin\node-daemon")
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -Path (Join-Path $Root "bin\system-config")
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -Path (Join-Path $Root "bin\verify-hub")
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -Path (Join-Path $Root "bin\scheduler")
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -Path (Join-Path $Root "bin\task-manager")
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -Path (Join-Path $Root "bin\kmsg")
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -Path (Join-Path $Root "bin\control-panel")
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -Path (Join-Path $Root "bin\control-panel\web")
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -Path (Join-Path $Root "bin\smb-service")
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -Path (Join-Path $Root "bin\repo-service")
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -Path (Join-Path $Root "bin\buckycli")
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -Path (Join-Path $Root "bin\node-active")
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -Path (Join-Path $Root "bin\buckyos_systest")
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -Path (Join-Path $Root "bin\aicc")
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -Path (Join-Path $Root "bin\msg-center")
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -Path (Join-Path $Root "bin\opendan")
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -Path (Join-Path $Root "etc\scheduler")
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -Path (Join-Path $Root "etc\boot_gateway.yaml")
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -Path (Join-Path $Root "bin\util.py")
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -Path (Join-Path $Root "bin\stop.py")
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -Path (Join-Path $Root "bin\buckyos_jarvis")
# END AUTO-GENERATED: modules
