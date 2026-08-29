#!/usr/bin/env python3

import argparse
import base64
import hashlib
import json
import os
import shutil
import stat
import subprocess
import tarfile
import tempfile
from pathlib import Path, PurePosixPath


ENVIRONMENT_NAMES = (
    "HOME,USERPROFILE,APPDATA,BUCKYOS_TOOL_CONFIG_DIR,BUCKYOS_TOOL_PROFILE,"
    "BUCKYOS_TOOL_ZONE,BUCKYOS_TOOL_ENDPOINT,BUCKYOS_TOOL_IDENTITY,"
    "BUCKYOS_TOOL_OUTPUT,BUCKYOS_IDENTITY_ROOT,BUCKYOS_SECURITY_ROOT,"
    "BUCKYOS_APPCLIENT_SESSION_TOKEN,BUCKYOS_ROOT,SOURCE_DATE_EPOCH"
)


def digest(path: Path, algorithm: str = "sha256") -> str:
    value = hashlib.new(algorithm)
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def load_manifest(path: Path) -> dict:
    value = json.loads(path.read_text(encoding="utf-8"))
    required = {
        "schema_version",
        "buckyos_version",
        "build_id",
        "tool_version",
        "sdk_version",
        "npm_tarball_sha256",
        "npm_integrity",
        "npm_files_sha256",
        "npm_files",
        "deno_version",
        "deno_sha256",
        "sbom_sha256",
        "protocol_version",
        "capability_range",
    }
    missing = sorted(required - value.keys())
    if missing:
        raise ValueError(f"release manifest is missing: {', '.join(missing)}")
    if value["schema_version"] != 1:
        raise ValueError("unsupported release manifest schema")
    return value


def verify_artifacts(
    tarball: Path,
    deno: Path,
    sbom: Path,
    release: dict,
    declared_deno_version: str | None = None,
) -> None:
    tarball_sha256 = digest(tarball)
    if tarball_sha256 != release["npm_tarball_sha256"]:
        raise ValueError("npm tarball SHA-256 differs from the release manifest")
    integrity = "sha512-" + base64.b64encode(
        bytes.fromhex(digest(tarball, "sha512"))
    ).decode("ascii")
    if integrity != release["npm_integrity"]:
        raise ValueError("npm tarball integrity differs from the release manifest")
    if digest(deno) != release["deno_sha256"]:
        raise ValueError("Deno binary SHA-256 differs from the release manifest")
    if digest(sbom) != release["sbom_sha256"]:
        raise ValueError("SBOM SHA-256 differs from the release manifest")
    if declared_deno_version is None:
        output = subprocess.run(
            [str(deno), "--version"], check=True, capture_output=True, text=True
        ).stdout.splitlines()
        observed = output[0].split()[1] if output else ""
    else:
        observed = declared_deno_version
    if observed != release["deno_version"]:
        raise ValueError(
            f"Deno version differs from the release manifest: {observed}"
        )


def extract_npm_tarball(tarball: Path, destination: Path) -> None:
    with tarfile.open(tarball, "r:gz") as archive:
        members = archive.getmembers()
        for member in members:
            path = PurePosixPath(member.name)
            if not path.parts or path.parts[0] != "package":
                raise ValueError(f"unexpected npm tarball entry: {member.name}")
            relative = PurePosixPath(*path.parts[1:])
            if not relative.parts or relative.is_absolute() or ".." in relative.parts:
                raise ValueError(f"unsafe npm tarball entry: {member.name}")
            if member.issym() or member.islnk() or not (member.isdir() or member.isfile()):
                raise ValueError(f"unsupported npm tarball entry: {member.name}")
            target = destination.joinpath(*relative.parts)
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            source = archive.extractfile(member)
            if source is None:
                raise ValueError(f"cannot read npm tarball entry: {member.name}")
            with source, target.open("wb") as output:
                shutil.copyfileobj(source, output)


def package_file_manifest(package_root: Path) -> list[dict]:
    output = []
    files = [path for path in package_root.rglob("*") if path.is_file()]
    files.sort(key=lambda path: path.relative_to(package_root).as_posix())
    for path in files:
        output.append(
            {
                "path": path.relative_to(package_root).as_posix(),
                "size": path.stat().st_size,
                "sha256": digest(path),
            }
        )
    return output


def write_launchers(bin_dir: Path, windows: bool) -> None:
    bin_dir.mkdir(parents=True, exist_ok=True)
    posix = bin_dir / "buckyos"
    posix.write_text(
        "\n".join(
            [
                "#!/bin/sh",
                "set -eu",
                'ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)',
                'TOOL="$ROOT/libexec/buckyos-tool"',
                'DENO="$TOOL/runtime/deno"',
                "export BUCKYOS_ROOT=${BUCKYOS_ROOT:-$ROOT}",
                'exec "$DENO" run --no-prompt --allow-read="$TOOL" '
                f'--allow-env="{ENVIRONMENT_NAMES}" --allow-run="$DENO" '
                '"$TOOL/cli/system_bootstrap.ts" "$@"',
                "",
            ]
        ),
        encoding="utf-8",
        newline="\n",
    )
    posix.chmod(posix.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    if windows:
        (bin_dir / "buckyos.cmd").write_text(
            "\r\n".join(
                [
                    "@echo off",
                    "setlocal",
                    'set "ROOT=%~dp0.."',
                    'set "TOOL=%ROOT%\\libexec\\buckyos-tool"',
                    'set "DENO=%TOOL%\\runtime\\deno.exe"',
                    'if not defined BUCKYOS_ROOT set "BUCKYOS_ROOT=%ROOT%"',
                    '"%DENO%" run --no-prompt --allow-read="%TOOL%" '
                    f'--allow-env="{ENVIRONMENT_NAMES}" --allow-run="%DENO%" '
                    '"%TOOL%\\cli\\system_bootstrap.ts" %*',
                    "",
                ]
            ),
            encoding="utf-8",
            newline="",
        )


def build(args: argparse.Namespace) -> None:
    tarball = args.artifact.resolve(strict=True)
    deno = args.deno.resolve(strict=True)
    sbom = args.sbom.resolve(strict=True)
    release = load_manifest(args.release_manifest.resolve(strict=True))
    verify_artifacts(tarball, deno, sbom, release, args.deno_version)
    rootfs = args.rootfs.resolve()
    rootfs.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="buckyos-tool-distribution-") as temporary:
        staging = Path(temporary) / "buckyos-tool"
        staging.mkdir()
        extract_npm_tarball(tarball, staging)
        package_json = json.loads(
            (staging / "package.json").read_text(encoding="utf-8")
        )
        version = package_json.get("version")
        if version != release["tool_version"] or version != release["sdk_version"]:
            raise ValueError("SDK, Tool, and npm package versions must be identical")
        required = [
            staging / "LICENSE",
            staging / "dist" / "node.mjs",
            staging / "cli" / "main.ts",
            staging / "cli" / "system_bootstrap.ts",
            staging / "cli" / "system_launcher.ts",
        ]
        missing = [str(path.relative_to(staging)) for path in required if not path.is_file()]
        if missing:
            raise ValueError(f"npm artifact is missing system files: {', '.join(missing)}")
        files = package_file_manifest(staging)
        if files != release["npm_files"]:
            raise ValueError("npm file manifest differs from the release manifest")
        files_digest = hashlib.sha256(
            json.dumps(files, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
        ).hexdigest()
        if files_digest != release["npm_files_sha256"]:
            raise ValueError("npm file manifest digest differs from the release manifest")
        runtime_dir = staging / "runtime"
        runtime_dir.mkdir()
        runtime_name = "deno.exe" if args.windows else "deno"
        runtime_target = runtime_dir / runtime_name
        shutil.copy2(deno, runtime_target)
        if not args.windows:
            runtime_target.chmod(runtime_target.stat().st_mode | stat.S_IXUSR)
        distribution = {
            **release,
            "npm_package_version": version,
            "npm_tarball": tarball.name,
            "npm_files": files,
            "runtime_excluded_from_npm_comparison": [f"runtime/{runtime_name}"],
        }
        (staging / "distribution.json").write_text(
            json.dumps(distribution, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        shutil.copy2(sbom, staging / "sbom.cdx.json")
        target = rootfs / "libexec" / "buckyos-tool"
        target.parent.mkdir(parents=True, exist_ok=True)
        backup = target.with_name(target.name + ".previous")
        if backup.exists():
            shutil.rmtree(backup)
        if target.exists():
            target.replace(backup)
        try:
            shutil.copytree(staging, target)
        except Exception:
            if target.exists():
                shutil.rmtree(target)
            if backup.exists():
                backup.replace(target)
            raise
        if backup.exists():
            shutil.rmtree(backup)
    write_launchers(rootfs / "bin", args.windows)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Build the immutable BuckyOS SDK/Tool system distribution"
    )
    parser.add_argument("--artifact", type=Path, required=True)
    parser.add_argument("--release-manifest", type=Path, required=True)
    parser.add_argument("--deno", type=Path, required=True)
    parser.add_argument(
        "--deno-version",
        help="declared target Deno version; skips executing a cross-architecture binary",
    )
    parser.add_argument("--sbom", type=Path, required=True)
    parser.add_argument(
        "--rootfs",
        type=Path,
        default=Path(__file__).resolve().parents[1] / "rootfs",
    )
    parser.add_argument("--windows", action="store_true")
    return parser.parse_args()


if __name__ == "__main__":
    build(parse_args())
