#!/usr/bin/env python3
"""dist/manifest.json of an ext-core release: per platform the file, sha256, size and R2 path (ExtTools TODO §3.10)."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

PLATFORMS = ["linux-x86_64", "linux-aarch64", "darwin-arm64", "darwin-x86_64", "windows-x86_64"]
SCHEMA = "buckyos.ext-tools/v1"


def sha256_of(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def build(version: str, dist: Path, manifest: dict, pikg: Path | None) -> dict:
    platforms = {}
    for platform in PLATFORMS:
        tar = dist / f"ext-core-{version}-{platform}.tar.gz"
        if not tar.is_file():
            continue
        sha = sha256_of(tar)
        platforms[platform] = {
            "file": tar.name,
            "sha256": f"sha256:{sha}",
            "size": tar.stat().st_size,
            "path": f"ext-tools/ext-core/{version}/{platform}/{sha}.tar.gz",
        }
    package = {"version": version, "manifest": manifest, "platforms": platforms}
    if pikg is not None and pikg.is_file():
        sha = sha256_of(pikg)
        package["pikg_digest"] = f"sha256:{sha}"
        package["pikg"] = {"file": pikg.name, "sha256": f"sha256:{sha}", "size": pikg.stat().st_size, "path": f"ext-tools/ext-core/{version}/{pikg.name}"}
    return {"schema": SCHEMA, "packages": {"ext-core": package}}


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--version", required=True)
    parser.add_argument("--dist", type=Path, required=True)
    parser.add_argument("--pikg", type=Path)
    args = parser.parse_args()
    here = Path(__file__).resolve().parent
    manifest = json.loads((here / "ext-manifest.json").read_text())
    release = build(args.version, args.dist, manifest, args.pikg)
    (args.dist / "manifest.json").write_text(json.dumps(release, indent=2) + "\n")


if __name__ == "__main__":
    main()
