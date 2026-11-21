@echo off
setlocal
set "ROOT=%~dp0.."
if not defined BUCKYOS_ROOT set "BUCKYOS_ROOT=%ROOT%"
"%ROOT%\bin\opendan\agent_tool.exe" xcall %*
