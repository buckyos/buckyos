#!/usr/bin/env python3
"""
Build a minimal macOS .pkg that installs and runs a LaunchDaemon to test Docker
availability from the system (root) domain.

The installed daemon runs once at load and writes logs to:
- /var/log/buckyos.docker-test.out.log
- /var/log/buckyos.docker-test.err.log

Build requirements:
- macOS with Xcode command line tools (pkgbuild)
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import tempfile
from pathlib import Path


DAEMON_LABEL = "com.buckyos.docker-test"
PLIST_PATH = f"/Library/LaunchDaemons/{DAEMON_LABEL}.plist"
RUN_SCRIPT_PATH = "/usr/local/lib/buckyos/docker-test/run.sh"
OUT_LOG = "/var/log/buckyos.docker-test.out.log"
ERR_LOG = "/var/log/buckyos.docker-test.err.log"


def _run(cmd: list[str]) -> None:
    subprocess.run(cmd, check=True)


def _write_text(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def build_pkg(*, out_pkg: Path, version: str) -> None:
    with tempfile.TemporaryDirectory(prefix="buckyos-docker-test-pkg-") as td:
        td_path = Path(td)
        root_dir = td_path / "root"
        scripts_dir = td_path / "scripts"

        # Payload files
        plist_file = root_dir / PLIST_PATH.lstrip("/")
        run_file = root_dir / RUN_SCRIPT_PATH.lstrip("/")

        _write_text(
            plist_file,
            f"""<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{DAEMON_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>/bin/zsh</string>
    <string>{RUN_SCRIPT_PATH}</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>PATH</key>
    <string>/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin</string>
  </dict>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <false/>
  <key>StandardOutPath</key>
  <string>{OUT_LOG}</string>
  <key>StandardErrorPath</key>
  <string>{ERR_LOG}</string>
</dict>
</plist>
""",
        )

        _write_text(
            run_file,
            """#!/bin/zsh
set -u

OUT_LOG="/var/log/buckyos.docker-test.out.log"
ERR_LOG="/var/log/buckyos.docker-test.err.log"

{
  echo "==== buckyos docker test (LaunchDaemon) ===="
  date
  echo "whoami: $(whoami)"
  echo "id: $(id)"
  echo "pwd: $(pwd)"
  echo "HOME: ${HOME:-}"
  echo "PATH: ${PATH:-}"
  echo ""

  echo "== docker binary =="
  command -v docker || true
  docker --version || true
  echo ""

  echo "== docker contexts =="
  docker context ls || true
  echo ""

  echo "== docker info =="
  docker info || true
  echo ""

  echo "== docker ps =="
  docker ps || true
  echo ""

  echo "== socket hints =="
  ls -l /var/run/docker.sock 2>/dev/null || true
  ls -l "/Users" 2>/dev/null | head -n 10 || true
  echo "==== end ===="
} >>"$OUT_LOG" 2>>"$ERR_LOG"
""",
        )

        # Installer scripts (run as root during install)
        postinstall = scripts_dir / "postinstall"
        _write_text(
            postinstall,
            f"""#!/bin/zsh
set -e

PLIST="{PLIST_PATH}"
LABEL="{DAEMON_LABEL}"
RUN="{RUN_SCRIPT_PATH}"

echo "[docker-test] ensuring permissions"
chown root:wheel "$PLIST" "$RUN" || true
chmod 644 "$PLIST" || true
chmod 755 "$RUN" || true

echo "[docker-test] (re)load daemon"
launchctl bootout system "$PLIST" >/dev/null 2>&1 || true
launchctl bootstrap system "$PLIST"
launchctl enable "system/$LABEL" >/dev/null 2>&1 || true
launchctl kickstart -k "system/$LABEL" || true

echo "[docker-test] installed. check logs:"
echo "  {OUT_LOG}"
echo "  {ERR_LOG}"
""",
        )
        os.chmod(postinstall, 0o755)

        out_pkg.parent.mkdir(parents=True, exist_ok=True)
        if out_pkg.exists():
            out_pkg.unlink()

        _run(
            [
                "pkgbuild",
                "--root",
                str(root_dir),
                "--scripts",
                str(scripts_dir),
                "--identifier",
                "com.buckyos.docker-test.pkg",
                "--version",
                version,
                "--install-location",
                "/",
                str(out_pkg),
            ]
        )


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--version", default="0.0.1", help="pkg version (default: 0.0.1)")
    ap.add_argument(
        "--out",
        default=str(Path.cwd() / "publish" / "buckyos-docker-daemon-test.pkg"),
        help="output pkg path (default: ./publish/buckyos-docker-daemon-test.pkg)",
    )
    args = ap.parse_args()

    out_pkg = Path(args.out).expanduser().resolve()
    build_pkg(out_pkg=out_pkg, version=str(args.version))
    print(str(out_pkg))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

