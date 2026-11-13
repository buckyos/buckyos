#!/usr/bin/env python3
"""Keep ext-core's third-party notices and corresponding source with its release."""
from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor
import gzip
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import tarfile
import tempfile
import time
import tomllib
from urllib.error import URLError
from urllib.parse import urlparse
from urllib.request import Request, urlopen
import zipfile


HERE = Path(__file__).resolve().parent
REPOSITORY_ROOT = HERE.parents[2]
PLATFORMS = {
    "linux-x86_64": "linux-64", "linux-aarch64": "linux-aarch64",
    "darwin-arm64": "osx-arm64", "darwin-x86_64": "osx-64", "windows-x86_64": "win-64",
}
SCHEMA = "buckyos.ext-tools-sources/v1"


class ComplianceError(RuntimeError):
    pass


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def yaml_document(path: Path) -> dict:
    # PyYAML is already a dependency of buckyos-devkit, installed by `uv sync`.
    try:
        import yaml
    except ImportError as error:
        raise ComplianceError("PyYAML is required; run with the existing buckyos-devkit environment (uv run).") from error
    try:
        value = yaml.safe_load(path.read_text())
    except yaml.YAMLError as error:
        raise ComplianceError(f"{path}: recipe is not rendered YAML: {error}") from error
    if not isinstance(value, dict):
        raise ComplianceError(f"{path}: expected a recipe mapping")
    return value


def safe_relative(value: str) -> Path:
    path = PurePosixPath(value.replace("\\", "/"))
    if path.is_absolute() or re.match(r"^[A-Za-z]:", str(path)) or ".." in path.parts or not path.parts:
        raise ComplianceError(f"unsafe relative path: {value}")
    return Path(*path.parts)


def copy_tree(source: Path, destination: Path) -> None:
    destination.mkdir(parents=True, exist_ok=True)
    for path in sorted(source.rglob("*")):
        if path.is_symlink():
            raise ComplianceError(f"source metadata must not contain symlinks: {path}")
        if path.is_file():
            target = destination / path.relative_to(source)
            target.parent.mkdir(parents=True, exist_ok=True)
            if target.exists():
                target.chmod(target.stat().st_mode | 0o200)
            shutil.copy2(path, target)
            target.chmod(target.stat().st_mode | 0o200)


def recipe_sources(recipe: Path) -> list[dict]:
    """Read rendered conda-build and rattler recipes, including staging caches."""
    selected = recipe / "rendered_recipe.yaml"
    if not selected.exists():
        selected = recipe / "meta.yaml"
    if not selected.exists():
        raise ComplianceError(f"{recipe}: no rendered recipe; cannot establish corresponding source")
    document = yaml_document(selected)
    body = document.get("recipe", document)
    found = []

    def visit(node: object) -> None:
        if isinstance(node, list):
            for item in node:
                visit(item)
        elif isinstance(node, dict):
            if "source" in node and node["source"]:
                sources = node["source"]
                sources = sources if isinstance(sources, list) else [sources]
                if not all(isinstance(item, dict) for item in sources):
                    raise ComplianceError(f"{selected}: unsupported source form")
                found.extend(sources)
            if "staging_caches" in node:
                visit(node["staging_caches"])

    visit(body)
    if not found and (recipe / "parent" / "meta.yaml").exists():
        return recipe_sources(recipe / "parent")
    for item in found:
        if any(key in item for key in ("git_url", "git_rev", "git", "svn_url", "hg_url")):
            raise ComplianceError(f"{selected}: VCS source requires a separately pinned source archive: {item}")
        if "{{" in json.dumps(item) or "${" in json.dumps(item):
            raise ComplianceError(f"{selected}: unresolved source template: {item}")
    return found


def download(urls: list[str], expected: str, cache: Path, offline: bool = False) -> Path:
    if not re.fullmatch(r"[0-9a-fA-F]{64}", expected):
        raise ComplianceError(f"source requires a SHA256 pin: {urls}")
    expected = expected.lower()
    cache.mkdir(parents=True, exist_ok=True)
    destination = cache / expected
    if destination.is_file() and sha256(destination) == expected:
        return destination
    if offline:
        raise ComplianceError(f"missing cached source {expected}: {urls}")
    errors = []
    mirrors_file = HERE / "source-mirrors.json"
    if mirrors_file.is_file():
        mirrors = json.loads(mirrors_file.read_text()).get(expected, {}).get("urls", [])
        urls = [*urls, *mirrors]
    candidates = []
    for url in urls:
        if url.startswith("http://"):
            candidates.append("https://" + url.removeprefix("http://"))
        candidates.append(url)
    for url in dict.fromkeys(candidates):
        if urlparse(url).scheme not in ("https", "http"):
            raise ComplianceError(f"unsupported download URL: {url}")
        for attempt in range(3):
            temporary = None
            try:
                request = Request(url, headers={"User-Agent": "BuckyOS-ext-core-source-collector/1"})
                with urlopen(request, timeout=45) as response, tempfile.NamedTemporaryFile(dir=cache, delete=False) as handle:
                    temporary = Path(handle.name)
                    shutil.copyfileobj(response, handle, length=1024 * 1024)
                actual = sha256(temporary)
                if actual != expected:
                    raise ComplianceError(f"SHA256 mismatch for {url}: expected {expected}, got {actual}")
                temporary.replace(destination)
                return destination
            except (OSError, URLError, ComplianceError) as error:
                errors.append(str(error))
                if temporary is not None:
                    temporary.unlink(missing_ok=True)
                if attempt < 2:
                    time.sleep(attempt + 1)
    raise ComplianceError(f"source unavailable {urls}: " + "; ".join(errors))


def license_name(name: str) -> bool:
    path = PurePosixPath(name)
    return path.name == "README.chromium" or any(part.lower() in {"licenses", "licences", "legal", "notices"} for part in path.parts[:-1]) or bool(
        re.match(r"(?i)^(licen[cs]e|copying|copyright|notice|authors|contributors|legal|ofl|unlicense|fontlog|[al]?gpl)([._-].*|\d.*)?$", path.name)
    )


def extract_notices(archive: Path, destination: Path) -> list[str]:
    """Copy actual upstream texts, never substitute an SPDX identifier for them."""
    destination.mkdir(parents=True, exist_ok=True)
    files = []

    def keep(name: str, contents: bytes) -> None:
        if not license_name(name) or not contents or len(contents) > 8 * 1024 * 1024:
            return
        try:
            relative = safe_relative(name)
        except ComplianceError:
            # A few upstream archives use ../AUTHORS.txt. Preserve its text without
            # ever using that path on disk; do not extract links or other members.
            relative = Path("_archive_paths") / hashlib.sha256(name.encode()).hexdigest() / PurePosixPath(name).name
            map_path = destination / "ARCHIVE-PATHS.json"
            mapping = json.loads(map_path.read_text()) if map_path.is_file() else {}
            mapping[relative.as_posix()] = name
            write_json(map_path, mapping)
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(contents)
        files.append(relative.as_posix())

    try:
        if zipfile.is_zipfile(archive):
            with zipfile.ZipFile(archive) as handle:
                for member in handle.infolist():
                    if not member.is_dir() and license_name(member.filename) and member.file_size <= 8 * 1024 * 1024:
                        keep(member.filename, handle.read(member))
        elif tarfile.is_tarfile(archive):
            with tarfile.open(archive) as handle:
                for member in handle:
                    if member.isfile() and license_name(member.name) and member.size <= 8 * 1024 * 1024:
                        stream = handle.extractfile(member)
                        if stream:
                            keep(member.name, stream.read())
    except (tarfile.TarError, zipfile.BadZipFile, OSError) as error:
        raise ComplianceError(f"cannot inspect source archive {archive}: {error}") from error
    return files


def cargo_sources(archive: Path) -> tuple[list[dict], list[str]]:
    """Include registry source pinned by a project's own lock, including static Rust code."""
    if not tarfile.is_tarfile(archive):
        return [], []
    lock = None
    with tarfile.open(archive) as handle:
        for member in handle:
            if member.isfile() and PurePosixPath(member.name).name == "Cargo.lock" and len(PurePosixPath(member.name).parts) <= 2:
                if member.size > 8 * 1024 * 1024:
                    raise ComplianceError(f"oversized Cargo.lock in {archive}")
                lock = tomllib.loads(handle.extractfile(member).read().decode())
                break
    if lock is None:
        return [], []
    artifacts = []
    unresolved = []
    for package in lock.get("package", []):
        origin = package.get("source", "")
        if not origin:
            continue
        if origin != "registry+https://github.com/rust-lang/crates.io-index":
            unresolved.append(f'Cargo.lock contains non-registry source requiring an exact source bundle: {package["name"]} {origin}')
            continue
        digest = package.get("checksum", "")
        if not re.fullmatch("[0-9a-f]{64}", digest):
            unresolved.append(f'Cargo.lock lacks checksum: {package["name"]} {package["version"]}')
            continue
        filename = f'{package["name"]}-{package["version"]}.crate'
        artifacts.append({"file": (Path("upstream") / digest / filename).as_posix(), "sha256": digest,
                          "urls": [f'https://static.crates.io/crates/{package["name"]}/{filename}'],
                          "kind": "dependency-source", "lockfile": "Cargo.lock"})
    return artifacts, unresolved


def extract_header_notices(archive: Path, destination: Path) -> list[str]:
    """Some projects (GLU, SQLite) put their license in source headers only."""
    if not tarfile.is_tarfile(archive):
        return []
    mapping = {}
    with tarfile.open(archive) as handle:
        for member in handle:
            if not member.isfile() or PurePosixPath(member.name).suffix not in {".c", ".h", ".cpp", ".hpp"}:
                continue
            stream = handle.extractfile(member)
            if stream is None:
                continue
            beginning = stream.read(65536)
            match = re.match(rb"\s*(/\*.*?\*/)", beginning, flags=re.S)
            if not match:
                continue
            header = match[1]
            lowered = header.lower()
            if b"copyright" not in lowered or not any(word in lowered for word in (b"permission", b"license", b"public domain", b"blessing")):
                continue
            digest = hashlib.sha256(header).hexdigest()
            relative = f"source-headers/{digest}.txt"
            target = destination / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(header + b"\n")
            mapping[member.name] = relative
    if mapping:
        write_json(destination / "SOURCE-HEADERS.json", mapping)
    return sorted(set(mapping.values()))


def artifact(source: dict, recipe: Path, destination: Path, source_root: Path) -> dict:
    if "path" in source:
        local = recipe / safe_relative(str(source["path"]))
        if not local.is_file():
            raise ComplianceError(f"recipe local source is missing: {local}")
        target = destination / "local-sources" / safe_relative(str(source["path"]))
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(local, target)
        return {"file": target.relative_to(source_root).as_posix(), "sha256": sha256(target), "kind": "recipe-source"}
    urls = source.get("url")
    urls = urls if isinstance(urls, list) else [urls]
    if not urls or not all(isinstance(url, str) and url for url in urls):
        raise ComplianceError(f"recipe source has no URL: {source}")
    digest = str(source.get("sha256", "")).lower()
    if not re.fullmatch("[0-9a-f]{64}", digest):
        raise ComplianceError(f"recipe source has no SHA256 pin: {source}")
    filename = PurePosixPath(urlparse(urls[0]).path).name or "source"
    relative = Path("upstream") / digest / safe_relative(filename)
    patches = source.get("patches") or []
    patches = patches if isinstance(patches, list) else [patches]
    for patch in patches:
        if not (recipe / safe_relative(str(patch))).is_file():
            parent = recipe / "parent" / safe_relative(str(patch))
            if not parent.is_file():
                raise ComplianceError(f"missing recipe patch: {recipe / str(patch)}")
    return {"file": relative.as_posix(), "sha256": digest, "urls": urls, "kind": "upstream-source", "recipe_source": source}


def first_party_notices(payload: Path) -> None:
    first_party = payload / "store/ext-core/LICENSES/buckyos-packaging"
    first_party.mkdir(parents=True, exist_ok=True)
    for name in ("LICENSE", "NOTICE"):
        shutil.copy2(REPOSITORY_ROOT / name, first_party / name)
    (first_party / "README.txt").write_text(
        "The included AGPL-3.0-only license and BuckyOS notices apply to BuckyOS-authored "
        "launchers, smoke tests, Windows shims, and build/packaging scripts. "
        "Third-party tools, libraries, fonts, and their source files retain their own licenses; "
        "they are not relicensed by this notice. The matching source archive includes the BuckyOS scripts.\n"
    )


def collect_conda(prefix: Path | None, lock: Path, package_cache: Path, source_root: Path,
                  payload: Path, platform: str, download_cache: Path, offline: bool = False) -> dict:
    source_root.mkdir(parents=True, exist_ok=True)
    (source_root / "conda-components.json").unlink(missing_ok=True)
    first_party_notices(payload)
    licenses = payload / "store/ext-core/LICENSES"
    licenses.mkdir(parents=True, exist_ok=True)
    records = {}
    for record in package_cache.rglob("info/repodata_record.json"):
        meta = json.loads(record.read_text())
        if meta.get("url"):
            records[meta["url"]] = record.parent
    locked = yaml_document(lock)
    installed = {p["url"]: p for p in locked.get("package", []) if p["platform"] == PLATFORMS[platform] and p["manager"] == "conda"}
    if not installed:
        raise ComplianceError(f"{lock}: no packages for {platform}")
    if prefix is not None:
        metadata = [json.loads(p.read_text()) for p in (prefix / "conda-meta").glob("*.json")]
        if not metadata:
            raise ComplianceError(f"{prefix}: conda-meta is missing; capture before pruning")
        if {p.get("url") for p in metadata} != set(installed):
            raise ComplianceError("installed conda prefix differs from conda-lock.yml")
    else:
        metadata = None
    components = []
    requests = {}
    errors = []
    for binary_url, pin in sorted(installed.items()):
        info = records.get(binary_url)
        if info is None:
            errors.append(f"missing extracted package metadata for {binary_url}; reinstall the locked prefix")
            continue
        meta = json.loads((info / "repodata_record.json").read_text())
        expected = pin["hash"]["sha256"]
        if meta.get("sha256") != expected:
            errors.append(f"package metadata hash differs from lock: {binary_url}")
            continue
        binary_archive = info.parent.parent / meta.get("fn", PurePosixPath(urlparse(binary_url).path).name)
        if not binary_archive.is_file() or sha256(binary_archive) != expected:
            errors.append(f"missing or changed locked binary package archive: {binary_archive}")
            continue
        package_id = f'{meta["name"]}-{meta["version"]}-{meta["build"]}'
        destination = source_root / "packages/conda" / package_id
        copy_tree(info, destination / "info")
        if metadata:
            write_json(destination / "installed-record.json", next(p for p in metadata if p["url"] == binary_url))
        file_list = (info / "files").read_text().splitlines() if (info / "files").is_file() else []
        if (info / "paths.json").is_file():
            file_list = [p["_path"] for p in json.loads((info / "paths.json").read_text())["paths"]]
        component = {"id": "conda:" + package_id, "name": meta["name"], "version": meta["version"],
                     "build": meta["build"], "license": meta.get("license", ""), "binary_url": binary_url,
                     "binary_sha256": expected, "recipe": (destination / "info/recipe").relative_to(source_root).as_posix(),
                     "package_metadata": (destination / "info").relative_to(source_root).as_posix(),
                     "metadata_only": not file_list, "sources": []}
        notices = licenses / "conda" / package_id
        if notices.exists():
            shutil.rmtree(notices)
        if (info / "licenses").is_dir():
            copy_tree(info / "licenses", notices)
        try:
            recipe = info / "recipe"
            sources = recipe_sources(recipe)
            if file_list and not sources:
                raise ComplianceError(f"{package_id}: package contains files but has no rendered source recipe")
            for entry in sources:
                record = artifact(entry, recipe, destination, source_root)
                if record.get("sha256") in requests:
                    record["file"] = requests[record["sha256"]]["file"]
                component["sources"].append(record)
                if record["kind"] == "upstream-source":
                    requests[record["sha256"]] = record
        except ComplianceError as error:
            errors.append(str(error))
        components.append(component)
    if errors:
        raise ComplianceError("Conda source inventory incomplete:\n" + "\n".join(errors))

    def retrieve(item: dict) -> None:
        target = source_root / item["file"]
        if target.is_file() and sha256(target) == item["sha256"]:
            return
        cached = download(item["urls"], item["sha256"], download_cache, offline)
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(cached, target)

    with ThreadPoolExecutor(max_workers=6) as pool:
        futures = [(item, pool.submit(retrieve, item)) for item in requests.values()]
        for item, future in futures:
            try:
                future.result()
            except Exception as error:
                errors.append(str(error))
    if errors:
        raise ComplianceError("Conda corresponding source downloads incomplete:\n" + "\n".join(errors))
    cargo_by_source = {}
    for item in requests.values():
        cargo_by_source[item["sha256"]] = cargo_sources(source_root / item["file"])
    cargo_requests = {}
    for component in components:
        additions = {}
        unresolved = []
        for item in component["sources"]:
            dependencies, gaps = cargo_by_source.get(item["sha256"], ([], []))
            unresolved.extend(gaps)
            for dependency in dependencies:
                additions[dependency["sha256"]] = dependency
        component["sources"].extend(additions.values())
        component["unresolved_source_requirements"] = sorted(set(unresolved))
        component["source_coverage"] = "incomplete" if unresolved else "recipe-and-locked-dependencies"
        cargo_requests.update(additions)
    with ThreadPoolExecutor(max_workers=6) as pool:
        futures = [pool.submit(retrieve, item) for item in cargo_requests.values()]
        for future in futures:
            try:
                future.result()
            except Exception as error:
                errors.append(str(error))
    if errors:
        raise ComplianceError("Conda locked Rust dependency source downloads incomplete:\n" + "\n".join(errors))
    for component in components:
        notices = licenses / "conda" / component["id"].removeprefix("conda:")
        if not any(p.is_file() for p in notices.rglob("*")):
            for item in component["sources"]:
                if item["kind"] == "upstream-source":
                    extract_notices(source_root / item["file"], notices / item["sha256"][:12])
                    if not any(p.is_file() for p in notices.rglob("*")):
                        extract_header_notices(source_root / item["file"], notices / item["sha256"][:12])
        if not component["metadata_only"] and not any(p.is_file() for p in notices.rglob("*")):
            errors.append(f'{component["id"]}: no actual license text found in package or source')
        for item in component["sources"]:
            if item["kind"] == "dependency-source":
                extract_notices(source_root / item["file"], notices / "locked-dependencies" / item["sha256"])
        component["license_directory"] = notices.relative_to(licenses).as_posix()
    if errors:
        raise ComplianceError("Conda license inventory incomplete:\n" + "\n".join(errors))
    result = {"schema": SCHEMA, "platform": platform, "components": components,
              "payload_binding": "installed-prefix-records" if prefix is not None else "lock-and-cache-only"}
    write_json(source_root / "conda-components.json", result)
    return result


def file_inventory(root: Path, exclude: set[str] | None = None) -> list[dict]:
    result = []
    for path in sorted(root.rglob("*")):
        relative = path.relative_to(root).as_posix()
        if exclude and any(relative == item or relative.startswith(item + "/") for item in exclude):
            continue
        if path.is_symlink():
            result.append({"path": relative, "symlink": os.readlink(path)})
        elif path.is_file():
            result.append({"path": relative, "sha256": sha256(path), "size": path.stat().st_size})
    return result


def verify_source_files(source_root: Path, components: list[dict]) -> None:
    errors = []
    hashes = {}
    for component in components:
        if component.get("unresolved_source_requirements"):
            errors.append(f'{component["id"]}: {component["unresolved_source_requirements"]}')
        if component.get("source_coverage") in {"partial", "incomplete", "unverified"}:
            errors.append(f'{component["id"]}: source coverage is {component["source_coverage"]}')
        if not component.get("sources") and not component.get("metadata_only"):
            errors.append(f'{component["id"]}: no corresponding source artifacts')
        for item in component.get("sources", []):
            path = source_root / safe_relative(item["file"])
            if path not in hashes:
                hashes[path] = sha256(path) if path.is_file() else None
            if hashes[path] != item["sha256"]:
                errors.append(f'{component["id"]}: missing or changed source artifact {item["file"]}')
    if errors:
        raise ComplianceError("Cannot release incomplete corresponding source:\n" + "\n".join(errors))


def finalize(payload: Path, source_root: Path, platform: str, version: str, dist: Path) -> dict:
    (source_root / "collection-report.json").unlink(missing_ok=True)
    (source_root / "source-manifest.json").unlink(missing_ok=True)
    for filename in ("source-manifest.json", "corresponding-source.json"):
        (payload / "store/ext-core/LICENSES" / filename).unlink(missing_ok=True)
    first_party_notices(payload)
    components = []
    for kind in ("conda", "official"):
        path = source_root / f"{kind}-components.json"
        if not path.is_file():
            raise ComplianceError(f"{kind} collection is missing: {path}")
        document = json.loads(path.read_text())
        if document.get("platform") != platform or document.get("schema") != SCHEMA:
            raise ComplianceError(f"source inventory schema/platform mismatch: {path}")
        components.extend(document["components"])
    licenses = payload / "store/ext-core/LICENSES"
    for component in components:
        directory = licenses / safe_relative(component["license_directory"])
        if not component.get("metadata_only") and not any(p.is_file() for p in directory.rglob("*")):
            raise ComplianceError(f'{component["id"]}: license texts are missing')
    for name in ("build.sh", "compliance.py", "official_sources.py", "official-sources.json", "source-mirrors.json", "release_manifest.py",
                 "conda-env.yml", "conda-lock.yml", "sources.sha256", "ext-manifest.json"):
        source = HERE / name
        if not source.is_file():
            raise ComplianceError(f"build recipe missing: {source}")
        target = source_root / "buckyos-build" / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, target)
    for source in [*HERE.glob("*.json"), *HERE.glob("*.md")]:
        shutil.copy2(source, source_root / "buckyos-build" / source.name)
    for name in ("smoke", "windows"):
        copy_tree(HERE / name, source_root / "buckyos-build" / name)
    for name in ("LICENSE", "NOTICE"):
        shutil.copy2(REPOSITORY_ROOT / name, source_root / "buckyos-build" / name)
    copied_licenses = source_root / "LICENSES"
    if copied_licenses.exists():
        shutil.rmtree(copied_licenses)
    for path in licenses.iterdir():
        if path.name in {"source-manifest.json", "corresponding-source.json", "SOURCE_CODE.md"}:
            continue
        if path.is_dir():
            copy_tree(path, copied_licenses / path.name)
        elif path.is_file():
            copied_licenses.mkdir(parents=True, exist_ok=True)
            shutil.copy2(path, copied_licenses / path.name)
    verify_source_files(source_root, components)
    manifest = {"schema": SCHEMA, "package": "ext-core", "version": version, "platform": platform,
                "components": components, "files": file_inventory(source_root, {"source-manifest.json"}),
                "payload_files": file_inventory(payload, {"store/ext-core/LICENSES"})}
    write_json(source_root / "source-manifest.json", manifest)
    shutil.copy2(source_root / "source-manifest.json", licenses / "source-manifest.json")
    filename = f"ext-core-{version}-{platform}-corresponding-source.tar.gz"
    dist.mkdir(parents=True, exist_ok=True)
    archive = dist / filename
    temporary = archive.with_suffix(archive.suffix + ".part")
    try:
        with temporary.open("wb") as raw, gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0, compresslevel=1) as compressed:
            with tarfile.open(fileobj=compressed, mode="w|") as tar:
                for path in sorted(source_root.rglob("*")):
                    if not path.is_file():
                        continue
                    info = tar.gettarinfo(str(path), arcname=path.relative_to(source_root).as_posix())
                    info.uid = info.gid = 0
                    info.uname = info.gname = ""
                    info.mtime = 0
                    with path.open("rb") as handle:
                        tar.addfile(info, handle)
        temporary.replace(archive)
    finally:
        temporary.unlink(missing_ok=True)
    digest = sha256(archive)
    reference = {"file": filename, "sha256": f"sha256:{digest}", "size": archive.stat().st_size,
                 "path": f"ext-tools/ext-core/{version}/{platform}/{digest}.source.tar.gz",
                 "manifest_sha256": "sha256:" + sha256(source_root / "source-manifest.json")}
    write_json(licenses / "corresponding-source.json", reference)
    (licenses / "SOURCE_CODE.md").write_text(
        f"# Source code for ext-core {version} ({platform})\n\n"
        "These tools retain their individual licenses. BuckyOS's repository license does not replace them.\n\n"
        "Only the BuckyOS-authored launchers, smoke tests, Windows shims, and packaging scripts are AGPL-3.0-only; "
        "their license and copyright notices are in `buckyos-packaging/`.\n\n"
        f"The matching source archive is `{filename}` (SHA256 `{digest}`). It must be offered alongside this payload, "
        "at the same download location and without an extra charge. The release manifest records its URL/path and digest. "
        "If you redistribute the payload, redistribute that archive and these notices as well.\n\n"
        f"Download: https://buckyos-installer.tbudr.top/{reference['path']}\n\n"
        "`source-manifest.json` records exact binary package URLs separately from upstream source URLs, hashes, "
        "licenses, Conda recipes, applied patches, and build configuration. The archive includes those source files "
        "and recipes, plus the BuckyOS repackaging scripts. See each recipe for upstream build requirements.\n\n"
        "LGPL libraries can be replaced by compatible rebuilt libraries in store/ext-core. No license here restricts "
        "modifying those libraries or reverse engineering for debugging those modifications. Static combinations "
        "require the corresponding rebuild/relink inputs recorded for that component.\n"
    )
    return reference


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["conda", "finalize"])
    parser.add_argument("--prefix", type=Path)
    parser.add_argument("--lock", type=Path, default=HERE / "conda-lock.yml")
    parser.add_argument("--package-cache", type=Path)
    parser.add_argument("--source-dir", type=Path, required=True)
    parser.add_argument("--payload", type=Path, required=True)
    parser.add_argument("--platform", choices=PLATFORMS, required=True)
    parser.add_argument("--download-cache", type=Path)
    parser.add_argument("--version")
    parser.add_argument("--dist", type=Path)
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()
    try:
        if args.command == "conda":
            if args.package_cache is None or args.download_cache is None:
                raise ComplianceError("conda requires --package-cache and --download-cache")
            result = collect_conda(args.prefix, args.lock, args.package_cache, args.source_dir, args.payload,
                                   args.platform, args.download_cache, args.offline)
            print(f'Collected {len(result["components"])} Conda components and corresponding source for {args.platform}')
        else:
            if args.version is None or args.dist is None:
                raise ComplianceError("finalize requires --version and --dist")
            result = finalize(args.payload, args.source_dir, args.platform, args.version, args.dist)
            print(f'Corresponding source: {result["file"]} {result["sha256"]}')
    except ComplianceError as error:
        if args.command == "finalize" and args.source_dir.is_dir():
            inventories = {}
            for kind in ("conda", "official"):
                path = args.source_dir / f"{kind}-components.json"
                if path.is_file():
                    inventories[kind] = {"file": path.name, "sha256": sha256(path),
                                         "components": len(json.loads(path.read_text()).get("components", []))}
            write_json(args.source_dir / "collection-report.json", {
                "schema": SCHEMA, "package": "ext-core", "platform": args.platform,
                "version": args.version, "release_ready": False, "error": str(error),
                "collected_inventories": inventories,
                "build_inputs": file_inventory(args.source_dir / "buckyos-build"),
            })
        parser.exit(1, f"ext-core compliance: {error}\n")


if __name__ == "__main__":
    main()
