#!/usr/bin/env python3
"""Put the online items of an ext-core payload where a node has them (`online` of ext-manifest.json).

A node downloads an online item once the package is installed. This does the same from the manifest alone, for the
build's check of the browser against the payload's libraries and for the smoke tests of CI:

    place_online.py --payload payload/linux-x86_64 --tools <tools> --platform linux-x86_64

<tools>/online/<id>/ gets the archive of the manifest's `url`, checked against `sha256` and `size_bytes`, unpacked into
`into` with the `rename`s done and the payload's `overlay` directory laid over it; <tools>/online/<id>.json is the record.
--archive <id>=<file> takes a file that is already here instead of the address, checked the same way.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import sys
import tarfile
import time
import urllib.request
import zipfile

RECORD_SCHEMA = "buckyos.ext_tools.online/1"


class PlaceError(RuntimeError):
    pass


def relative(value, what):
    path = PurePosixPath(value)
    if not value or path.is_absolute() or "\\" in value or any(part in ("..", ".") for part in path.parts):
        raise PlaceError(f"{what}: not a path inside the item: {value!r}")
    return Path(*path.parts)


def sha256_of(path):
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def download(url, target):
    request = urllib.request.Request(url, headers={"User-Agent": "buckyos-ext-tools/1"})
    for attempt in range(3):
        try:
            with urllib.request.urlopen(request, timeout=120) as response, target.open("wb") as handle:
                shutil.copyfileobj(response, handle, length=1024 * 1024)
            return
        except OSError:
            if attempt == 2:
                raise
            time.sleep(2 * (attempt + 1))


def unzip(archive, dest):
    """Modes and symbolic links as the archive has them: a macOS app bundle is full of both."""
    root = dest.resolve()
    with zipfile.ZipFile(archive) as handle:
        for member in handle.infolist():
            target = dest / relative(member.filename.rstrip("/"), member.filename)
            mode = member.external_attr >> 16
            if member.is_dir():
                target.mkdir(parents=True, exist_ok=True)
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            if stat.S_ISLNK(mode):
                link = handle.read(member).decode()
                if os.path.isabs(link) or root not in (target.parent / link).resolve().parents:
                    raise PlaceError(f"{member.filename}: a link out of the archive: {link}")
                target.symlink_to(link)
                continue
            with handle.open(member) as source, target.open("wb") as out:
                shutil.copyfileobj(source, out, length=1024 * 1024)
            if mode & 0o777:
                target.chmod(mode & 0o777)


def unpack(archive, kind, dest):
    dest.mkdir(parents=True)
    if kind == "zip":
        unzip(archive, dest)
    elif kind == "tar.gz":
        with tarfile.open(archive, "r:gz") as handle:
            handle.extractall(dest, filter="tar")
    else:
        raise PlaceError(f"format: zip or tar.gz, not {kind!r}")


def overlay(source, dest):
    if not source.is_dir():
        raise PlaceError(f"overlay: no such directory in the payload: {source}")
    for path in sorted(source.rglob("*")):
        target = dest / path.relative_to(source)
        if path.is_symlink():
            raise PlaceError(f"overlay: a link: {path}")
        if path.is_dir():
            target.mkdir(parents=True, exist_ok=True)
        else:
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(path, target)


def place(payload, tools, platform, item, archive=None):
    """Returns the record, or None when the item has nothing for `platform`."""
    manifest = json.loads((payload / "ext-manifest.json").read_text(encoding="utf-8"))
    found = [entry for entry in manifest.get("online", []) if entry["id"] == item]
    if not found or not re.fullmatch(r"[a-z0-9-]+", item):
        raise PlaceError(f"{manifest['id']} declares no online item {item!r}")
    entry = found[0]["platforms"].get(platform)
    if entry is None:
        return None
    online = tools / "online"
    final = online / item
    layer = payload / relative(entry["overlay"], "overlay") if entry.get("overlay") else None
    if layer is not None and layer.resolve() == final.resolve():
        raise PlaceError(f"{payload} is unpacked as the tools directory itself: its {entry['overlay']} is where the item goes")
    online.mkdir(parents=True, exist_ok=True)
    staged = online / f".{item}.staged"
    fetched = online / f".{item}.download"
    for leftover in (staged, fetched):
        if leftover.is_dir():
            shutil.rmtree(leftover)
        else:
            leftover.unlink(missing_ok=True)
    try:
        if archive is None:
            download(entry["url"], fetched)
            archive = fetched
        if archive.stat().st_size != entry["size_bytes"]:
            raise PlaceError(f"{item}: {archive.stat().st_size} bytes, the manifest says {entry['size_bytes']}")
        if sha256_of(archive) != entry["sha256"]:
            raise PlaceError(f"{item}: the archive's sha256 is not the manifest's {entry['sha256']}")
        unpack(archive, entry["format"], staged / relative(entry["into"], "into") if entry.get("into") else staged)
        for source, target in entry.get("rename", []):
            source, target = staged / relative(source, "rename"), staged / relative(target, "rename")
            if not source.exists() or target.exists():
                raise PlaceError(f"{item}: cannot rename {source.relative_to(staged)} to {target.relative_to(staged)}")
            source.rename(target)
        if layer is not None:
            overlay(layer, staged)
        if final.exists():
            shutil.rmtree(final)
        staged.rename(final)
    finally:
        fetched.unlink(missing_ok=True)
        if staged.exists():
            shutil.rmtree(staged)
    record = {
        "schema": RECORD_SCHEMA, "id": item, "version": found[0]["version"], "package": manifest["id"], "platform": platform,
        "sha256": entry["sha256"], "state": "ready", "size_bytes": entry["size_bytes"], "downloaded_bytes": entry["size_bytes"],
        "error": None, "updated_at_ms": int(time.time() * 1000),
    }
    (online / f"{item}.json").write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--payload", type=Path, required=True, help="an unpacked payload: the directory of ext-manifest.json")
    parser.add_argument("--tools", type=Path, required=True, help="the tools directory: online/ is made in it")
    parser.add_argument("--platform", required=True)
    parser.add_argument("--item", action="append", help="default: every online item of the manifest")
    parser.add_argument("--archive", action="append", default=[], metavar="ID=FILE")
    args = parser.parse_args()
    archives = dict(pair.split("=", 1) for pair in args.archive)
    try:
        manifest = json.loads((args.payload / "ext-manifest.json").read_text(encoding="utf-8"))
        for item in args.item or [entry["id"] for entry in manifest.get("online", [])]:
            record = place(args.payload, args.tools, args.platform, item, Path(archives[item]) if item in archives else None)
            if record is None:
                print(f"[place_online] {item}: nothing for {args.platform}")
            else:
                print(f"[place_online] {item} {record['version']}: {args.tools / 'online' / item}")
    except (PlaceError, OSError, KeyError, ValueError, zipfile.BadZipFile, tarfile.TarError) as error:
        print(f"place_online: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
