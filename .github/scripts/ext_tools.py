#!/usr/bin/env python3
"""Keep ext-core's facts in one piece and resolve its release (ExtTools TODO §3.10, T2a / T2b).

check-table   doc/app_service/paios容器需求.md §6.4 table B <-> ext-manifest.json provides <-> what the build pins
resolve       the release manifest of the pinned version from R2, every tar checked with HEAD, as bin/ext-tools/ext_tools.json
latest        latest.json of a release, for ext-core-latest and R2
"""
import argparse
import json
from pathlib import Path
import re
import sys
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
EXT_CORE = ROOT / "publish" / "exttool" / "ext-core"
TABLE_DOC = ROOT / "doc" / "app_service" / "paios容器需求.md"
SCHEMA = "buckyos.ext-tools/v1"
PLATFORMS = ("linux-x86_64", "linux-aarch64", "darwin-arm64", "darwin-x86_64", "windows-x86_64")
DEFAULT_BASE_URL = "https://buckyos-installer.tbudr.top"
# A row of table B names its tool in the source column: who has to pin it.
CONDA_TOOLS = {"ffmpeg", "imagemagick", "poppler", "tesseract", "curl", "xz", "git", "tree", "unzip", "avahi"}
OFFICIAL_TOOLS = {"pandoc": "pandoc-", "python": "cpython-", "uv": "uv-", "node": "node-", "deno": "deno-", "jq": "jq-", "mingit": "MinGit-", "busybox": "busybox-", "libreoffice": "LibreOffice_", "noto-cjk": "NotoSansCJK-"}


def table_commands(text):
    """The command column of table B."""
    start = text.index("**表 B")
    end = text.index("**表 C", start)
    commands = set()
    for line in text[start:end].splitlines():
        cells = line.split("|")
        if len(cells) < 4 or not line.startswith("|") or cells[1].strip() in ("命令", "---"):
            continue
        commands.update(re.findall(r"`([^`\s]+)`", re.sub(r"（.*?）", "", cells[1])))
    return commands


def check_table(table_text, manifest, conda_env_text, sources_text):
    errors = []
    documented = table_commands(table_text)
    provided = {p["command"] for p in manifest["provides"]}
    for name in sorted(documented - provided):
        errors.append(f"table B lists `{name}`, ext-manifest.json does not provide it")
    for name in sorted(provided - documented):
        errors.append(f"ext-manifest.json provides `{name}`, table B does not list it")
    for tool in sorted(CONDA_TOOLS):
        if not re.search(rf"^\s*-\s*{re.escape(tool)}\b", conda_env_text, re.M):
            errors.append(f"conda-env.yml does not ask for {tool}")
    for tool, prefix in sorted(OFFICIAL_TOOLS.items()):
        if not re.search(rf"\s{re.escape(prefix)}", sources_text):
            errors.append(f"sources.sha256 pins nothing for {tool}")
    return errors


def request_of(url, method="GET"):
    # The host is behind Cloudflare, which refuses urllib's default User-Agent (403, error 1010).
    return urllib.request.Request(url, method=method, headers={"Cache-Control": "no-cache", "User-Agent": "buckyos-ext-tools/1"})


def fetch_json(url):
    with urllib.request.urlopen(request_of(url), timeout=60) as response:
        return json.loads(response.read())


def head_size(url):
    with urllib.request.urlopen(request_of(url, "HEAD"), timeout=60) as response:
        return int(response.headers.get("Content-Length", -1))


def resolve(version, base_url, fetch=fetch_json, head=head_size):
    """`fetch` and `head` raising means the version is not released, and a main build must then fail."""
    release = fetch(f"{base_url}/ext-tools/ext-core/{version}/manifest.json")
    if release.get("schema") != SCHEMA:
        raise ValueError(f"{version}: schema is {release.get('schema')}")
    package = release["packages"]["ext-core"]
    if package.get("version") != version:
        raise ValueError(f"{version}: the release says {package.get('version')}")
    missing = [p for p in PLATFORMS if p not in package["platforms"]]
    if missing:
        raise ValueError(f"{version}: no payload for {', '.join(missing)}")
    for name, entry in package["platforms"].items():
        source = entry.get("corresponding_source")
        if not isinstance(source, dict):
            raise ValueError(f"{version}/{name}: no corresponding source archive")
        if not re.fullmatch(r"sha256:[0-9a-f]{64}", source.get("manifest_sha256", "")):
            raise ValueError(f"{version}/{name}: no valid source manifest checksum")
        for artifact in (entry, source):
            path = artifact.get("path", "")
            if not path.startswith(f"ext-tools/ext-core/{version}/{name}/") or ".." in path.split("/"):
                raise ValueError(f"{version}/{name}: invalid artifact path: {path}")
            if not re.fullmatch(r"sha256:[0-9a-f]{64}", artifact.get("sha256", "")):
                raise ValueError(f"{version}/{name}: invalid artifact checksum")
            if not isinstance(artifact.get("size"), int) or artifact["size"] <= 0:
                raise ValueError(f"{version}/{name}: invalid artifact size")
            artifact["url"] = f"{base_url.rstrip('/')}/{path}"
            size = head(artifact["url"])
            if size != artifact["size"]:
                raise ValueError(f"{artifact['url']}: {size} bytes, the release says {artifact['size']}")
    return release


def latest_of(version, release):
    package = release["packages"]["ext-core"]
    return {"schema": SCHEMA, "version": version, "pikg_digest": package.get("pikg_digest"), "platforms": package["platforms"]}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("check-table")
    resolve_parser = commands.add_parser("resolve")
    resolve_parser.add_argument("--version")
    resolve_parser.add_argument("--base-url", default=DEFAULT_BASE_URL)
    resolve_parser.add_argument("--output", type=Path, required=True)
    latest_parser = commands.add_parser("latest")
    latest_parser.add_argument("--version", required=True)
    latest_parser.add_argument("--manifest", type=Path, required=True)
    latest_parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.command == "check-table":
        errors = check_table(
            TABLE_DOC.read_text(encoding="utf-8"),
            json.loads((EXT_CORE / "ext-manifest.json").read_text()),
            (EXT_CORE / "conda-env.yml").read_text(),
            (EXT_CORE / "sources.sha256").read_text(),
        )
        for error in errors:
            print(error, file=sys.stderr)
        return 1 if errors else 0
    if args.command == "latest":
        release = json.loads(args.manifest.read_text())
        args.output.write_text(json.dumps(latest_of(args.version, release), indent=2) + "\n")
        return 0
    version = args.version or json.loads((EXT_CORE / "ext-manifest.json").read_text())["version"]
    try:
        release = resolve(version, args.base_url)
    except (urllib.error.URLError, OSError, ValueError, KeyError) as error:
        print(f"ext-core {version} is not released: {error}. Publish it (ext-core.yml) before raising the version.", file=sys.stderr)
        return 1
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(release, indent=2) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
