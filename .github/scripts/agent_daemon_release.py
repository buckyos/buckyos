#!/usr/bin/env python3
"""Assemble GitHub Release payloads for the standalone BuckyOS Agent Daemon.

Input: the merged build artifacts of .github/workflows/agent-daemon.yml, one archive
plus `.sha256` per target and the Linux launcher. Output: two asset directories.

  <output>/versioned/assets   immutable release `agent-daemon-v<version>`
  <output>/latest/assets      rolling release `agent-daemon-latest` (unversioned
                              file names, so links never change) plus latest.json

Both get `notes.md` beside `assets/`. latest.json names the current version and the
versioned download URL and SHA-256 per target; the Linux launcher reads it.
"""

import argparse
import datetime
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import sys

ROOT = Path(__file__).resolve().parents[2]
LATEST_TAG = "agent-daemon-latest"
LAUNCHER = "start-buckyos-agent-daemon.sh"
PLATFORM_LABELS = {
    "x86_64-unknown-linux-musl": "Linux x86_64 (static musl)",
    "aarch64-unknown-linux-gnu": "Linux ARM64 (glibc)",
    "aarch64-apple-darwin": "macOS Apple silicon",
    "x86_64-apple-darwin": "macOS Intel",
    "x86_64-pc-windows-msvc": "Windows x64",
    "aarch64-pc-windows-msvc": "Windows ARM64",
}


def load_packager():
    script = ROOT / "src/tools/package_agent_daemon.py"
    spec = importlib.util.spec_from_file_location("package_agent_daemon", script)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def release_tag(version: str) -> str:
    return f"agent-daemon-v{version}"


def unversioned_name(target: str, name: str, version: str) -> str:
    return name.replace(f"-{version}-", "-", 1)


def read_checksum(archive: Path) -> str:
    checksum = archive.with_name(archive.name + ".sha256")
    if not checksum.is_file():
        raise ValueError(f"missing checksum file {checksum.name}")
    recorded = checksum.read_text(encoding="ascii").split()
    if len(recorded) != 2 or recorded[1] != archive.name or not re.fullmatch(r"[0-9a-f]{64}", recorded[0]):
        raise ValueError(f"malformed checksum file {checksum.name}")
    actual = hashlib.sha256(archive.read_bytes()).hexdigest()
    if actual != recorded[0]:
        raise ValueError(f"checksum mismatch for {archive.name}")
    return actual


def collect(dist: Path, packager, *, allow_missing: bool) -> tuple[str, dict]:
    assets = {}
    missing = []
    versions = set()
    for target in packager.TARGETS:
        archive = packager.find_archive(dist, target)
        if archive is None:
            missing.append(target)
            continue
        assets[target] = {"file": archive.name, "path": archive, "sha256": read_checksum(archive)}
        versions.add(packager.archive_version(archive, target))
    if missing and not allow_missing:
        raise ValueError("missing release archives for: " + ", ".join(missing))
    if not assets:
        raise ValueError("no release archives found")
    if len(versions) != 1:
        raise ValueError("release archives have different versions: " + ", ".join(sorted(versions)))
    unexpected = sorted(
        entry.name for entry in dist.iterdir()
        if entry.is_file() and entry.name not in {LAUNCHER, *(a["file"] for a in assets.values()), *(a["file"] + ".sha256" for a in assets.values())}
    )
    if unexpected:
        raise ValueError("unexpected files in dist: " + ", ".join(unexpected))
    if not (dist / LAUNCHER).is_file():
        raise ValueError(f"missing {LAUNCHER}")
    return versions.pop(), assets


def write_checksum(path: Path) -> str:
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    path.with_name(path.name + ".sha256").write_text(f"{digest}  {path.name}\n", encoding="ascii")
    return digest


def write_sums(directory: Path, names: list[str]) -> None:
    lines = [f"{hashlib.sha256((directory / name).read_bytes()).hexdigest()}  {name}\n" for name in sorted(names)]
    (directory / "SHA256SUMS").write_text("".join(lines), encoding="ascii")


def notes(version: str, commit: str, repo_url: str, assets: dict, *, rolling: bool) -> str:
    tag = release_tag(version)
    lines = []
    if rolling:
        lines += [f"# BuckyOS Agent Daemon (latest: {version})", "",
                  "Rolling release. The file names below never change, so they can be linked from "
                  f"documentation and scripts; they currently resolve to [{tag}]({repo_url}/releases/tag/{tag}). "
                  "`latest.json` names the current version with per-target SHA-256 digests.", ""]
    else:
        lines += [f"# BuckyOS Agent Daemon {version}", "",
                  "Standalone foreground execution endpoint for a BuckyOS Zone, running as the current OS user on "
                  "Linux, macOS and Windows. No full Node, Docker or Node.js is required.", ""]
    lines += [f"Built by GitHub Actions from `{commit}`.", "", "## Downloads", "",
              "| Platform | File | SHA-256 |", "| --- | --- | --- |"]
    for target, asset in assets.items():
        name = unversioned_name(target, asset["file"], version) if rolling else asset["file"]
        url = f"{repo_url}/releases/download/{LATEST_TAG if rolling else tag}/{name}"
        lines.append(f"| {PLATFORM_LABELS.get(target, target)} | [{name}]({url}) | `{asset['sha256']}` |")
    launcher_tag = LATEST_TAG if rolling else tag
    lines += ["", f"Linux launcher: [{LAUNCHER}]({repo_url}/releases/download/{launcher_tag}/{LAUNCHER}). "
              "`SHA256SUMS` lists every asset.", "", "## Run", "",
              "Linux (downloads the archive named by latest.json, verifies it, runs in the foreground):", "", "```sh",
              f"curl -fsSLO {repo_url}/releases/download/{LATEST_TAG}/{LAUNCHER}",
              f"sh {LAUNCHER} run --url-stdin   # paste the access URL issued by Settings → Nodes/Devices",
              "```", "",
              "macOS: unpack the tar.gz, then `./buckyos_agent_daemon doctor --json` in the logged-in user's Terminal. "
              "Grant Screen Recording and Accessibility to the terminal that launches it.", "",
              "Windows: unpack the ZIP in the current user's PowerShell, then `.\\buckyos_agent_daemon.exe doctor --json`. "
              "The executable statically links the MSVC runtime; no VC++ Redistributable is needed.", "",
              "## Integrity", "",
              "Each archive ships with a `.sha256` file and `manifest.json` records the binary digest and runtime "
              "requirements. Binaries are not code-signed or notarized; macOS Gatekeeper and Windows SmartScreen will warn.", ""]
    return "\n".join(lines)


def assemble(dist: Path, output: Path, *, repo_url: str, commit: str, tag: str | None, allow_missing: bool) -> dict:
    packager = load_packager()
    version, assets = collect(dist, packager, allow_missing=allow_missing)
    expected = release_tag(version)
    if tag is not None and tag != expected:
        raise ValueError(f"tag {tag} does not match archive version {version} (expected {expected})")
    if output.exists():
        shutil.rmtree(output)
    versioned = output / "versioned" / "assets"
    latest = output / "latest" / "assets"
    versioned.mkdir(parents=True)
    latest.mkdir(parents=True)
    published = datetime.datetime.now(datetime.timezone.utc).replace(microsecond=0).isoformat()
    manifest = {
        "schema_version": 1,
        "product": "buckyos-agent-daemon",
        "version": version,
        "tag": expected,
        "commit": commit,
        "published_at": published,
        "assets": {},
    }
    versioned_names, latest_names = [LAUNCHER], [LAUNCHER]
    for target, asset in assets.items():
        shutil.copyfile(asset["path"], versioned / asset["file"])
        shutil.copyfile(asset["path"].with_name(asset["file"] + ".sha256"), versioned / (asset["file"] + ".sha256"))
        stable = unversioned_name(target, asset["file"], version)
        shutil.copyfile(asset["path"], latest / stable)
        write_checksum(latest / stable)
        versioned_names.append(asset["file"])
        latest_names.append(stable)
        manifest["assets"][target] = {
            "file": asset["file"],
            "url": f"{repo_url}/releases/download/{expected}/{asset['file']}",
            "stable_url": f"{repo_url}/releases/download/{LATEST_TAG}/{stable}",
            "sha256": asset["sha256"],
        }
    for directory in (versioned, latest):
        shutil.copyfile(dist / LAUNCHER, directory / LAUNCHER)
        (directory / LAUNCHER).chmod(0o755)
        (directory / "latest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    write_sums(versioned, versioned_names + ["latest.json"])
    write_sums(latest, latest_names + ["latest.json"])
    (output / "versioned" / "notes.md").write_text(notes(version, commit, repo_url, assets, rolling=False), encoding="utf-8")
    (output / "latest" / "notes.md").write_text(notes(version, commit, repo_url, assets, rolling=True), encoding="utf-8")
    return {"version": version, "tag": expected, "latest_tag": LATEST_TAG, "targets": sorted(assets)}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--dist", type=Path, required=True, help="directory with the merged build artifacts")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--repo-url", default="https://github.com/buckyos/buckyos")
    parser.add_argument("--commit", default=os.environ.get("GITHUB_SHA", "unknown"))
    parser.add_argument("--tag", help="pushed tag; must equal agent-daemon-v<archive version>, including +buildYYMMDD")
    parser.add_argument("--allow-missing", action="store_true", help="publish even if some targets did not build")
    args = parser.parse_args(argv)
    try:
        result = assemble(args.dist, args.output, repo_url=args.repo_url.rstrip("/"), commit=args.commit,
                          tag=args.tag, allow_missing=args.allow_missing)
    except (OSError, ValueError) as error:
        print(f"::error::{error}", file=sys.stderr)
        return 1
    github_output = os.environ.get("GITHUB_OUTPUT")
    if github_output:
        with open(github_output, "a", encoding="utf-8") as stream:
            for key in ("version", "tag", "latest_tag"):
                stream.write(f"{key}={result[key]}\n")
    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
