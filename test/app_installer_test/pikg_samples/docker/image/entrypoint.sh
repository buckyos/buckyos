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
    <p>Built and packaged locally with buckyos-tool.</p>
  </body>
</html>
EOF

if busybox --list | grep -qx httpd; then
  exec busybox httpd -f -p 80 -h /www
fi

while true; do
  {
    printf 'HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n'
    cat /www/index.html
  } | busybox nc -l -p 80
done
