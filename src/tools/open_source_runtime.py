import csv
import json
import os
import platform
import re
import subprocess
import xml.etree.ElementTree as ET
from pathlib import Path


def native_platform() -> str:
    system = {"Linux": "linux", "Darwin": "darwin", "Windows": "windows"}.get(platform.system(), "")
    machine = platform.machine().lower()
    arch = {"amd64": "x86_64", "x86_64": "x86_64", "aarch64": "aarch64", "arm64": "aarch64"}.get(machine, "")
    if system == "darwin" and arch == "aarch64":
        arch = "arm64"
    return f"{system}-{arch}"


def license_files(paths: list[Path]) -> list[dict]:
    files = []
    seen = set()
    for path in paths:
        if not path.is_file():
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        if text and text not in seen:
            files.append({"name": path.name, "text": text})
            seen.add(text)
    return files


def package(name: str, version: str, ecosystem: str, scope: str, license_id=None, files=None, homepage="", repository="") -> dict:
    return {
        "name": name,
        "version": version,
        "ecosystem": ecosystem,
        "license": license_id,
        "description": "",
        "homepage": homepage,
        "repository": repository,
        "scopes": [scope],
        "files": files or [],
    }


def probe_version(path: Path, pattern: str, arguments=None) -> str | None:
    if not path.is_file():
        return None
    try:
        output = subprocess.run(
            [str(path.resolve()), *(arguments or ["--version"])],
            capture_output=True,
            text=True,
            timeout=10,
            check=True,
            env={**os.environ, "DENO_NO_UPDATE_CHECK": "1", "UV_NO_PROGRESS": "1"},
        ).stdout
    except (OSError, subprocess.SubprocessError):
        return None
    match = re.search(pattern, output)
    return match.group(1) if match else None


def libreoffice_version(store: Path) -> str | None:
    registry = store / "libreoffice/share/registry/main.xcd"
    if not registry.is_file():
        return None
    try:
        root = ET.parse(registry).getroot()
    except (OSError, ET.ParseError):
        return None
    for element in root.iter("prop"):
        if element.get("{http://openoffice.org/2001/registry}name") == "ooSetupVersionAboutBox":
            value = element.findtext("value")
            if value and re.fullmatch(r"\d+(?:\.\d+)+", value):
                return value
    return None


def collect_ext_core(repo: Path) -> tuple[list[dict], list[dict], list[str]]:
    source = repo / "publish/exttool/ext-core"
    manifest_file = source / "ext-manifest.json"
    if not manifest_file.is_file():
        return [], [], []
    manifest = json.loads(manifest_file.read_text(encoding="utf-8"))
    packages = []
    scopes = []
    notices = []
    for target in sorted(manifest.get("platforms", [])):
        scope = f"ext-core-{target}"
        store = source / "payload" / target / "store/ext-core"
        metadata = store / "LICENSES/conda-forge.tsv"
        status = "partial" if store.is_dir() else "unavailable"
        scopes.append({"id": scope, "label": f"Extension tools ({target})", "status": status})
        if not store.is_dir():
            notices.append(f"Extension tool payload for {target} is unavailable; its dependencies are not included.")
            continue
        if metadata.is_file():
            with metadata.open(encoding="utf-8", newline="") as stream:
                for row in csv.DictReader(stream, delimiter="\t"):
                    if not row.get("name") or not row.get("version"):
                        continue
                    name = row["name"]
                    candidates = []
                    for directory in (store / "conda/share/licenses" / name, store / "conda/share" / name):
                        if directory.is_dir():
                            candidates.extend(p for p in directory.iterdir() if p.name.upper().startswith(("LICENSE", "COPYING", "NOTICE")))
                    packages.append(package(
                        name, row["version"], "conda", scope,
                        row.get("license") or None, license_files(sorted(candidates)),
                        homepage=f"https://anaconda.org/conda-forge/{name}",
                    ))
        else:
            notices.append(f"Extension tool payload for {target} has no conda dependency license inventory.")

        office_version = libreoffice_version(store)
        if office_version:
            office_files = license_files([store / "LICENSES/LibreOffice-LICENSE", store / "libreoffice/LICENSE"])
            office_license = "SEE LICENSE IN " + office_files[0]["name"] if office_files else None
            packages.append(package("libreoffice", office_version, "runtime", scope, office_license, office_files))

        npm_path = store / ("node/node_modules/npm/package.json" if target.startswith("windows-") else "node/lib/node_modules/npm/package.json")
        if npm_path.is_file():
            npm = json.loads(npm_path.read_text(encoding="utf-8"))
            if npm.get("version"):
                npm_license = npm.get("license")
                packages.append(package("npm", npm["version"], "runtime", scope,
                                        npm_license if isinstance(npm_license, str) else None,
                                        license_files([npm_path.parent / "LICENSE"])))

        missing = []
        runtimes = [
            ("pandoc", "pandoc/bin/pandoc", r"^pandoc (\S+)", [], None),
            ("python", "python/bin/python3", r"^Python (\S+)", list(store.glob("python/lib/python*/LICENSE.txt")) + [store / "python/LICENSE.txt"], ["-I", "--version"]),
            ("uv", "uv/uv", r"^uv (\S+)", list((store / "uv").glob("LICENSE*")), None),
            ("node", "node/bin/node", r"^v(\d+\.\d+\.\d+)\s", [store / "node/LICENSE"], None),
            ("deno", "deno/deno", r"^deno (\S+)", list((store / "deno").glob("LICENSE*")), None),
            ("jq", "jq/jq", r"^jq-(\S+)", list((store / "jq").glob("COPYING*")), None),
        ]
        for name, relative, pattern, paths, arguments in runtimes:
            if target.startswith("windows-"):
                relative = {"python": "python/python", "node": "node/node"}.get(name, relative) + ".exe"
            version = probe_version(store / relative, pattern, arguments) if target == native_platform() else None
            if version:
                files = license_files(paths)
                license_id = "SEE LICENSE IN " + files[0]["name"] if files else None
                packages.append(package(name, version, "runtime", scope, license_id, files))
            else:
                missing.append(name)
        if not office_version:
            missing.append("libreoffice")
        if missing:
            notices.append(f"Extension tool versions could not be verified for {target}: {', '.join(missing)}.")
    if any(s["status"] != "unavailable" for s in scopes):
        notices.append("Extension tool entries describe the available build payloads. Tools installed or upgraded separately, bundled font versions, and pre-warmed container caches are not fully inventoried.")
    return packages, scopes, notices
