#!/usr/bin/env python3
"""
Unified local desktop package entrypoint.

This wrapper owns the common staging flow:
- prepare BUCKYOS_BUILD_ROOT
- optionally stage the desktop app bundle/exe
- dispatch to the platform-specific packager under src/publish/
"""

from __future__ import annotations

import argparse
import os
import platform
import shutil
import subprocess
import sys
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover
    tomllib = None  # type: ignore[assignment]


REPO_ROOT = Path(__file__).resolve().parent
SRC_DIR = REPO_ROOT / "src"
PUBLISH_DIR = SRC_DIR / "publish"
VERSION_FILE = SRC_DIR / "VERSION"
CYFS_SRC_DIR = REPO_ROOT.parent / "cyfs-gateway" / "src"
DESKTOP_APP_REPO_DIR = REPO_ROOT.parent / "BuckyOSApp"


@dataclass(frozen=True)
class TargetScript:
    platform_key: str
    script_path: Path
    architecture: str
    build_root: Path


def _detect_platform_key() -> str:
    system_name = platform.system().lower()
    if system_name == "darwin":
        return "macos"
    if system_name == "linux":
        return "linux"
    if system_name == "windows":
        return "windows"
    raise RuntimeError(f"unsupported operating system: {platform.system()}")


def _default_build_root(platform_key: str) -> Path:
    if platform_key == "windows":
        raw = os.environ.get("BUCKYOS_BUILD_ROOT", r"C:\opt\buckyosci")
    else:
        raw = os.environ.get("BUCKYOS_BUILD_ROOT", "/opt/buckyosci")
    return Path(raw).expanduser()


def _normalize_arch(raw_arch: str, platform_key: str) -> str:
    arch = (raw_arch or "").strip().lower()
    if arch in ("x86_64", "amd64"):
        return "amd64"
    if arch in ("arm64", "aarch64"):
        return "aarch64" if platform_key == "macos" else "arm64"
    raise RuntimeError(f"unsupported architecture '{raw_arch}' for {platform_key}")


def detect_target(arch_override: str | None = None, build_root_override: str | None = None) -> TargetScript:
    platform_key = _detect_platform_key()
    raw_arch = arch_override or platform.machine()
    architecture = _normalize_arch(raw_arch, platform_key)

    script_name = {
        "macos": "make_local_osx_pkg.py",
        "linux": "make_local_deb.py",
        "windows": "make_local_win_installer.py",
    }[platform_key]
    build_root = Path(build_root_override).expanduser() if build_root_override else _default_build_root(platform_key)
    return TargetScript(
        platform_key=platform_key,
        script_path=PUBLISH_DIR / script_name,
        architecture=architecture,
        build_root=build_root,
    )


def _default_version() -> str:
    base_version = VERSION_FILE.read_text(encoding="utf-8").strip()
    if not base_version:
        raise RuntimeError(f"empty version file: {VERSION_FILE}")
    return f"{base_version}+build{datetime.now().strftime('%y%m%d')}"


def _python_executable() -> str:
    candidates = [
        REPO_ROOT / "venv" / "bin" / "python3",
        REPO_ROOT / "venv" / "bin" / "python",
        REPO_ROOT / "venv" / "Scripts" / "python.exe",
    ]
    for candidate in candidates:
        if candidate.exists():
            return str(candidate)
    return sys.executable or "python3"


def _run(cmd: list[str], *, cwd: Path | None = None, dry_run: bool = False) -> int:
    cwd_path = cwd or REPO_ROOT
    print("[run]", " ".join(cmd), f"(cwd={cwd_path})")
    if dry_run:
        return 0
    completed = subprocess.run(cmd, cwd=cwd_path)
    return int(completed.returncode)


def _run_checked(cmd: list[str], *, cwd: Path | None = None, dry_run: bool = False) -> None:
    rc = _run(cmd, cwd=cwd, dry_run=dry_run)
    if rc != 0:
        raise RuntimeError(f"command failed with exit code {rc}: {' '.join(cmd)}")


def _remove_tree(path: Path, *, dry_run: bool) -> None:
    if not path.exists():
        return
    print("[clean]", path)
    if dry_run:
        return
    shutil.rmtree(path, ignore_errors=True)


def _copy_path(src: Path, dst: Path, *, dry_run: bool) -> None:
    print("[stage]", src, "->", dst)
    if dry_run:
        return
    if dst.exists():
        if dst.is_dir():
            shutil.rmtree(dst, ignore_errors=True)
        else:
            dst.unlink()
    dst.parent.mkdir(parents=True, exist_ok=True)
    if src.is_dir():
        shutil.copytree(src, dst)
    else:
        shutil.copy2(src, dst)


def _expected_desktop_app_path(target: TargetScript) -> Path | None:
    if target.platform_key == "macos":
        return target.build_root / "BuckyOSApp" / "BuckyOS.app"
    if target.platform_key == "windows":
        return target.build_root / "BuckyOSApp" / "buckyosapp.exe"
    return None


def _cargo_target_dir() -> Path | None:
    cargo_config_paths = [
        Path.home() / ".cargo" / "config.toml",
        Path.home() / ".cargo" / "config",
    ]
    if tomllib is None:
        return None
    for config_path in cargo_config_paths:
        if not config_path.exists():
            continue
        try:
            data = tomllib.loads(config_path.read_text(encoding="utf-8"))
        except Exception:
            continue
        build = data.get("build")
        if isinstance(build, dict):
            target_dir = build.get("target-dir")
            if isinstance(target_dir, str) and target_dir.strip():
                return Path(target_dir).expanduser()
    return None


def _rust_build_root() -> Path:
    rust_build = os.environ.get("RUST_BUILD")
    if rust_build:
        return Path(rust_build).expanduser()
    cargo_target_dir = _cargo_target_dir()
    if cargo_target_dir is not None:
        return cargo_target_dir
    return Path("/tmp/rust_build")


def _desktop_build_roots() -> list[Path]:
    return [
        _rust_build_root(),
        DESKTOP_APP_REPO_DIR / "src-tauri" / "target",
    ]


def _built_desktop_app_candidates(target: TargetScript) -> list[Path]:
    if target.platform_key == "macos":
        candidates: list[Path] = []
        for root in _desktop_build_roots():
            candidates.append(root / "release" / "bundle" / "macos" / "BuckyOS.app")
        return candidates
    if target.platform_key == "windows":
        candidates = []
        for root in _desktop_build_roots():
            candidates.extend(
                [
                    root / "release" / "buckyosapp.exe",
                    root / "release" / "BuckyOS.exe",
                ]
            )
        return candidates
    return []


def _build_desktop_app(target: TargetScript, *, dry_run: bool) -> Path | None:
    if target.platform_key not in ("macos", "windows"):
        return None
    if not DESKTOP_APP_REPO_DIR.exists():
        return None

    _run_checked(["pnpm", "run", "tauri", "build"], cwd=DESKTOP_APP_REPO_DIR, dry_run=dry_run)
    candidates = _built_desktop_app_candidates(target)
    if dry_run:
        return candidates[0] if candidates else None
    for candidate in candidates:
        if candidate.exists():
            return candidate
    return None


def _stage_desktop_app(
    target: TargetScript,
    desktop_app: str | None,
    *,
    dry_run: bool,
    skip_desktop_app_build: bool,
) -> None:
    expected = _expected_desktop_app_path(target)
    if expected is None:
        return

    if desktop_app:
        src = Path(desktop_app).expanduser()
        if not src.exists():
            raise RuntimeError(f"desktop app artifact not found: {src}")
        if target.platform_key == "macos" and not src.is_dir():
            raise RuntimeError(f"macOS desktop app must be a .app directory: {src}")
        if target.platform_key == "windows" and src.is_dir():
            raise RuntimeError(f"Windows desktop app must be an .exe file: {src}")
        _copy_path(src, expected, dry_run=dry_run)
        return

    if not skip_desktop_app_build:
        built = _build_desktop_app(target, dry_run=dry_run)
        if built is not None:
            _copy_path(built, expected, dry_run=dry_run)
            return

    if expected.exists():
        print(f"[stage] reuse existing desktop app: {expected}")
        return

    raise RuntimeError(
        "desktop app artifact is required but missing. "
        f"Expected {expected}, or place BuckyOSApp beside this repo, or pass --desktop-app <path>."
    )


def _prepare_common_build_root(
    *,
    target: TargetScript,
    dry_run: bool,
    skip_cargo_update: bool,
    skip_cyfs_gateway: bool,
    desktop_app: str | None,
    skip_desktop_app_build: bool,
) -> None:
    python_exe = _python_executable()
    buckyos_root = target.build_root / "buckyos"
    buckycli_root = target.build_root / "buckycli"

    print(f"[prepare] platform={target.platform_key} arch={target.architecture} build_root={target.build_root}")
    if target.platform_key in ("macos", "windows"):
        print(f"[prepare] RUST_BUILD={_rust_build_root()}")

    _remove_tree(buckyos_root, dry_run=dry_run)
    _remove_tree(buckycli_root, dry_run=dry_run)

    if (not skip_cyfs_gateway) and CYFS_SRC_DIR.exists():
        if not dry_run:
            buckyos_root.mkdir(parents=True, exist_ok=True)
        if not skip_cargo_update:
            _run_checked(["cargo", "update"], cwd=CYFS_SRC_DIR, dry_run=dry_run)
        _run_checked(["buckyos-build"], cwd=CYFS_SRC_DIR, dry_run=dry_run)
        _run_checked(
            ["buckyos-install", "--all", f"--target-rootfs={buckyos_root}", "--app=cyfs-gateway"],
            cwd=CYFS_SRC_DIR,
            dry_run=dry_run,
        )
    elif skip_cyfs_gateway:
        print("[prepare] skip cyfs-gateway by request")
    else:
        print(f"[prepare] skip cyfs-gateway, repo not found: {CYFS_SRC_DIR}")

    if not skip_cargo_update:
        _run_checked(["cargo", "update"], cwd=SRC_DIR, dry_run=dry_run)
    _run_checked(["buckyos-build"], cwd=SRC_DIR, dry_run=dry_run)
    _run_checked(
        ["buckyos-install", "--all", f"--target-rootfs={buckycli_root}", "--app=buckycli"],
        cwd=SRC_DIR,
        dry_run=dry_run,
    )
    _run_checked(
        ["buckyos-install", "--all", f"--target-rootfs={buckyos_root}", "--app=buckyos"],
        cwd=SRC_DIR,
        dry_run=dry_run,
    )
    _run_checked([python_exe, "make_config.py", "release", f"--rootfs={buckyos_root}"], cwd=SRC_DIR, dry_run=dry_run)

    _stage_desktop_app(
        target,
        desktop_app,
        dry_run=dry_run,
        skip_desktop_app_build=skip_desktop_app_build,
    )


def prepare_root(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(prog="make_local_pkg.py prepare-root")
    parser.add_argument("--arch", default=None, help="Override detected architecture")
    parser.add_argument("--build-root", default=None, help="Override BUCKYOS_BUILD_ROOT")
    parser.add_argument("--app-publish-dir", default=None, help="Alias of --build-root for compatibility")
    parser.add_argument("--desktop-app", default=None, help="Path to BuckyOS desktop app bundle/exe to stage")
    parser.add_argument(
        "--skip-desktop-app-build",
        action="store_true",
        help="Do not auto-build desktop app from ../BuckyOSApp",
    )
    parser.add_argument("--skip-cargo-update", action="store_true", help="Skip cargo update in shared build steps")
    parser.add_argument("--skip-cyfs-gateway", action="store_true", help="Skip cyfs-gateway staging")
    parser.add_argument("--dry-run", action="store_true", help="Print commands without executing them")
    args = parser.parse_args(argv)

    build_root_override = args.build_root or args.app_publish_dir
    target = detect_target(args.arch, build_root_override)
    _prepare_common_build_root(
        target=target,
        dry_run=bool(args.dry_run),
        skip_cargo_update=bool(args.skip_cargo_update),
        skip_cyfs_gateway=bool(args.skip_cyfs_gateway),
        desktop_app=args.desktop_app,
        skip_desktop_app_build=bool(args.skip_desktop_app_build),
    )
    return 0


def build_pkg(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(prog="make_local_pkg.py build-pkg")
    parser.add_argument("version", nargs="?", help="Package version, defaults to src/VERSION + buildYYMMDD")
    parser.add_argument("--arch", default=None, help="Override detected architecture")
    parser.add_argument("--project", default=str(SRC_DIR / "bucky_project.yaml"))
    parser.add_argument("--build-root", default=None, help="Override BUCKYOS_BUILD_ROOT")
    parser.add_argument("--app-publish-dir", default=None, help="Alias of --build-root for compatibility")
    parser.add_argument("--out-dir", default=str(REPO_ROOT / "publish"))
    parser.add_argument("--desktop-app", default=None, help="Path to BuckyOS desktop app bundle/exe to stage")
    parser.add_argument(
        "--skip-desktop-app-build",
        action="store_true",
        help="Do not auto-build desktop app from ../BuckyOSApp",
    )
    parser.add_argument("--skip-prepare", action="store_true", help="Assume BUCKYOS_BUILD_ROOT is already staged")
    parser.add_argument("--skip-cargo-update", action="store_true", help="Skip cargo update in shared build steps")
    parser.add_argument("--skip-cyfs-gateway", action="store_true", help="Skip cyfs-gateway staging")
    parser.add_argument("--no-sync-scripts", action="store_true")
    parser.add_argument("--dry-run", action="store_true")
    args, forwarded = parser.parse_known_args(argv)

    build_root_override = args.build_root or args.app_publish_dir
    target = detect_target(args.arch, build_root_override)
    version = args.version or _default_version()

    if not args.skip_prepare:
        _prepare_common_build_root(
            target=target,
            dry_run=bool(args.dry_run),
            skip_cargo_update=bool(args.skip_cargo_update),
            skip_cyfs_gateway=bool(args.skip_cyfs_gateway),
            desktop_app=args.desktop_app,
            skip_desktop_app_build=bool(args.skip_desktop_app_build),
        )
    else:
        print("[prepare] skipped by --skip-prepare")

    cmd = [
        _python_executable(),
        str(target.script_path),
        "build-pkg",
        target.architecture,
        version,
        "--project",
        args.project,
        "--app-publish-dir",
        str(target.build_root),
        "--out-dir",
        args.out_dir,
    ]
    if args.no_sync_scripts:
        cmd.append("--no-sync-scripts")
    if args.dry_run:
        cmd.append("--dry-run")
    cmd += forwarded
    _run_checked(cmd, dry_run=False)
    return 0


def verify_pkg(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(prog="make_local_pkg.py verify-pkg")
    parser.add_argument("pkg", help="Path to package file")
    parser.add_argument("--arch", default=None, help="Override detected architecture")
    parser.add_argument("--build-root", default=None, help="Override BUCKYOS_BUILD_ROOT")
    parser.add_argument("--project", default=str(SRC_DIR / "bucky_project.yaml"))
    args, forwarded = parser.parse_known_args(argv)

    target = detect_target(args.arch, args.build_root)
    base_cmd = [_python_executable(), str(target.script_path)]
    if target.platform_key == "windows":
        cmd = base_cmd + ["verify-pkg", "--pkg", args.pkg, "--project", args.project]
    else:
        cmd = base_cmd + ["verify-pkg", args.pkg, "--project", args.project]
    cmd += forwarded
    return _run(cmd)


def sync_scripts(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(prog="make_local_pkg.py sync-scripts")
    parser.add_argument("--arch", default=None, help="Override detected architecture")
    parser.add_argument("--build-root", default=None, help="Override BUCKYOS_BUILD_ROOT")
    parser.add_argument("--project", default=str(SRC_DIR / "bucky_project.yaml"))
    args, forwarded = parser.parse_known_args(argv)

    target = detect_target(args.arch, args.build_root)
    if target.platform_key == "macos":
        cmd = [_python_executable(), str(target.script_path), "sync-macos-scripts", "--project", args.project]
    elif target.platform_key == "windows":
        cmd = [_python_executable(), str(target.script_path), "sync", "--project", args.project]
    else:
        raise RuntimeError("sync-scripts is not supported for Linux packages")
    cmd += forwarded
    return _run(cmd)


def local_action(action: str, argv: list[str]) -> int:
    parser = argparse.ArgumentParser(prog=f"make_local_pkg.py {action}")
    parser.add_argument("--arch", default=None, help="Override detected architecture")
    parser.add_argument("--build-root", default=None, help="Override BUCKYOS_BUILD_ROOT")
    parser.add_argument("--project", default=str(SRC_DIR / "bucky_project.yaml"))
    parser.add_argument("--target", default=None)
    parser.add_argument("--source", default=None)
    parser.add_argument("--dry-run", action="store_true")
    args, forwarded = parser.parse_known_args(argv)

    target = detect_target(args.arch, args.build_root)
    if target.platform_key == "windows":
        raise RuntimeError(f"{action} is not supported by the Windows installer script")

    cmd = [_python_executable(), str(target.script_path), action, "--project", args.project]
    if args.target:
        cmd += ["--target", args.target]
    if args.source:
        cmd += ["--source", args.source]
    if args.dry_run:
        cmd.append("--dry-run")
    cmd += forwarded
    return _run(cmd)


def show_target(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(prog="make_local_pkg.py show-target")
    parser.add_argument("--arch", default=None, help="Override detected architecture")
    parser.add_argument("--build-root", default=None, help="Override BUCKYOS_BUILD_ROOT")
    args = parser.parse_args(argv)

    target = detect_target(args.arch, args.build_root)
    print(f"platform={target.platform_key}")
    print(f"arch={target.architecture}")
    print(f"build_root={target.build_root}")
    print(f"script={target.script_path}")
    return 0


def main(argv: list[str]) -> int:
    if len(argv) < 2 or argv[1] in ("-h", "--help"):
        print(
            "Usage: make_local_pkg.py <command> [args...]\n\n"
            "Commands:\n"
            "  prepare-root   Prepare BUCKYOS_BUILD_ROOT for the current OS/arch\n"
            "  build-pkg      Prepare BUCKYOS_BUILD_ROOT and build a package\n"
            "  verify-pkg     Verify a built package for the current OS/arch\n"
            "  sync-scripts   Sync generated installer scripts where supported\n"
            "  install        Local install helper (macOS/Linux only)\n"
            "  update         Local update helper (macOS/Linux only)\n"
            "  uninstall      Local uninstall helper (macOS/Linux only)\n"
            "  show-target    Print the detected platform/script mapping\n"
        )
        return 0

    command = argv[1]
    try:
        if command == "prepare-root":
            return prepare_root(argv[2:])
        if command == "build-pkg":
            return build_pkg(argv[2:])
        if command == "verify-pkg":
            return verify_pkg(argv[2:])
        if command == "sync-scripts":
            return sync_scripts(argv[2:])
        if command in ("install", "update", "uninstall"):
            return local_action(command, argv[2:])
        if command == "show-target":
            return show_target(argv[2:])
        raise RuntimeError(f"unknown command: {command}")
    except RuntimeError as err:
        print(f"error: {err}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
