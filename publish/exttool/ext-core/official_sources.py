#!/usr/bin/env python3
"""Collect pinned upstream materials for the official ext-core binaries."""
from __future__ import annotations

import argparse
import fnmatch
import hashlib
import os
import zipfile
from concurrent.futures import ThreadPoolExecutor
import json
from pathlib import Path
import re
import shutil
import tarfile
from urllib.parse import urlparse

from compliance import ComplianceError, cargo_sources, download, extract_notices, safe_relative, sha256, write_json

HERE = Path(__file__).resolve().parent


def validate_inputs(manifest: dict, build_script: Path, binary_pins: Path) -> None:
    if manifest.get("schema") != "buckyos.official-source-inputs/v1":
        raise ComplianceError("unsupported official source manifest")
    actual = dict(re.findall(r"^([A-Z_]+)=(\S+)$", build_script.read_text(), re.M))
    for key, value in manifest["build_versions"].items():
        if actual.get(key) != value:
            raise ComplianceError(f"{key} changed; update and verify official-sources.json before release")
    pins = {parts[1]: parts[0] for line in binary_pins.read_text().splitlines() if len(parts := line.split()) == 2}
    for component in manifest["components"]:
        for binary in component["binaries"].values():
            if pins.get(binary["file"]) != binary["sha256"]:
                raise ComplianceError(f"binary pin changed: {binary['file']}; update its source provenance")


def dependency_inputs(archive: Path, specification: dict) -> list[dict]:
    with tarfile.open(archive) as handle:
        member = handle.getmember(specification["member"])
        if not member.isfile() or member.size > 2_000_000:
            raise ComplianceError("invalid upstream dependency manifest")
        content = handle.extractfile(member).read().decode("utf-8")
    if specification.get("format") == "python-build-standalone-json":
        entries = []
        for name, item in json.loads(content).items():
            if name.startswith("cpython-") or "-bin" in name:
                continue
            if "library_names" not in item and "licenses" not in item:
                continue
            urls = [item["url"]]
            if item.get("upstream_url"):
                urls.append(item["upstream_url"])
            urls.extend(specification.get("mirrors", {}).get(name, []))
            entries.append({"file": Path(urlparse(urls[0]).path).name, "urls": urls,
                            "sha256": item["sha256"], "kind": "dependency-source",
                            "recipe_variable": name})
        return entries
    values = dict(re.findall(r"^([A-Z0-9_]+)\s*:?=\s*(.*?)\s*$", content, re.M))
    entries = []
    for key, value in values.items():
        if not key.endswith("_TARBALL"):
            continue
        for _ in range(10):
            resolved = re.sub(r"\$\(([A-Z0-9_]+)\)", lambda match: values.get(match[1], match[0]), value)
            if resolved == value:
                break
            value = resolved
        digest = values.get(key[:-8] + "_SHA256SUM", "")
        if "$" in value or not re.fullmatch(r"[0-9a-f]{64}", digest):
            raise ComplianceError(f"unresolved upstream dependency: {key}")
        safe_relative(value)
        entries.append({"file": value, "urls": [specification["url_base"] + value], "sha256": digest,
                        "kind": "upstream-build-tool" if value.endswith(".nupkg") else "dependency-source",
                        "recipe_variable": key})
    if not entries:
        raise ComplianceError("upstream dependency manifest contains no pinned sources")
    return entries


def verify_payload(payload: Path, binary: dict) -> dict:
    path = payload / "store/ext-core" / safe_relative(binary["payload_file"])
    if not path.is_file():
        raise ComplianceError(f"official component missing from payload: {path}")
    actual = sha256(path)
    original = Path(os.environ.get("EXT_CORE_CACHE", str(HERE / ".cache"))) / safe_relative(binary["file"])
    if not original.is_file() or sha256(original) != binary["sha256"]:
        raise ComplianceError(f"missing or changed original binary archive: {original}")
    result = {"file": binary["payload_file"], "sha256": actual,
              "original_archive_sha256_verified": True}
    if "archive_member" not in binary:
        result["verification"] = "original-build-input-verified; unpacked-payload-match-not-verified"
        result["note"] = "This installer format requires its platform unpacker. File presence/hash and the original archive pin do not prove that an independently supplied old payload was unpacked from this archive."
        return result
    pattern = binary["archive_member"]
    if pattern is None:
        expected = binary["sha256"]
    elif zipfile.is_zipfile(original):
        with zipfile.ZipFile(original) as handle:
            entries = [item for item in handle.infolist() if not item.is_dir() and fnmatch.fnmatchcase(item.filename, pattern)]
            if len(entries) != 1:
                raise ComplianceError(f"expected one executable matching {pattern} in {original}")
            with handle.open(entries[0]) as stream:
                expected = hashlib.file_digest(stream, "sha256").hexdigest()
    else:
        with tarfile.open(original) as handle:
            entries = [item for item in handle if item.isfile() and fnmatch.fnmatchcase(item.name, pattern)]
            if len(entries) != 1:
                raise ComplianceError(f"expected one executable matching {pattern} in {original}")
            with handle.extractfile(entries[0]) as stream:
                expected = hashlib.file_digest(stream, "sha256").hexdigest()
    if actual != expected:
        raise ComplianceError(f"payload executable differs from pinned upstream binary: {path}")
    result["verification"] = "selected-payload-file-matches-pinned-archive"
    return result


def package_provenance(component: dict, binary: dict, source_root: Path) -> dict | None:
    specification = component.get("package_provenance")
    if not specification:
        return None
    path = HERE / safe_relative(specification["file"])
    if sha256(path) != specification["sha256"]:
        raise ComplianceError("package provenance changed without updating its pin")
    provenance = json.loads(path.read_text())
    if provenance.get("schema") != "buckyos.mingit-package-sources/v1" or provenance["binary_sha256"] != binary["sha256"]:
        raise ComplianceError("MinGit source-package provenance does not match the binary")
    original = Path(os.environ.get("EXT_CORE_CACHE", str(HERE / ".cache"))) / safe_relative(binary["file"])
    with zipfile.ZipFile(original) as handle:
        contents = handle.read(provenance["binary_manifest_member"]).decode("utf-8")
    actual = {tuple(line.split()) for line in contents.splitlines() if line.strip()}
    expected = {(item["name"], item["version"]) for item in provenance["packages"]}
    if actual != expected:
        raise ComplianceError("MinGit bundled package list differs from source provenance")
    source_pins = {(item["file"], item["sha256"]) for item in component["sources"]}
    for item in provenance["packages"]:
        if (item["source_file"], item["source_sha256"]) not in source_pins:
            raise ComplianceError(f"MinGit package has no pinned source: {item['name']}")
    target = source_root / "provenance" / path.name
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(path, target)
    return {"file": str(target.relative_to(source_root)), "sha256": specification["sha256"],
            "kind": "package-source-provenance", "urls": []}


def collect_official(payload: Path, source_root: Path, platform: str, download_cache: Path,
                     offline: bool = False) -> dict:
    (source_root / "official-components.json").unlink(missing_ok=True)
    manifest = json.loads((HERE / "official-sources.json").read_text())
    validate_inputs(manifest, HERE / "build.sh", HERE / "sources.sha256")
    source_root.mkdir(parents=True, exist_ok=True)
    licenses = payload / "store/ext-core/LICENSES"
    records = []

    def obtain(item: dict) -> tuple[dict, Path]:
        filename = safe_relative(item["file"])
        cached = download(item["urls"], item["sha256"], download_cache, offline)
        target = source_root / "upstream" / item["sha256"] / filename
        target.parent.mkdir(parents=True, exist_ok=True)
        if not target.is_file() or sha256(target) != item["sha256"]:
            shutil.copy2(cached, target)
        return ({"file": str(target.relative_to(source_root)), "sha256": item["sha256"],
                 "urls": item["urls"], "kind": item["kind"]}, target)

    selected = [item for item in manifest["components"] if platform in item["binaries"]]
    if not selected:
        raise ComplianceError(f"no official components for {platform}")
    for component in selected:
        name = component["name"]
        binary = component["binaries"][platform]
        payload_verification = verify_payload(payload, binary)
        unresolved = list(component.get("unresolved_source_requirements", []))
        notice_dir = licenses / "official" / safe_relative(name)
        notice_dir.mkdir(parents=True, exist_ok=True)
        # Remove a previous component's generated notices so an update cannot pass
        # merely because an old version left license files behind.
        for old in notice_dir.iterdir():
            if old.is_dir():
                shutil.rmtree(old)
            else:
                old.unlink()
        sources = []
        archives = {}
        provenance = package_provenance(component, binary, source_root)
        if provenance:
            sources.append(provenance)
        for specification in component.get("provenance_files", []):
            path = HERE / safe_relative(specification["file"])
            if sha256(path) != specification["sha256"]:
                raise ComplianceError(f"source provenance changed without updating its pin: {path}")
            target = source_root / "provenance" / path.name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(path, target)
            sources.append({"file": str(target.relative_to(source_root)), "sha256": specification["sha256"],
                            "kind": "source-provenance", "urls": []})
        if component.get("extract_binary_notices"):
            original = Path(os.environ.get("EXT_CORE_CACHE", str(HERE / ".cache"))) / safe_relative(binary["file"])
            if not extract_notices(original, notice_dir / "original-binary"):
                raise ComplianceError(f"{name}: expected notices absent from original binary archive")
        for item in component["sources"]:
            record, archive = obtain(item)
            sources.append(record)
            archives[item["file"]] = archive
            extract_notices(archive, notice_dir / safe_relative(item["file"]))
        cargo_archives = [archives[item["file"]] for item in component["sources"]
                          if component.get("collect_cargo_dependencies") or item.get("collect_cargo_dependencies")]
        if cargo_archives:
            inputs = []
            for archive in cargo_archives:
                locked, gaps = cargo_sources(archive)
                unresolved.extend(gaps)
                inputs.extend({**item, "file": Path(item["file"]).name} for item in locked)
            with ThreadPoolExecutor(max_workers=8) as executor:
                obtained = list(executor.map(obtain, inputs))
            for record, archive in obtained:
                sources.append(record)
                extract_notices(archive, notice_dir / "cargo" / archive.name)
        dependency_spec = component.get("dependency_manifest")
        if dependency_spec:
            inputs = dependency_inputs(archives[dependency_spec["archive"]], dependency_spec)
            with ThreadPoolExecutor(max_workers=8) as executor:
                obtained = list(executor.map(obtain, inputs))
            for record, archive in obtained:
                sources.append(record)
                extract_notices(archive, notice_dir / "dependencies" / archive.name)
        notice_sources = []
        for item in component.get("notices", []):
            record, path = obtain(item)
            notice_sources.append(record)
            shutil.copy2(path, notice_dir / safe_relative(item["file"]))
        if not any(path.is_file() for path in notice_dir.rglob("*")):
            raise ComplianceError(f"{name}: upstream material contains no license notices")
        records.append({"id": f"official:{name}-{component['version']}", "name": name,
                        "version": component["version"], "license": component["license"],
                        "license_scope": component["license_scope"],
                        "binary_url": binary["url"], "binary_sha256": binary["sha256"],
                        "payload_verification": payload_verification,
                        "sources": sources, "notice_sources": notice_sources,
                        "license_directory": str(notice_dir.relative_to(licenses)),
                        "source_coverage": "incomplete" if unresolved else "pinned-upstream-materials",
                        "source_note": component.get("source_note", "The archive includes upstream build recipes. Source collection does not certify bit-for-bit reproducibility or relicense bundled dependencies."),
                        "unresolved_source_requirements": unresolved})
    shutil.copy2(HERE / "official-sources.json", source_root / "official-source-inputs.json")
    result = {"schema": "buckyos.ext-tools-sources/v1", "platform": platform, "components": records}
    write_json(source_root / "official-components.json", result)
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--payload", type=Path, required=True)
    parser.add_argument("--source-root", "--source-dir", dest="source_root", type=Path, required=True)
    parser.add_argument("--platform", required=True)
    parser.add_argument("--cache", "--download-cache", dest="cache", type=Path, required=True)
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()
    try:
        result = collect_official(args.payload, args.source_root, args.platform, args.cache, args.offline)
    except (ComplianceError, OSError, KeyError, ValueError, tarfile.TarError) as error:
        parser.exit(1, f"official source collection: {error}\n")
    pending = [item["name"] for item in result["components"] if item["unresolved_source_requirements"]]
    print(f"Collected {len(result['components'])} official component records; release blockers: {', '.join(pending) or 'none recorded'}")


if __name__ == "__main__":
    main()
