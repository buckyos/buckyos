$ErrorActionPreference = "Continue"

# BEGIN AUTO-GENERATED: clean_paths

Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -Path (Join-Path $Root "cache")

Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -Path (Join-Path $Root "logs")

Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -Path (Join-Path $Root "tmp")

Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -Path (Join-Path $Root "local")

Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -Path (Join-Path $Root "etc")

# END AUTO-GENERATED: clean_paths

