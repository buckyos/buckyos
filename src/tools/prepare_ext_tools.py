#!/usr/bin/env python3
"""The seed payload of ext-core for this machine: rootfs/bin/ext-tools/ (ExtTools TODO §3.10, T2d).

node_daemon unpacks it into $BUCKYOS_ROOT/tools the first time, on a developer's machine as on an installed one. This
script only gets the file there, and only when it is not there yet:

  prepare_ext_tools.py                    the version publish/exttool/ext-core/ext-manifest.json pins, from R2
  prepare_ext_tools.py --latest           the newest release instead
  prepare_ext_tools.py --force            download again
  prepare_ext_tools.py --local <payload>  package a payload built by publish/exttool/ext-core/build.sh; delete
                                          $BUCKYOS_ROOT/tools/installs/ext-core.json and restart to install it
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform as host_platform
import shutil
import sys
import tarfile
import urllib.error
import urllib.request
from pathlib import Path

SRC_DIR = Path(__file__).resolve().parent.parent
MANIFEST = SRC_DIR.parent / "publish" / "exttool" / "ext-core" / "ext-manifest.json"
DEFAULT_OUT = SRC_DIR / "rootfs" / "bin" / "ext-tools"
PIN_FILE = "ext_tools.json"
PIN_SCHEMA = "buckyos.ext-tools/v1"
R2_BASE = os.environ.get("BUCKYOS_EXT_TOOLS_BASE_URL", "https://buckyos-installer.tbudr.top")
LATEST_URL = os.environ.get("BUCKYOS_EXT_TOOLS_LATEST_URL", f"{R2_BASE}/ext-tools/ext-core/latest.json")


def current_platform() -> str:
    machine = host_platform.machine().lower()
    arch = {"amd64": "x86_64", "x86_64": "x86_64", "arm64": "aarch64", "aarch64": "aarch64"}.get(machine)
    if arch is None:
        raise SystemExit(f"no extension packages for {machine}")
    if sys.platform == "win32":
        return "windows-x86_64"
    if sys.platform == "darwin":
        return "darwin-arm64" if arch == "aarch64" else "darwin-x86_64"
    return f"linux-{arch}"


def pinned_version() -> str:
    return json.loads(MANIFEST.read_text())["version"]


def sha256_of(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return f"sha256:{digest.hexdigest()}"


def ready(out: Path, version: str | None, platform: str) -> bool:
    """Is the tar of this platform here, whole, and of the wanted version (`None`: any)?"""
    try:
        pin = json.loads((out / PIN_FILE).read_text())
        package = pin["packages"]["ext-core"]
        entry = package["platforms"][platform]
    except (OSError, ValueError, KeyError):
        return False
    if version is not None and package.get("version") != version:
        return False
    tar = out / entry["file"]
    return tar.is_file() and tar.stat().st_size == entry["size"]


def write_pin(out: Path, release: dict, platform: str, tar_name: str) -> None:
    """Only this machine's platform: the pin says what is in the directory, not what the release has."""
    package = dict(release["packages"]["ext-core"])
    package["platforms"] = {platform: {**package["platforms"][platform], "file": tar_name}}
    (out / PIN_FILE).write_text(json.dumps({"schema": PIN_SCHEMA, "packages": {"ext-core": package}}, indent=2) + "\n")


def request_of(url: str) -> urllib.request.Request:
    # The host is behind Cloudflare, which refuses urllib's default User-Agent (403, error 1010).
    return urllib.request.Request(url, headers={"Cache-Control": "no-cache", "User-Agent": "buckyos-ext-tools/1"})


def fetch_json(url: str) -> dict:
    with urllib.request.urlopen(request_of(url), timeout=60) as response:
        return json.loads(response.read())


def download(url: str, dest: Path, size: int, sha256: str) -> None:
    temp = dest.with_suffix(dest.suffix + ".part")
    with urllib.request.urlopen(request_of(url), timeout=600) as response, temp.open("wb") as handle:
        while True:
            block = response.read(1024 * 1024)
            if not block:
                break
            handle.write(block)
    if temp.stat().st_size != size or sha256_of(temp) != sha256:
        temp.unlink()
        raise SystemExit(f"{url}: not the file the release lists (size or sha256 differs)")
    temp.replace(dest)


def from_release(out: Path, version: str, platform: str) -> None:
    release = fetch_json(f"{R2_BASE}/ext-tools/ext-core/{version}/manifest.json")
    entry = release["packages"]["ext-core"]["platforms"].get(platform)
    if entry is None:
        raise SystemExit(f"ext-core {version} has no payload for {platform}")
    out.mkdir(parents=True, exist_ok=True)
    for old in out.glob("ext-core-*.tar.gz"):
        old.unlink()
    print(f"Downloading ext-core {version} for {platform} ({entry['size'] // (1024 * 1024)} MiB)", flush=True)
    download(f"{R2_BASE}/{entry['path']}", out / entry["file"], entry["size"], entry["sha256"])
    write_pin(out, release, platform, entry["file"])


def from_local(out: Path, payload: Path, platform: str) -> None:
    manifest = json.loads((payload / "ext-manifest.json").read_text())
    if platform not in manifest["platforms"]:
        raise SystemExit(f"{payload} does not list {platform}")
    out.mkdir(parents=True, exist_ok=True)
    for old in out.glob("ext-core-*.tar.gz"):
        old.unlink()
    name = f"ext-core-{manifest['version']}-{platform}.tar.gz"
    with tarfile.open(out / name, "w:gz", compresslevel=6) as archive:
        for child in sorted(payload.iterdir()):
            archive.add(child, arcname=child.name)
    entry = {"file": name, "sha256": sha256_of(out / name), "size": (out / name).stat().st_size}
    release = {"packages": {"ext-core": {"version": manifest["version"], "manifest": manifest, "platforms": {platform: entry}}}}
    write_pin(out, release, platform, name)
    print(f"{out / name}: {entry['size'] // (1024 * 1024)} MiB, {entry['sha256']}")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--latest", action="store_true")
    parser.add_argument("--force", action="store_true")
    parser.add_argument("--local", type=Path)
    parser.add_argument("--platform", default=None)
    parser.add_argument("--out", type=Path, default=DEFAULT_OUT)
    args = parser.parse_args(argv)
    platform = args.platform or current_platform()
    if args.local:
        from_local(args.out, args.local, platform)
        return 0
    version = fetch_json(LATEST_URL)["version"] if args.latest else pinned_version()
    if not args.force and ready(args.out, version, platform):
        return 0
    # A seed image built on a developer's machine takes what that machine already prepared.
    if not args.force and args.out != DEFAULT_OUT and ready(DEFAULT_OUT, version, platform):
        args.out.mkdir(parents=True, exist_ok=True)
        pin = json.loads((DEFAULT_OUT / PIN_FILE).read_text())
        shutil.copy2(DEFAULT_OUT / pin["packages"]["ext-core"]["platforms"][platform]["file"], args.out)
        shutil.copy2(DEFAULT_OUT / PIN_FILE, args.out)
        return 0
    try:
        from_release(args.out, version, platform)
    except (urllib.error.URLError, OSError, ValueError) as error:
        print(f"ext-core {version} for {platform} is not available: {error}", file=sys.stderr)
        print("Build it with publish/exttool/ext-core/build.sh and pass the payload with --local.", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
