#!/usr/bin/env python3

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile


SOURCE = Path(__file__).resolve().parents[1]
TARGETS = {
    "x86_64-unknown-linux-musl": ("linux", "x86_64"),
    "aarch64-unknown-linux-gnu": ("linux", "aarch64"),
    "x86_64-apple-darwin": ("macos", "x86_64"),
    "aarch64-apple-darwin": ("macos", "aarch64"),
}


def package(binary: Path, target: str, output: Path) -> Path:
    platform, arch = TARGETS[target]
    if not binary.is_file():
        raise ValueError(f"daemon binary not found: {binary}")
    version = (SOURCE / "VERSION").read_text(encoding="utf-8").strip()
    name = f"buckyos-agent-daemon-{version}-{target}"
    output.mkdir(parents=True, exist_ok=True)
    archive = output / f"{name}.tar.gz"
    with tempfile.TemporaryDirectory(prefix="buckyos-agent-daemon-") as temporary:
        payload = Path(temporary) / name
        payload.mkdir()
        executable = payload / "buckyos_agent_daemon"
        shutil.copyfile(binary, executable)
        executable.chmod(0o755)
        shutil.copyfile(SOURCE / "rootfs/bin/buckyos-agent-daemon/README.md", payload / "README.md")
        manifest = {
            "schema_version": 1,
            "version": version,
            "target": target,
            "platform": platform,
            "arch": arch,
            "binary": executable.name,
            "sha256": hashlib.sha256(executable.read_bytes()).hexdigest(),
            "identity_mode": "current_user",
            "foreground": True,
            "requirements": {
                "shell": "/bin/bash",
                "macos_minimum": "14.0" if platform == "macos" else None,
                "desktop_permissions": ["screen_recording", "accessibility"] if platform == "macos" else [],
            },
        }
        (payload / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
        with tarfile.open(archive, "w:gz") as tar:
            tar.add(payload, arcname=name)
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    archive.with_name(archive.name + ".sha256").write_text(f"{digest}  {archive.name}\n", encoding="ascii")
    return archive


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Build and package a foreground BuckyOS Agent Daemon")
    parser.add_argument("--target", required=True, choices=TARGETS)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--binary", type=Path, help="Package an already built binary without running Cargo")
    args = parser.parse_args(argv)
    binary = args.binary
    if binary is None:
        result = subprocess.run([
            "cargo", "build", "--release", "--package", "buckyos_agent_daemon", "--target", args.target,
        ], cwd=SOURCE)
        if result.returncode:
            return result.returncode
        metadata = subprocess.run([
            "cargo", "metadata", "--format-version", "1", "--no-deps",
        ], cwd=SOURCE, capture_output=True, text=True)
        if metadata.returncode:
            print(metadata.stderr, file=sys.stderr, end="")
            return metadata.returncode
        target_dir = Path(json.loads(metadata.stdout)["target_directory"])
        binary = target_dir / args.target / "release/buckyos_agent_daemon"
    try:
        print(package(binary, args.target, args.output))
    except (OSError, ValueError) as error:
        parser.exit(1, f"Cannot package agent daemon: {error}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
