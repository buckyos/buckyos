#!/usr/bin/env python3
"""dist/manifest.json of an ext-core release: per platform the file, sha256, size and R2 path (ExtTools TODO §3.10)."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import tarfile

from compliance import ComplianceError, SCHEMA as SOURCE_SCHEMA, safe_relative

PLATFORMS = ["linux-x86_64", "linux-aarch64", "darwin-arm64", "darwin-x86_64", "windows-x86_64"]
SCHEMA = "buckyos.ext-tools/v1"


def sha256_of(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def archive_members(archive: tarfile.TarFile) -> dict[str, tarfile.TarInfo]:
    result = {}
    for member in archive.getmembers():
        name = member.name.removeprefix("./").rstrip("/")
        if not name or name == ".":
            continue
        safe_relative(name)
        if name in result:
            raise ComplianceError(f"archive contains duplicate member: {name}")
        result[name] = member
    return result


def read_json(archive: tarfile.TarFile, members: dict, name: str) -> tuple[dict, str]:
    member = members.get(name)
    if member is None or not member.isfile() or member.size > 32 * 1024 * 1024:
        raise ComplianceError(f"missing or oversized compliance manifest: {name}")
    handle = archive.extractfile(member)
    if handle is None:
        raise ComplianceError(f"cannot read compliance manifest: {name}")
    data = handle.read()
    return json.loads(data), hashlib.sha256(data).hexdigest()


def verify_inventory(archive: tarfile.TarFile, members: dict, inventory: list[dict]) -> None:
    # Read compressed archives in their physical order. Lexical path order can
    # otherwise force repeated decompression of multi-gigabyte tool payloads.
    ordered = sorted(inventory, key=lambda entry: members.get(safe_relative(entry["path"]).as_posix()).offset
                     if safe_relative(entry["path"]).as_posix() in members else -1)
    hashes = {}
    for entry in ordered:
        member = members.get(safe_relative(entry["path"]).as_posix())
        if member is None:
            raise ComplianceError(f'archive is missing required file: {entry["path"]}')
        if "symlink" in entry:
            if not member.issym() or member.linkname != entry["symlink"]:
                raise ComplianceError(f'archive symlink changed: {entry["path"]}')
            continue
        if not (member.isfile() or member.islnk()):
            raise ComplianceError(f'archive file is not regular: {entry["path"]}')
        previous = hashes.get(member.linkname.removeprefix("./")) if member.islnk() else None
        if previous is None:
            handle = archive.extractfile(member)
            if handle is None:
                raise ComplianceError(f'cannot read archive member: {entry["path"]}')
            digest = hashlib.sha256()
            size = 0
            for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                digest.update(chunk)
                size += len(chunk)
            previous = (digest.hexdigest(), size)
        hashes[member.name.removeprefix("./")] = previous
        if previous != (entry["sha256"], entry["size"]):
            raise ComplianceError(f'archive file hash/size changed: {entry["path"]}')


def corresponding_source(payload: Path, dist: Path, version: str, platform: str) -> dict:
    prefix = "store/ext-core/LICENSES/"
    with tarfile.open(payload, "r:gz") as tar:
        members = archive_members(tar)
        reference, _ = read_json(tar, members, prefix + "corresponding-source.json")
        manifest, manifest_digest = read_json(tar, members, prefix + "source-manifest.json")
        if manifest.get("schema") != SOURCE_SCHEMA or manifest.get("version") != version or manifest.get("platform") != platform:
            raise ComplianceError(f"{payload}: source manifest identity mismatch")
        if reference.get("manifest_sha256") != "sha256:" + manifest_digest:
            raise ComplianceError(f"{payload}: source manifest digest mismatch")
        for component in manifest.get("components", []):
            if component.get("unresolved_source_requirements") or component.get("source_coverage") in {"partial", "incomplete", "unverified"}:
                raise ComplianceError(f'{payload}: incomplete corresponding source for {component["id"]}')
        inventory = manifest.get("payload_files")
        if not isinstance(inventory, list) or not inventory:
            raise ComplianceError(f"{payload}: missing payload file inventory")
        verify_inventory(tar, members, inventory)
        license_inventory = []
        for entry in manifest.get("files", []):
            if entry["path"].startswith("LICENSES/"):
                license_inventory.append({**entry, "path": prefix + entry["path"].removeprefix("LICENSES/")})
        if not license_inventory:
            raise ComplianceError(f"{payload}: missing license file inventory")
        verify_inventory(tar, members, license_inventory)
        expected = {entry["path"] for entry in inventory}
        extra = [name for name, member in members.items() if not member.isdir() and not name.startswith(prefix) and name not in expected]
        if extra:
            raise ComplianceError(f"{payload}: unrecorded payload files: {extra[:5]}")
    filename = f"ext-core-{version}-{platform}-corresponding-source.tar.gz"
    if reference.get("file") != filename:
        raise ComplianceError(f"{payload}: unexpected corresponding-source filename")
    source = dist / filename
    if not source.is_file():
        raise ComplianceError(f"corresponding source must accompany the binary: {source}")
    digest = sha256_of(source)
    if reference.get("sha256") != "sha256:" + digest or reference.get("size") != source.stat().st_size:
        raise ComplianceError(f"{source}: corresponding-source archive digest/size mismatch")
    expected_path = f"ext-tools/ext-core/{version}/{platform}/{digest}.source.tar.gz"
    if reference.get("path") != expected_path:
        raise ComplianceError(f"{source}: corresponding-source release path mismatch")
    with tarfile.open(source, "r:gz") as tar:
        members = archive_members(tar)
        source_manifest, source_digest = read_json(tar, members, "source-manifest.json")
        if source_digest != manifest_digest or source_manifest != manifest:
            raise ComplianceError(f"{source}: source manifest differs from the binary payload")
        inventory = manifest.get("files")
        if not isinstance(inventory, list) or not inventory:
            raise ComplianceError(f"{source}: missing corresponding-source file inventory")
        verify_inventory(tar, members, inventory)
        expected = {entry["path"] for entry in inventory}
        for component in manifest["components"]:
            for item in component.get("sources", []):
                entry = next((row for row in inventory if row["path"] == item["file"]), None)
                if entry is None or entry.get("sha256") != item["sha256"]:
                    raise ComplianceError(f'{source}: unverified source artifact {item["file"]}')
        if any(not item.isdir() and name not in expected and name != "source-manifest.json" for name, item in members.items()):
            raise ComplianceError(f"{source}: unrecorded corresponding-source files")
    return reference


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
            "corresponding_source": corresponding_source(tar, dist, version, platform),
        }
    if not platforms:
        raise ComplianceError(f"{dist}: no payload archives for ext-core {version}")
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
