#!/bin/sh
set -eu

mkdir -p /www
cat > /www/index.html <<'EOF'
<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <title>Docker Fixture</title>
  </head>
  <body>
    <h1>Docker Fixture</h1>
    <p>Served from a locally published image tar.</p>
  </body>
</html>
EOF

exec busybox httpd -f -p 80 -h /www
