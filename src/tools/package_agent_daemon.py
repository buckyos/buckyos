#!/usr/bin/env python3

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import zipfile


SOURCE = Path(__file__).resolve().parents[1]
PUBLISH = SOURCE / "rootfs" / "bin" / "buckyos-agent-daemon"
TARGETS = {
    "x86_64-unknown-linux-musl": ("linux", "x86_64"),
    "aarch64-unknown-linux-gnu": ("linux", "aarch64"),
    "x86_64-apple-darwin": ("macos", "x86_64"),
    "aarch64-apple-darwin": ("macos", "aarch64"),
    "x86_64-pc-windows-msvc": ("windows", "x86_64"),
    "aarch64-pc-windows-msvc": ("windows", "aarch64"),
}
ELF_MACHINES = {"x86_64": "Advanced Micro Devices X86-64", "aarch64": "AArch64"}


def version() -> str:
    value = (SOURCE / "VERSION").read_text(encoding="utf-8").strip()
    if not re.fullmatch(r"[A-Za-z0-9._+-]+", value):
        raise ValueError("invalid src/VERSION")
    return value


def archive_name(target: str) -> str:
    """File name of the release archive for a target, without directory."""
    platform, _ = TARGETS[target]
    return f"buckyos-agent-daemon-{version()}-{target}{'.zip' if platform == 'windows' else '.tar.gz'}"


def inspect_elf(binary: Path, target: str) -> dict:
    """Check a Linux binary with readelf: architecture, shared libraries and glibc floor."""
    _, arch = TARGETS[target]
    if shutil.which("readelf") is None:
        raise ValueError("readelf (binutils) is required to package a Linux target")
    header = subprocess.check_output(["readelf", "-h", str(binary)], text=True)
    if ELF_MACHINES[arch] not in header:
        raise ValueError(f"binary architecture does not match {target}")
    dynamic = subprocess.check_output(["readelf", "-d", str(binary)], text=True)
    libraries = sorted(set(re.findall(r"Shared library: \[(.*?)\]", dynamic)))
    if target.endswith("-musl") and libraries:
        raise ValueError(f"musl release must be statically linked; found {libraries}")
    symbols = subprocess.check_output(["readelf", "--version-info", str(binary)], text=True)
    versions = re.findall(r"GLIBC_(\d+\.\d+(?:\.\d+)?)", symbols)
    glibc = max(versions, key=lambda v: tuple(map(int, v.split(".")))) if versions else None
    return {"dependencies": libraries, "minimum_glibc": glibc}


def package(binary: Path, target: str, output: Path, *, static_crt: bool = False) -> Path:
    platform, arch = TARGETS[target]
    if not binary.is_file():
        raise ValueError(f"daemon binary not found: {binary}")
    release = version()
    name = f"buckyos-agent-daemon-{release}-{target}"
    linux = inspect_elf(binary, target) if platform == "linux" else {}
    output.mkdir(parents=True, exist_ok=True)
    archive = output / archive_name(target)
    with tempfile.TemporaryDirectory(prefix="buckyos-agent-daemon-") as temporary:
        payload = Path(temporary) / name
        payload.mkdir()
        executable = payload / ("buckyos_agent_daemon.exe" if platform == "windows" else "buckyos_agent_daemon")
        shutil.copyfile(binary, executable)
        shutil.copyfile(SOURCE / "rootfs/bin/buckyos-agent-daemon/README.md", payload / "README.md")
        executables = [executable]
        if platform == "linux":
            service = payload / "service.sh"
            shutil.copyfile(PUBLISH / "service.sh", service)
            executables.append(service)
        manifest = {
            "schema_version": 1,
            "version": release,
            "target": target,
            "platform": platform,
            "arch": arch,
            "binary": executable.name,
            "sha256": hashlib.sha256(executable.read_bytes()).hexdigest(),
            "identity_mode": "current_user",
            "foreground": True,
            "files": sorted(entry.name for entry in payload.iterdir()) + ["manifest.json"],
            "requirements": {
                "shell": "powershell.exe" if platform == "windows" else "/bin/bash",
                "interactive_user_session": platform in ("macos", "windows"),
                "macos_minimum": "14.0" if platform == "macos" else None,
                "windows_minimum": "10.0.15063" if platform == "windows" else None,
                "windows_crt": ("static" if static_crt else "prebuilt_unverified") if platform == "windows" else None,
                "visual_cpp_runtime": f"Microsoft Visual C++ v14 Redistributable ({arch})" if platform == "windows" and not static_crt else None,
                "desktop_permissions": ["screen_recording", "accessibility"] if platform == "macos" else [],
                "linux_ui": "unsupported" if platform == "linux" else None,
                **linux,
            },
            "signature": "HTTPS release with SHA-256 checksum; no detached signing configured",
        }
        (payload / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
        if platform == "windows":
            with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as zipped:
                for entry in sorted(payload.iterdir()):
                    zipped.write(entry, arcname=f"{name}/{entry.name}")
        else:
            with tarfile.open(archive, "w:gz") as tar:
                for entry in (payload, *sorted(payload.iterdir())):
                    info = tar.gettarinfo(entry, arcname=f"{name}/{entry.name}" if entry != payload else name)
                    info.mode = 0o755 if entry == payload or entry in executables else 0o644
                    info.uid = info.gid = 0
                    info.uname = info.gname = ""
                    if info.isfile():
                        with entry.open("rb") as stream:
                            tar.addfile(info, stream)
                    else:
                        tar.addfile(info)
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    archive.with_name(archive.name + ".sha256").write_text(f"{digest}  {archive.name}\n", encoding="ascii")
    if platform == "linux":
        launcher = output / "start-buckyos-agent-daemon.sh"
        shutil.copyfile(PUBLISH / "start-buckyos-agent-daemon.sh", launcher)
        launcher.chmod(0o755)
    return archive


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Build and package a foreground BuckyOS Agent Daemon")
    parser.add_argument("--target", required=True, choices=TARGETS)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--binary", type=Path, help="Package an already built binary without running Cargo; Windows CRT dependencies remain unverified")
    args = parser.parse_args(argv)
    binary = args.binary
    static_crt = binary is None and TARGETS[args.target][0] == "windows"
    if binary is None:
        build_options = {"cwd": SOURCE}
        if static_crt:
            environment = os.environ.copy()
            if "CARGO_ENCODED_RUSTFLAGS" in environment:
                encoded = environment["CARGO_ENCODED_RUSTFLAGS"]
            else:
                encoded = "\x1f".join(environment.get("RUSTFLAGS", "").split())
            environment["CARGO_ENCODED_RUSTFLAGS"] = (
                (encoded + "\x1f" if encoded else "") + "-C\x1ftarget-feature=+crt-static"
            )
            build_options["env"] = environment
        result = subprocess.run([
            "cargo", "build", "--release", "--package", "buckyos_agent_daemon", "--target", args.target,
        ], **build_options)
        if result.returncode:
            return result.returncode
        metadata = subprocess.run([
            "cargo", "metadata", "--format-version", "1", "--no-deps",
        ], cwd=SOURCE, capture_output=True, text=True)
        if metadata.returncode:
            print(metadata.stderr, file=sys.stderr, end="")
            return metadata.returncode
        target_dir = Path(json.loads(metadata.stdout)["target_directory"])
        filename = "buckyos_agent_daemon.exe" if TARGETS[args.target][0] == "windows" else "buckyos_agent_daemon"
        binary = target_dir / args.target / "release" / filename
    try:
        print(package(binary, args.target, args.output, static_crt=static_crt))
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Cannot package agent daemon: {error}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
