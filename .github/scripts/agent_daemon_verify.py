#!/usr/bin/env python3
"""Verify a packaged BuckyOS Agent Daemon archive on the platform that built it.

Checks the `.sha256` file, the archive layout (one top-level directory), the
manifest, that the binary digest in the manifest matches, that `doctor --json`
runs and reports the source version, and the platform-specific dynamic
dependencies: no MSVC CRT or OpenSSL DLL imports on Windows (static CRT,
vendored OpenSSL) and only system libraries on macOS. Linux static/glibc facts
are checked by the packager itself.
"""

import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tarfile
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[2]
FORBIDDEN_WINDOWS_IMPORTS = ("vcruntime", "msvcp", "libssl", "libcrypto", "ucrtbased")


def load_packager():
    script = ROOT / "src/tools/package_agent_daemon.py"
    spec = importlib.util.spec_from_file_location("package_agent_daemon", script)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def extract(archive: Path, destination: Path) -> Path:
    if archive.suffix == ".zip":
        with zipfile.ZipFile(archive) as zipped:
            names = zipped.namelist()
            zipped.extractall(destination)
    else:
        with tarfile.open(archive) as tar:
            names = tar.getnames()
            tar.extractall(destination, filter="data")
    roots = {name.split("/", 1)[0] for name in names}
    if len(roots) != 1 or roots == {""}:
        raise ValueError(f"archive must hold exactly one top-level directory, found {sorted(roots)}")
    directory = destination / roots.pop()
    if directory.name != archive.name.removesuffix(".zip").removesuffix(".tar.gz"):
        raise ValueError(f"top-level directory {directory.name} does not match archive name")
    return directory


def check_windows_imports(binary: Path) -> list[str]:
    dumpbin = shutil.which("dumpbin")
    if dumpbin is None:
        raise ValueError("dumpbin is not on PATH; run inside an MSVC developer environment")
    listing = subprocess.check_output([dumpbin, "/dependents", str(binary)], text=True)
    imports = [line.strip() for line in listing.splitlines() if line.strip().lower().endswith(".dll")]
    forbidden = [name for name in imports if name.lower().startswith(FORBIDDEN_WINDOWS_IMPORTS)]
    if forbidden:
        raise ValueError(f"release binary imports non-system DLLs: {forbidden}")
    return imports


def check_macos_imports(binary: Path) -> list[str]:
    listing = subprocess.check_output(["otool", "-L", str(binary)], text=True)
    libraries = [line.split()[0] for line in listing.splitlines()[1:] if line.strip()]
    foreign = [lib for lib in libraries if not lib.startswith(("/usr/lib/", "/System/Library/"))]
    if foreign:
        raise ValueError(f"release binary links non-system libraries: {foreign}")
    return libraries


def verify(dist: Path, target: str) -> dict:
    packager = load_packager()
    version = packager.version()
    expected_platform, _ = packager.TARGETS[target]
    archive = dist / packager.archive_name(target)
    if not archive.is_file():
        raise ValueError(f"missing {archive}")
    recorded = archive.with_name(archive.name + ".sha256").read_text(encoding="ascii").split()
    if recorded != [hashlib.sha256(archive.read_bytes()).hexdigest(), archive.name]:
        raise ValueError(f"{archive.name}.sha256 does not match the archive")
    if expected_platform == "linux" and not (dist / "start-buckyos-agent-daemon.sh").is_file():
        raise ValueError("Linux dist lacks start-buckyos-agent-daemon.sh")
    with tempfile.TemporaryDirectory(prefix="agent-daemon-verify-") as temporary:
        directory = extract(archive, Path(temporary) / "unpacked")
        manifest = json.loads((directory / "manifest.json").read_text(encoding="utf-8"))
        if manifest["version"] != version or manifest["target"] != target or manifest["platform"] != expected_platform:
            raise ValueError(f"manifest does not describe {target} {version}: {manifest}")
        binary = directory / manifest["binary"]
        if hashlib.sha256(binary.read_bytes()).hexdigest() != manifest["sha256"]:
            raise ValueError("manifest sha256 does not match the packaged binary")
        present = sorted(entry.name for entry in directory.iterdir())
        if present != sorted(manifest["files"]):
            raise ValueError(f"manifest files {sorted(manifest['files'])} differ from archive {present}")
        if expected_platform == "windows" and manifest["requirements"]["windows_crt"] != "static":
            raise ValueError("Windows release must be built with a static CRT")
        if expected_platform != "windows":
            binary.chmod(0o755)
        doctor = subprocess.run(
            [str(binary), "--data-dir", str(Path(temporary) / "data"), "doctor", "--json"],
            capture_output=True, text=True, timeout=120,
        )
        if doctor.returncode != 0:
            raise ValueError(f"doctor --json failed ({doctor.returncode}): {doctor.stderr.strip()}")
        report = json.loads(doctor.stdout)
        if report.get("version") != version:
            raise ValueError(f"doctor reports version {report.get('version')!r}, expected {version!r}")
        imports = []
        if expected_platform == "windows":
            imports = check_windows_imports(binary)
        elif expected_platform == "macos":
            imports = check_macos_imports(binary)
    return {"archive": archive.name, "sha256": recorded[0], "host": platform.platform(), "imports": imports,
            "doctor": {"identity": report.get("identity"), "ui": report.get("capabilities", {}).get("ui")}}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--dist", type=Path, required=True)
    parser.add_argument("--target", required=True)
    args = parser.parse_args(argv)
    try:
        print(json.dumps(verify(args.dist, args.target), indent=2))
    except (OSError, ValueError, KeyError, json.JSONDecodeError, subprocess.SubprocessError) as error:
        print(f"::error::{error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
