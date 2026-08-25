@echo off
setlocal
deno run --quiet --allow-env --allow-run=deno "%~dp0win_launcher.ts" %*
exit /b %ERRORLEVEL%
