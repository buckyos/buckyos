#!/usr/bin/env python3

from __future__ import annotations

import argparse
import hashlib
import io
import json
import os
import re
import subprocess
import sys
import tarfile
import urllib.parse
import urllib.request
from pathlib import Path

from open_source_runtime import collect_ext_core
from open_source_gateway import collect_gateway


WEB_PROJECTS = {
    "desktop": ("Desktop", "src/frame/desktop"),
    "opendan": ("OpenDAN Web", "src/frame/opendan/web"),
    "node-active": ("Node activation", "src/kernel/node_active"),
    "sys-test": ("System test app", "src/apps/sys_test"),
    "sys-test-web": ("System test Web", "src/apps/sys_test/web"),
    "text-editor": ("Text editor", "src/apps/text_editor"),
}
LICENSE_NAME = re.compile(r"^(licen[cs]e|copying|copyright|notice|authors)([._-].*)?$", re.I)
LICENSE_GRANT = re.compile(r"^(licen[cs]es?|copying)([._-].*)?$", re.I)
BASE_NOTICES = [
    "Versions come from resolved Cargo metadata, installed production npm dependencies, the SDK distribution SBOM and available extension payload manifests. Build dependencies and dependencies for other target platforms may also be listed.",
    "This build inventory does not inspect the host operating system, separately installed apps, container base images or packages downloaded at runtime. Missing license metadata is shown explicitly; bundled license notices take precedence over a short license identifier.",
]
RUNTIME_LICENSES = {
    "deno": ("denoland/deno", "v{version}", ["LICENSE.md"]),
    "jq": ("jqlang/jq", "jq-{version}", ["COPYING"]),
    "pandoc": ("jgm/pandoc", "{version}", ["COPYRIGHT"]),
    "uv": ("astral-sh/uv", "{version}", ["LICENSE-MIT", "LICENSE-APACHE"]),
}


def read_json(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def safe_url(value: object) -> str:
    if isinstance(value, dict):
        value = value.get("url", "")
    if not isinstance(value, str):
        return ""
    value = value.removeprefix("git+")
    value = re.sub(r"^git://", "https://", value)
    value = re.sub(r"^git@github.com:", "https://github.com/", value)
    parsed = urllib.parse.urlsplit(value)
    return value if parsed.scheme in {"http", "https"} and parsed.hostname and not parsed.username else ""


def license_value(value: object) -> str | None:
    if isinstance(value, str):
        return value.strip() or None
    if isinstance(value, dict):
        return license_value(value.get("type"))
    if isinstance(value, list):
        licenses = [license_value(item) for item in value]
        return " OR ".join(item for item in licenses if item) or None
    return None


def license_files(directory: Path, explicit: str | None = None) -> list[dict]:
    candidates = []
    for current, directories, files in os.walk(directory):
        current = Path(current)
        depth = len(current.relative_to(directory).parts)
        directories[:] = sorted(name for name in directories if name not in {"node_modules", ".git", "target", "__pycache__", ".cache"} and depth < 8)
        in_license_directory = any(LICENSE_GRANT.match(part) for part in current.relative_to(directory).parts)
        candidates.extend(current / name for name in files if in_license_directory or LICENSE_NAME.match(name))
    if explicit:
        path = (directory / explicit).resolve()
        if path.is_relative_to(directory.resolve()):
            candidates.append(path)
    result = []
    for path in sorted(set(candidates)):
        if not path.is_file() or not path.resolve().is_relative_to(directory.resolve()):
            continue
        if path.stat().st_size > 2_000_000:
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeError):
            continue
        if text.strip():
            result.append({"name": path.relative_to(directory).as_posix(), "text": text})
    return result


def license_grant_files(files: list[dict]) -> list[dict]:
    return [file for file in files if any(LICENSE_GRANT.match(part) for part in Path(file["name"]).parts)]


def npm_package(directory: Path) -> dict:
    data = read_json(directory / "package.json")
    return {
        "name": data["name"], "version": data["version"], "ecosystem": "npm",
        "license": license_value(data.get("license", data.get("licenses"))),
        "description": data.get("description", ""), "homepage": safe_url(data.get("homepage")),
        "repository": safe_url(data.get("repository")), "files": license_files(directory),
    }


def resolve_npm(directory: Path, name: str) -> Path | None:
    for parent in (directory, *directory.parents):
        candidate = parent / "node_modules" / name
        if (candidate / "package.json").is_file():
            return candidate.resolve()
    return None


def collect_web(repo: Path, scope: str) -> tuple[list[dict], dict, list[str]]:
    label, relative = WEB_PROJECTS[scope]
    root = repo / relative
    if not (root / "node_modules").is_dir():
        return [], {"id": scope, "label": label, "status": "unavailable"}, [f"{label}: dependencies are not installed."]
    pending = [root]
    visited = {root}
    packages = []
    missing = set()
    while pending:
        directory = pending.pop()
        manifest = read_json(directory / "package.json")
        optional = manifest.get("optionalDependencies", {})
        names = set(manifest.get("dependencies", {})) | set(optional)
        if directory != root:
            names |= set(manifest.get("peerDependencies", {}))
        for name in sorted(names):
            dependency = resolve_npm(directory, name)
            if dependency is None:
                if name not in optional and not manifest.get("peerDependenciesMeta", {}).get(name, {}).get("optional"):
                    missing.add(name)
                continue
            if dependency in visited:
                continue
            visited.add(dependency)
            pending.append(dependency)
            package = npm_package(dependency)
            package["scopes"] = [scope]
            packages.append(package)
    status = "partial" if missing else "complete"
    notices = [f"{label}: unresolved installed dependencies: {', '.join(sorted(missing))}."] if missing else []
    return packages, {"id": scope, "label": label, "status": status}, notices


def collect_cargo(repo: Path, metadata_path: Path | None, offline: bool = False) -> tuple[list[dict], dict, list[str]]:
    if metadata_path:
        metadata = read_json(metadata_path)
    else:
        metadata = json.loads(subprocess.check_output(
            ["cargo", "metadata", "--locked", *(["--offline"] if offline else []), "--format-version", "1"], cwd=repo / "src", text=True,
        ))
    members = set(metadata["workspace_members"])
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    data = {package["id"]: package for package in metadata["packages"]}
    pending = [key for key in members if not Path(data[key]["manifest_path"]).is_relative_to(repo / "src" / "test")]
    reachable = set()
    while pending:
        key = pending.pop()
        if key in reachable:
            continue
        reachable.add(key)
        for dependency in nodes[key]["deps"]:
            if any(kind["kind"] != "dev" for kind in dependency["dep_kinds"]):
                pending.append(dependency["pkg"])
    packages = []
    for key in sorted(reachable - members):
        package = data[key]
        source = package.get("source") or ""
        revision = source.rsplit("#", 1)[-1] if source.startswith("git+") and "#" in source else ""
        packages.append({
            "name": package["name"], "version": package["version"], "ecosystem": "cargo",
            "license": license_value(package.get("license")), "description": package.get("description") or "",
            "homepage": safe_url(package.get("homepage")), "repository": safe_url(package.get("repository")),
            "revision": revision, "scopes": ["rust"],
            "files": license_files(Path(package["manifest_path"]).parent, package.get("license_file")),
        })
    return packages, {"id": "rust", "label": "BuckyOS Rust services", "status": "complete"}, []


def fetch_npm_package(name: str, version: str) -> dict:
    url = f"https://registry.npmjs.org/{urllib.parse.quote(name, safe='')}/{urllib.parse.quote(version, safe='')}"
    with urllib.request.urlopen(url, timeout=30) as response:
        metadata = json.load(response)
    files = []
    with urllib.request.urlopen(metadata["dist"]["tarball"], timeout=60) as response:
        with tarfile.open(fileobj=io.BytesIO(response.read()), mode="r:gz") as archive:
            for member in archive.getmembers():
                parts = Path(member.name).parts
                if not member.isfile() or len(parts) < 2 or not LICENSE_NAME.match(parts[1]) or member.size > 2_000_000:
                    continue
                stream = archive.extractfile(member)
                if stream:
                    try:
                        files.append({"name": "/".join(parts[1:]), "text": stream.read().decode("utf-8")})
                    except UnicodeError:
                        pass
    return {
        "name": name, "version": version, "ecosystem": "npm",
        "license": license_value(metadata.get("license", metadata.get("licenses"))),
        "description": metadata.get("description", ""), "homepage": safe_url(metadata.get("homepage")),
        "repository": safe_url(metadata.get("repository")), "files": files,
    }


def collect_sdk(repo: Path, known: list[dict], fetch: bool) -> tuple[list[dict], dict, list[str]]:
    root = repo / "src/rootfs/libexec/buckyos-tool"
    sbom_path = root / "sbom.cdx.json"
    if not sbom_path.is_file():
        return [], {"id": "sdk-tool", "label": "SDK and CLI runtime", "status": "unavailable"}, ["SDK and CLI distribution SBOM is unavailable."]
    available = {(p["ecosystem"], p["name"], p["version"]): p for p in known}
    source = repo.parent / "buckyos-websdk"
    if (source / "node_modules/.pnpm").is_dir():
        for modules in sorted((source / "node_modules/.pnpm").glob("*/node_modules")):
            for manifest in sorted([*modules.glob("*/package.json"), *modules.glob("@*/*/package.json")]):
                data = read_json(manifest)
                key = ("npm", data["name"], data["version"])
                if key not in available:
                    available[key] = npm_package(manifest.parent)
    packages = []
    if (root / "package.json").is_file():
        packages.append({**npm_package(root), "scopes": ["sdk-tool"]})
    missing = []
    for component in read_json(sbom_path).get("components", []):
        name, version = component.get("name"), component.get("version")
        if not name or not version:
            continue
        ecosystem = "npm" if component.get("purl", "").startswith("pkg:npm/") else "runtime"
        key = (ecosystem, name, version)
        package = available.get(key)
        if not package and fetch and ecosystem == "npm":
            try:
                package = fetch_npm_package(name, version)
            except (OSError, ValueError, tarfile.TarError) as error:
                print(f"Unable to read npm metadata for {name}@{version}: {error}", file=sys.stderr)
        if not package and fetch and name == "deno":
            url = f"https://raw.githubusercontent.com/denoland/deno/v{version}/LICENSE.md"
            try:
                with urllib.request.urlopen(url, timeout=30) as response:
                    text = response.read().decode("utf-8")
                package = {"name": name, "version": version, "ecosystem": "runtime", "license": "MIT" if "MIT License" in text else None,
                           "homepage": "https://deno.com/", "repository": "https://github.com/denoland/deno", "files": [{"name": "LICENSE.md", "text": text}]}
            except (OSError, UnicodeError):
                pass
        if not package:
            licenses = [item.get("expression") or item.get("license", {}).get("id") or item.get("license", {}).get("name") for item in component.get("licenses", [])]
            package = {"name": name, "version": version, "ecosystem": ecosystem, "license": " OR ".join(item for item in licenses if item) or None, "files": []}
        if not package.get("license"):
            missing.append(f"{name}@{version}")
        packages.append({**package, "scopes": ["sdk-tool"]})
    notices = ["SDK distribution: license metadata is unavailable for " + ", ".join(missing) + "."] if missing else []
    return packages, {"id": "sdk-tool", "label": "SDK and CLI runtime", "status": "partial" if missing else "complete"}, notices


def collect_assets(repo: Path) -> tuple[list[dict], dict, list[str]]:
    root = repo / "src/frame/desktop/public/aiworkspace/stencils"
    index = root / "index.json"
    if not index.is_file():
        return [], {"id": "assets", "label": "Bundled assets", "status": "unavailable"}, []
    metadata = read_json(index)
    digest = hashlib.sha256()
    for path in sorted(root.glob("*.json")):
        digest.update(path.name.encode())
        digest.update(path.read_bytes())
    return [{"name": "draw.io stencil library", "version": f"snapshot-{metadata['generated_at'][:10]}-{digest.hexdigest()[:12]}",
             "ecosystem": "asset", "license": "Apache-2.0 (see bundled notices)", "scopes": ["assets"],
             "description": "Bundled stencil snapshot; the upstream release version was not recorded.",
             "repository": "https://github.com/jgraph/drawio", "files": license_files(root)}], {
                 "id": "assets", "label": "Bundled assets", "status": "partial",
             }, ["The draw.io stencil snapshot is identified by its recorded date and content hash; its upstream release version was not recorded."]


def read_existing(output: Path) -> tuple[list[dict], list[dict], list[str]]:
    if not (output / "index.json").is_file():
        return [], [], []
    index = read_json(output / "index.json")
    details = {}
    for path in (output / "licenses").glob("*.json"):
        details.update(read_json(path))
    packages = [{**package, "files": details.get(package["id"], {}).get("files", [])} for package in index["packages"]]
    return packages, index["scopes"], index.get("notices", [])


def complete_licenses(packages: list[dict], previous: list[dict], fetch: bool) -> None:
    cached = {(p["ecosystem"], p["name"], p["version"], p.get("revision", "")): p for p in previous}
    for package in packages:
        key = (package["ecosystem"], package["name"], package["version"], package.get("revision", ""))
        runtime = package["ecosystem"] == "runtime" and package["name"] in RUNTIME_LICENSES
        if runtime and not package.get("files") and cached.get(key, {}).get("files"):
            package["files"] = cached[key]["files"]
            package["license"] = package.get("license") or cached[key].get("license")
            for field in ["homepage", "repository"]:
                package[field] = package.get(field) or cached[key].get(field, "")
        if runtime and fetch and not package.get("files"):
            repository, tag, paths = RUNTIME_LICENSES[package["name"]]
            files = []
            for path in paths:
                url = f"https://raw.githubusercontent.com/{repository}/{tag.format(version=package['version'])}/{path}"
                try:
                    with urllib.request.urlopen(url, timeout=30) as response:
                        text = response.read().decode("utf-8")
                    if text.strip():
                        files.append({"name": path, "text": text})
                except (OSError, UnicodeError) as error:
                    print(f"Unable to read {package['name']}@{package['version']} license: {error}", file=sys.stderr)
            if files:
                package["files"] = files
                package["repository"] = f"https://github.com/{repository}"
        grants = package.get("files", []) if runtime else license_grant_files(package.get("files", []))
        if not package.get("license") and grants:
            package["license"] = "SEE LICENSE IN " + ", ".join(f["name"] for f in grants)


def write_inventory(output: Path, packages: list[dict], scopes: list[dict], notices: list[str]) -> dict:
    merged = {}
    for item in packages:
        identity = [item["ecosystem"], item["name"], item["version"], item.get("revision", "")]
        key = hashlib.sha256(json.dumps(identity).encode()).hexdigest()[:20]
        if key not in merged:
            merged[key] = {"id": key, **{field: item.get(field) if isinstance(item.get(field), str) else "" for field in ["name", "version", "ecosystem", "description", "homepage", "repository", "revision"]},
                           "license": item.get("license"), "scopes": [], "files": []}
        package = merged[key]
        package["scopes"] = sorted(set(package["scopes"] + item["scopes"]))
        for field in ["license", "description", "homepage", "repository"]:
            if not package[field] and isinstance(item.get(field), str) and item[field]:
                package[field] = item[field]
        for entry in item.get("files", []):
            if entry not in package["files"]:
                package["files"].append(entry)
    details = {char: {} for char in "0123456789abcdef"}
    result = []
    for package in sorted(merged.values(), key=lambda item: (item["name"].casefold(), item["version"], item["id"])):
        files = sorted(package.pop("files"), key=lambda item: (item["name"], item["text"]))
        package["licenseFileCount"] = len(files)
        details[package["id"][0]][package["id"]] = {"files": files}
        result.append(package)
    index = {"schemaVersion": 1, "scopes": sorted(scopes, key=lambda item: item["id"]), "packages": result, "notices": list(dict.fromkeys(BASE_NOTICES + notices))}
    (output / "licenses").mkdir(parents=True, exist_ok=True)
    for char, bucket in details.items():
        (output / "licenses" / f"{char}.json").write_text(json.dumps(bucket, ensure_ascii=False, separators=(",", ":")) + "\n", encoding="utf-8")
    (output / "index.json").write_text(json.dumps(index, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return index


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument("--output", type=Path)
    parser.add_argument("--scope", action="append", choices=["rust", *WEB_PROJECTS, "sdk-tool", "ext-core", "assets", "gateway"])
    parser.add_argument("--cargo-metadata", type=Path)
    parser.add_argument("--fetch-metadata", action="store_true")
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()
    repo = args.repo.resolve()
    output = args.output or repo / "src/frame/desktop/public/open-source"
    old_packages, old_scopes, _ = read_existing(output)
    selected = set(args.scope or ["rust", *WEB_PROJECTS, "sdk-tool", "ext-core", "assets", "gateway"])
    refreshed = lambda scope: scope in selected or ("ext-core" in selected and scope.startswith("ext-core"))
    packages = [{**p, "scopes": [s for s in p["scopes"] if not refreshed(s)]} for p in old_packages]
    packages = [p for p in packages if p["scopes"]]
    scopes = [{**scope, "isSnapshot": True} for scope in old_scopes if not refreshed(scope["id"])]
    notices = []
    for scope in WEB_PROJECTS:
        if scope in selected:
            collected, status, notes = collect_web(repo, scope)
            packages.extend(collected)
            status["notices"] = notes
            status["isSnapshot"] = False
            scopes.append(status)
            notices.extend(notes)
    if "rust" in selected:
        collected, status, notes = collect_cargo(repo, args.cargo_metadata, args.offline)
        packages.extend(collected)
        status["notices"] = notes
        status["isSnapshot"] = False
        scopes.append(status)
        notices.extend(notes)
    if "gateway" in selected:
        collected, status, notes = collect_gateway(repo, collect_cargo, args.offline)
        packages.extend(collected)
        status["notices"] = notes
        scopes.append(status)
        notices.extend(notes)
    if "sdk-tool" in selected:
        collected, status, notes = collect_sdk(repo, old_packages + packages, args.fetch_metadata)
        packages.extend(collected)
        status["notices"] = notes
        status["isSnapshot"] = False
        scopes.append(status)
        notices.extend(notes)
    if "ext-core" in selected:
        collected, statuses, notes = collect_ext_core(repo)
        packages.extend(collected)
        for status in statuses:
            status["isSnapshot"] = False
            status["notices"] = notes
        scopes.extend(statuses)
        notices.extend(notes)
    if "assets" in selected:
        collected, status, notes = collect_assets(repo)
        packages.extend(collected)
        status["notices"] = notes
        status["isSnapshot"] = False
        scopes.append(status)
        notices.extend(notes)
    complete_licenses(packages, old_packages, args.fetch_metadata)
    for scope in scopes:
        missing = sorted({f"{p['name']}@{p['version']}" for p in packages if scope["id"] in p["scopes"] and not p.get("license")})
        if missing:
            if scope["status"] == "complete":
                scope["status"] = "partial"
            scope.setdefault("notices", []).append(f"{scope['label']}: license metadata is unavailable for {', '.join(missing)}.")
        scope["notices"] = list(dict.fromkeys(scope.get("notices", [])))
    notices = [note for scope in scopes for note in scope.get("notices", [])]
    index = write_inventory(output, packages, scopes, notices)
    print(f"Open-source inventory: {len(index['packages'])} packages, {len(index['scopes'])} sources.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
