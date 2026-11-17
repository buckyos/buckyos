#!/usr/bin/env python3
"""Build the payloads of ext-fonts, the font package (section 6.4 of the paios container requirements, table C).

The package carries no font. Each font family is an online item of ext-manifest.json: a node downloads the upstream's
own release archive into <tools>/online/<item>/ when an administrator asks for it. What the payload holds is the
manifest, store/ext-fonts/fonts.conf, which tells fontconfig where those items keep their fonts (ext-core's fontconfig
takes it in), and a notice. Every platform has the same payload but for the line of the notice that names it (a PIKG
takes no two sub-packages with one digest), and a build gives the same bytes every time.

  build.py                       payload/<platform>/
  build.py --archive             and dist/ext-fonts-<version>-<platform>.tar.gz
  build.py release --pikg <file> dist/manifest.json of a release: per platform the file, sha256, size and R2 path
"""
from __future__ import annotations

import argparse
import gzip
import hashlib
import io
import json
from pathlib import Path
import re
import shutil
import tarfile

HERE = Path(__file__).resolve().parent
MANIFEST = HERE / "ext-manifest.json"
FONTS_CONF = HERE / "fonts.conf"
PACKAGE = "ext-fonts"
SCHEMA = "buckyos.ext-tools/v1"
# fonts.conf lives at <tools>/versions/ext-fonts/<dir>/store/ext-fonts/: <tools> is five levels up from there.
ITEM_DIR = re.compile(r'<dir prefix="relative">\.\./\.\./\.\./\.\./\.\./online/([a-z0-9-]+)/([^<]+)</dir>')


def manifest() -> dict:
    return json.loads(MANIFEST.read_text(encoding="utf-8"))


def font_dirs(text: str) -> dict[str, str]:
    """Online item -> the directory inside it that fonts.conf points fontconfig at."""
    return {item: path for item, path in ITEM_DIR.findall(text)}


def notice(m: dict, platform: str) -> str:
    lines = [
        f"# ext-fonts {m['version']} for {platform}",
        "",
        "This package carries no font file. Each font below is an online item: a node downloads it, when an administrator",
        "asks for it, from the address given here, which is the upstream's own release on GitHub, and checks it against the",
        "SHA256 given here. BuckyOS does not distribute these fonts. Each comes under its own license, all of which allow",
        "embedding the font in a PDF.",
        "",
        "The files of this package (ext-manifest.json, fonts.conf and this notice) are BuckyOS's, AGPL-3.0-only.",
        "",
    ]
    for item in m["online"]:
        source = next(iter(item["platforms"].values()))
        lines += [f"## {item['id']} {item['version']}", "", item["desc"], "", f"- {source['url']}", f"- sha256 {source['sha256']}", f"- {item['license']}", ""]
    return "\n".join(lines)


def write_payload(dest: Path, platform: str) -> None:
    m = manifest()
    if dest.exists():
        shutil.rmtree(dest)
    store = dest / "store" / PACKAGE
    store.mkdir(parents=True)
    shutil.copyfile(MANIFEST, dest / "ext-manifest.json")
    shutil.copyfile(FONTS_CONF, store / "fonts.conf")
    (store / "NOTICE.md").write_text(notice(m, platform), encoding="utf-8")


def write_tar(payload: Path, target: Path) -> None:
    """Sorted members, no owners and no times, so that the same payload is the same tar."""
    data = io.BytesIO()
    with tarfile.open(fileobj=data, mode="w", format=tarfile.PAX_FORMAT) as tar:
        for path in sorted(payload.rglob("*")):
            info = tar.gettarinfo(str(path), arcname=path.relative_to(payload).as_posix())
            info.uid = info.gid = 0
            info.uname = info.gname = ""
            info.mtime = 0
            info.mode = 0o755 if info.isdir() else 0o644
            if info.isfile():
                with path.open("rb") as handle:
                    tar.addfile(info, handle)
            else:
                tar.addfile(info)
    with target.open("wb") as raw, gzip.GzipFile(filename="", fileobj=raw, mode="wb", compresslevel=9, mtime=0) as gz:
        gz.write(data.getvalue())


def sha256_of(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def tar_name(version: str, platform: str) -> str:
    return f"{PACKAGE}-{version}-{platform}.tar.gz"


def release(dist: Path, pikg: Path | None) -> dict:
    m = manifest()
    version = m["version"]
    platforms = {}
    for platform in m["platforms"]:
        tar = dist / tar_name(version, platform)
        if not tar.is_file():
            raise SystemExit(f"{tar}: missing; run build.py --archive first")
        sha = sha256_of(tar)
        platforms[platform] = {"file": tar.name, "sha256": f"sha256:{sha}", "size": tar.stat().st_size, "path": f"ext-tools/{PACKAGE}/{version}/{platform}/{sha}.tar.gz"}
    package = {"version": version, "manifest": m, "platforms": platforms}
    if pikg is not None:
        sha = sha256_of(pikg)
        package["pikg_digest"] = f"sha256:{sha}"
        package["pikg"] = {"file": pikg.name, "sha256": f"sha256:{sha}", "size": pikg.stat().st_size, "path": f"ext-tools/{PACKAGE}/{version}/{pikg.name}"}
    return {"schema": SCHEMA, "packages": {PACKAGE: package}}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("command", nargs="?", choices=["payload", "release"], default="payload")
    parser.add_argument("--archive", action="store_true")
    parser.add_argument("--out", type=Path, default=HERE / "payload")
    parser.add_argument("--dist", type=Path, default=HERE / "dist")
    parser.add_argument("--pikg", type=Path)
    args = parser.parse_args(argv)
    m = manifest()
    if args.command == "release":
        (args.dist / "manifest.json").write_text(json.dumps(release(args.dist, args.pikg), indent=2) + "\n")
        return 0
    for platform in m["platforms"]:
        write_payload(args.out / platform, platform)
        if args.archive:
            args.dist.mkdir(parents=True, exist_ok=True)
            write_tar(args.out / platform, args.dist / tar_name(m["version"], platform))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
