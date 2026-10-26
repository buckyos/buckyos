#!/usr/bin/env python3
"""Assemble and verify the macOS BuckyOS Runtime bundle.

On macOS the installed node_daemon is the main executable of
`bin/node-daemon/BuckyOS Runtime.app`. The Rust build drops the raw Cargo
output into the node_daemon module directory; `assemble` moves it into the
bundle built from `kernel/node_daemon/macos/`, signs the bundle and verifies
the signature, so every later copy (rootfs, dev install, staged pkg root)
carries one signed Runtime and no loose binary.
"""

from __future__ import annotations

import argparse
import os
import plistlib
import shutil
import subprocess
import sys
from pathlib import Path


BUNDLE_NAME = "BuckyOS Runtime.app"
BUNDLE_ID = "com.buckyos.runtime"
EXECUTABLE = "node_daemon"
ICON_FILE = "BuckyOSRuntime.icns"
SIGN_IDENTITY_ENV = "BUCKYOS_MACOS_CODESIGN_IDENTITY"

SRC_DIR = Path(__file__).resolve().parents[1]
TEMPLATE_DIR = SRC_DIR / "kernel" / "node_daemon" / "macos"
VERSION_FILE = SRC_DIR / "VERSION"
DEFAULT_MODULE_DIR = SRC_DIR / "rootfs" / "bin" / "node-daemon"

FIXED_INFO_KEYS = (
    "CFBundleExecutable",
    "CFBundleIdentifier",
    "CFBundlePackageType",
    "LSMinimumSystemVersion",
    "LSUIElement",
)
MACHO_MAGICS = {
    bytes.fromhex(value)
    for value in ("cffaedfe", "feedfacf", "cefaedfe", "feedface", "cafebabe", "bebafeca")
}


def bundle_path(module_dir: Path) -> Path:
    return module_dir / BUNDLE_NAME


def bundle_executable(bundle: Path) -> Path:
    return bundle / "Contents" / "MacOS" / EXECUTABLE


def is_macho(path: Path) -> bool:
    try:
        with path.open("rb") as handle:
            return handle.read(4) in MACHO_MAGICS
    except OSError:
        return False


def _template_info() -> dict:
    with (TEMPLATE_DIR / "Info.plist").open("rb") as handle:
        return plistlib.load(handle)


def _write_info_plist(contents_dir: Path, version: str) -> None:
    info = _template_info()
    info["CFBundleShortVersionString"] = version
    info["CFBundleVersion"] = version
    with (contents_dir / "Info.plist").open("wb") as handle:
        plistlib.dump(info, handle)


def _sign(bundle: Path, identity: str) -> None:
    if shutil.which("codesign") is None:
        raise RuntimeError("codesign not found; the Runtime bundle can only be assembled on macOS")
    command = ["codesign", "--force", "--sign", identity]
    if identity != "-":
        command += ["--options", "runtime", "--timestamp"]
    subprocess.run([*command, str(bundle)], check=True)


def _signature_failures(bundle: Path) -> list[str]:
    if shutil.which("codesign") is None:
        return ["codesign not found; cannot verify the Runtime signature"]
    checked = subprocess.run(
        ["codesign", "--verify", "--strict", "--verbose=2", str(bundle)],
        capture_output=True,
        text=True,
    )
    if checked.returncode != 0:
        return [f"codesign --verify failed: {(checked.stderr or checked.stdout).strip()}"]
    described = subprocess.run(["codesign", "-dv", str(bundle)], capture_output=True, text=True)
    if f"Identifier={BUNDLE_ID}" not in described.stderr:
        return [f"signature identifier is not {BUNDLE_ID}: {described.stderr.strip()}"]
    return []


def verify(bundle: Path, *, check_signature: bool = True, check_siblings: bool = True) -> list[str]:
    failures: list[str] = []
    info_path = bundle / "Contents" / "Info.plist"
    try:
        with info_path.open("rb") as handle:
            info = plistlib.load(handle)
    except (OSError, plistlib.InvalidFileException) as error:
        return [f"cannot read {info_path}: {error}"]
    template = _template_info()
    for key in FIXED_INFO_KEYS:
        if info.get(key) != template[key]:
            failures.append(f"Info.plist {key}={info.get(key)!r}, expected {template[key]!r}")
    if not info.get("CFBundleShortVersionString") or info.get("CFBundleShortVersionString") == "0.0.0":
        failures.append("Info.plist has no version")
    executable = bundle_executable(bundle)
    if not executable.is_file() or not is_macho(executable):
        failures.append(f"missing Mach-O main executable: {executable}")
    elif not os.access(executable, os.X_OK):
        failures.append(f"main executable is not executable: {executable}")
    macos_dir = executable.parent
    if macos_dir.is_dir():
        extra = sorted(p.name for p in macos_dir.iterdir() if p.name != EXECUTABLE)
        if extra:
            failures.append(f"unexpected files in Contents/MacOS: {', '.join(extra)}")
    if not (bundle / "Contents" / "Resources" / ICON_FILE).is_file():
        failures.append(f"missing Contents/Resources/{ICON_FILE}")
    if check_siblings and (bundle.parent / EXECUTABLE).exists():
        failures.append(f"loose {EXECUTABLE} beside the bundle: {bundle.parent / EXECUTABLE}")
    if check_signature and not failures:
        failures.extend(_signature_failures(bundle))
    return failures


def assemble(module_dir: Path, *, version: str | None = None, identity: str | None = None) -> str:
    """Bundle a freshly built raw node_daemon; returns what happened."""
    raw = module_dir / EXECUTABLE
    bundle = bundle_path(module_dir)
    if not raw.is_file():
        return "unchanged" if bundle.is_dir() else "absent"
    if not is_macho(raw):
        # A cross build for another OS owns this staging directory now.
        if bundle.exists():
            shutil.rmtree(bundle)
        return "not-macos"
    version = version or VERSION_FILE.read_text(encoding="utf-8").strip()
    identity = identity or os.environ.get(SIGN_IDENTITY_ENV) or "-"
    staging_root = module_dir / ".runtime-bundle-staging"
    if staging_root.exists():
        shutil.rmtree(staging_root)
    staged = bundle_path(staging_root)
    try:
        (staged / "Contents" / "MacOS").mkdir(parents=True)
        (staged / "Contents" / "Resources").mkdir()
        _write_info_plist(staged / "Contents", version)
        shutil.copy2(TEMPLATE_DIR / ICON_FILE, staged / "Contents" / "Resources" / ICON_FILE)
        shutil.copy2(raw, bundle_executable(staged))
        bundle_executable(staged).chmod(0o755)
        _sign(staged, identity)
        failures = verify(staged, check_siblings=False)
        if failures:
            raise RuntimeError("; ".join(failures))
        if bundle.exists():
            shutil.rmtree(bundle)
        os.replace(staged, bundle)
    finally:
        shutil.rmtree(staging_root, ignore_errors=True)
    raw.unlink()
    failures = verify(bundle)
    if failures:
        raise RuntimeError("; ".join(failures))
    return "assembled"


def _resolve_bundle(path: Path) -> Path:
    return path if path.name == BUNDLE_NAME else bundle_path(path)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="macos_runtime_bundle.py", description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)
    assemble_parser = commands.add_parser("assemble", help="bundle the raw node_daemon of a module directory")
    assemble_parser.add_argument("module_dir", nargs="?", type=Path, default=DEFAULT_MODULE_DIR)
    assemble_parser.add_argument("--version", default=None)
    assemble_parser.add_argument("--sign-identity", default=None, help=f"codesign identity (default ${SIGN_IDENTITY_ENV} or ad-hoc)")
    verify_parser = commands.add_parser("verify", help="verify a Runtime bundle or the module directory holding it")
    verify_parser.add_argument("path", nargs="?", type=Path, default=DEFAULT_MODULE_DIR)
    args = parser.parse_args(argv)

    if args.command == "assemble":
        try:
            outcome = assemble(args.module_dir, version=args.version, identity=args.sign_identity)
        except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
            print(f"Runtime bundle assembly failed: {error}", file=sys.stderr)
            return 1
        print(f"Runtime bundle {outcome}: {bundle_path(args.module_dir)}")
        return 0

    bundle = _resolve_bundle(args.path)
    failures = verify(bundle)
    if failures:
        print(f"Runtime bundle verification failed: {bundle}", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print(f"Runtime bundle verified: {bundle}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
