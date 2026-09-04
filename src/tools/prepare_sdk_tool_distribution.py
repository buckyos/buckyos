#!/usr/bin/env python3

import argparse
import base64
import hashlib
import json
import os
import platform
import shutil
import subprocess
import sys
import tarfile
from pathlib import Path, PurePosixPath
from urllib.parse import quote


PACKAGE_SPEC = "buckyos@latest"
REPO_ROOT = Path(__file__).resolve().parents[2]


def digest(path: Path, algorithm: str = "sha256") -> str:
    value = hashlib.new(algorithm)
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )


def npm_command(args: list[str]) -> list[str]:
    if os.name == "nt":
        return [
            os.environ.get("ComSpec", "cmd.exe"),
            "/d",
            "/s",
            "/c",
            "npm.cmd",
            *args,
        ]
    return ["npm", *args]


def prepare_npm_artifact(work_dir: Path) -> tuple[Path, dict]:
    artifact_dir = work_dir / "npm"
    artifact_dir.mkdir(parents=True, exist_ok=True)
    completed = subprocess.run(
        npm_command(
            [
                "pack",
                PACKAGE_SPEC,
                "--json",
                "--ignore-scripts",
                "--pack-destination",
                str(artifact_dir),
            ]
        ),
        check=True,
        capture_output=True,
        text=True,
        env=os.environ.copy(),
    )
    try:
        result = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise ValueError(f"npm pack returned invalid JSON: {completed.stdout}") from error
    if not isinstance(result, list) or len(result) != 1 or not isinstance(result[0], dict):
        raise ValueError("npm pack returned an unexpected result")

    metadata = result[0]
    filename = Path(str(metadata.get("filename", ""))).name
    if not filename:
        raise ValueError("npm pack did not report an artifact filename")
    artifact = artifact_dir / filename
    if not artifact.is_file():
        raise FileNotFoundError(f"npm pack artifact is missing: {artifact}")

    calculated_integrity = "sha512-" + base64.b64encode(
        bytes.fromhex(digest(artifact, "sha512"))
    ).decode("ascii")
    reported_integrity = str(metadata.get("integrity", ""))
    if reported_integrity and reported_integrity != calculated_integrity:
        raise ValueError("npm artifact integrity differs from npm pack metadata")
    reported_shasum = str(metadata.get("shasum", ""))
    if reported_shasum and reported_shasum != digest(artifact, "sha1"):
        raise ValueError("npm artifact SHA-1 differs from npm pack metadata")
    return artifact, metadata


def npm_file_manifest(tarball: Path) -> tuple[list[dict], dict]:
    files = []
    package_json = None
    seen = set()
    with tarfile.open(tarball, "r:gz") as archive:
        for member in archive.getmembers():
            path = PurePosixPath(member.name)
            if not path.parts or path.parts[0] != "package":
                raise ValueError(f"unexpected npm tarball entry: {member.name}")
            relative = PurePosixPath(*path.parts[1:])
            if not relative.parts or relative.is_absolute() or ".." in relative.parts:
                raise ValueError(f"unsafe npm tarball entry: {member.name}")
            if member.issym() or member.islnk() or not (member.isdir() or member.isfile()):
                raise ValueError(f"unsupported npm tarball entry: {member.name}")
            if member.isdir():
                continue
            relative_name = relative.as_posix()
            if relative_name in seen:
                raise ValueError(f"duplicate npm tarball entry: {member.name}")
            seen.add(relative_name)
            source = archive.extractfile(member)
            if source is None:
                raise ValueError(f"cannot read npm tarball entry: {member.name}")
            with source:
                content = source.read()
            files.append(
                {
                    "path": relative_name,
                    "size": len(content),
                    "sha256": hashlib.sha256(content).hexdigest(),
                }
            )
            if relative_name == "package.json":
                package_json = json.loads(content.decode("utf-8"))
    if package_json is None:
        raise ValueError("npm artifact does not contain package.json")
    files.sort(key=lambda item: item["path"])
    return files, package_json


def npm_purl(name: str, version: str) -> str:
    if name.startswith("@"):
        scope, package_name = name[1:].split("/", 1)
        encoded_name = f"%40{quote(scope, safe='')}/{quote(package_name, safe='')}"
    else:
        encoded_name = quote(name, safe="")
    return f"pkg:npm/{encoded_name}@{quote(version, safe='')}"


def create_sbom(
    tarball: Path,
    package_name: str,
    package_version: str,
    deno: Path,
    deno_version: str,
) -> dict:
    root_ref = npm_purl(package_name, package_version)
    deno_ref = f"pkg:generic/deno@{quote(deno_version, safe='')}"
    return {
        "bomFormat": "CycloneDX",
        "specVersion": "1.6",
        "version": 1,
        "metadata": {
            "component": {
                "type": "application",
                "bom-ref": root_ref,
                "name": package_name,
                "version": package_version,
                "purl": root_ref,
                "hashes": [
                    {"alg": "SHA-256", "content": digest(tarball)},
                    {"alg": "SHA-512", "content": digest(tarball, "sha512")},
                ],
                "properties": [
                    {"name": "buckyos:distribution", "value": "npm-and-system"}
                ],
            }
        },
        "components": [
            {
                "type": "application",
                "bom-ref": deno_ref,
                "name": "deno",
                "version": deno_version,
                "purl": deno_ref,
                "hashes": [{"alg": "SHA-256", "content": digest(deno)}],
                "properties": [
                    {
                        "name": "buckyos:distribution",
                        "value": "system-only-runtime",
                    }
                ],
            }
        ],
        "dependencies": [
            {"ref": root_ref, "dependsOn": [deno_ref]},
            {"ref": deno_ref, "dependsOn": []},
        ],
    }


def installed_deno() -> tuple[Path, str]:
    executable = shutil.which("deno")
    if executable is None:
        raise FileNotFoundError("deno is not installed or is missing from PATH")
    deno = Path(executable).resolve(strict=True)
    output = subprocess.run(
        [str(deno), "--version"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.splitlines()
    fields = output[0].split() if output else []
    if len(fields) < 2 or fields[0] != "deno":
        raise ValueError(f"unable to read Deno version from: {deno}")
    return deno, fields[1]


def git_commit() -> str:
    return subprocess.run(
        ["git", "-C", str(REPO_ROOT), "rev-parse", "HEAD"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()


def build(work_dir: Path) -> None:
    work_dir = work_dir.resolve()
    deno, deno_version = installed_deno()
    tarball, npm_metadata = prepare_npm_artifact(work_dir)
    files, package_json = npm_file_manifest(tarball)
    package_name = str(package_json.get("name", ""))
    package_version = str(package_json.get("version", ""))
    if package_name != "buckyos" or not package_version:
        raise ValueError("npm artifact is not a versioned buckyos package")
    if npm_metadata.get("name") and npm_metadata["name"] != package_name:
        raise ValueError("npm package name differs from npm pack metadata")
    if npm_metadata.get("version") and npm_metadata["version"] != package_version:
        raise ValueError("npm package version differs from npm pack metadata")

    files_digest = hashlib.sha256(
        json.dumps(files, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
    ).hexdigest()
    sbom_path = work_dir / "sbom.cdx.json"
    write_json(
        sbom_path,
        create_sbom(tarball, package_name, package_version, deno, deno_version),
    )
    release = {
        "schema_version": 1,
        "buckyos_version": (REPO_ROOT / "src" / "VERSION").read_text(encoding="utf-8").strip(),
        "build_id": f"{git_commit()}:{sys.platform}:{platform.machine().lower()}",
        "tool_version": package_version,
        "sdk_version": package_version,
        "npm_tarball_sha256": digest(tarball),
        "npm_integrity": "sha512-"
        + base64.b64encode(bytes.fromhex(digest(tarball, "sha512"))).decode("ascii"),
        "npm_files_sha256": files_digest,
        "npm_files": files,
        "deno_version": deno_version,
        "deno_sha256": digest(deno),
        "sbom_sha256": digest(sbom_path),
        "protocol_version": "1",
        "capability_range": "buckyos.tool.v1",
    }
    release_path = work_dir / "release.json"
    write_json(release_path, release)

    command = [
        sys.executable,
        str(REPO_ROOT / "src" / "tools" / "build_sdk_tool_distribution.py"),
        "--artifact",
        str(tarball),
        "--release-manifest",
        str(release_path),
        "--deno",
        str(deno),
        "--deno-version",
        deno_version,
        "--sbom",
        str(sbom_path),
        "--rootfs",
        str(REPO_ROOT / "src" / "rootfs"),
    ]
    if os.name == "nt":
        command.append("--windows")
    print(
        f"[sdk-tool] package={package_name}@{package_version} "
        f"deno={deno_version} native={sys.platform}/{platform.machine().lower()}",
        flush=True,
    )
    subprocess.run(command, check=True)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Prepare the native BuckyOS SDK/Tool system distribution"
    )
    parser.add_argument("--work-dir", type=Path, required=True)
    return parser.parse_args()


if __name__ == "__main__":
    arguments = parse_args()
    build(arguments.work_dir)
