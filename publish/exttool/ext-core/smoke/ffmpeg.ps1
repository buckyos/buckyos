$ErrorActionPreference = 'Continue'
$dir = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid())
New-Item -ItemType Directory -Path $dir | Out-Null
try {
  & ffmpeg -hide_banner -loglevel error -f lavfi -i testsrc=size=16x16:rate=1:duration=1 -y "$dir\clip.mkv"
  if ($LASTEXITCODE -ne 0) { exit 1 }
  $streams = & ffprobe -hide_banner -loglevel error -show_entries stream=codec_type -of csv=p=0 "$dir\clip.mkv"
  if ($LASTEXITCODE -ne 0 -or ($streams -notmatch 'video')) { exit 1 }
  $fonts = & ffmpeg -hide_banner -loglevel warning -f lavfi -i color=size=16x16:rate=1:duration=1 -vf drawtext=text=x -f null - 2>&1
  if (($fonts | Out-String) -match 'Cannot load default config') { exit 1 }
} finally { Remove-Item -Recurse -Force $dir }
