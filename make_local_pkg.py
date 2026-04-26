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
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from typing import Any

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover
    tomllib = None  # type: ignore[assignment]

try:
    import yaml  # type: ignore
except ModuleNotFoundError:  # pragma: no cover
    yaml = None  # type: ignore[assignment]


REPO_ROOT = Path(__file__).resolve().parent
SRC_DIR = REPO_ROOT / "src"
PUBLISH_DIR = SRC_DIR / "publish"
VERSION_FILE = SRC_DIR / "VERSION"
CYFS_SRC_DIR = REPO_ROOT.parent / "cyfs-gateway" / "src"
DESKTOP_APP_REPO_DIR = REPO_ROOT.parent / "BuckyOSApp"
IGNORED_STAGE_NAMES = {".DS_Store", "__pycache__"}


@dataclass(frozen=True)
class TargetScript:
    platform_key: str
    script_path: Path
    architecture: str
    build_root: Path


def _require_yaml() -> Any:
    if yaml is None:
        raise RuntimeError(
            "PyYAML is required to analyze bucky_project.yaml. "
            "Use the repo venv or install buckyos-devkit / `pip install pyyaml`."
        )
    return yaml


def _yaml_load_file(path: Path) -> dict[str, Any]:
    yaml_mod = _require_yaml()
    data = yaml_mod.safe_load(path.read_text(encoding="utf-8"))
    if data is None:
        return {}
    if not isinstance(data, dict):
        raise RuntimeError(f"YAML root must be a map: {path}")
    return data


def _json_load_file(path: Path) -> dict[str, Any]:
    data = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(data, dict):
        raise RuntimeError(f"JSON root must be a map: {path}")
    return data


def _load_project_config(path: Path) -> dict[str, Any]:
    suffix = path.suffix.lower()
    if suffix in (".yaml", ".yml"):
        return _yaml_load_file(path)
    if suffix == ".json":
        return _json_load_file(path)
    raise RuntimeError(f"unsupported project config format: {path}")


def _expand_project_vars(raw: str, *, build_root: Path) -> str:
    out = os.path.expanduser(raw)
    replacements = {
        "BUCKYOS_ROOT": os.environ.get("BUCKYOS_ROOT", "/opt/buckyos"),
        "BUCKYOS_BUILD_ROOT": str(build_root),
        "APPDATA": os.environ.get("APPDATA") or "%APPDATA%",
        "LOCALAPPDATA": os.environ.get("LOCALAPPDATA") or "%LOCALAPPDATA%",
        "USERPROFILE": os.environ.get("USERPROFILE") or "%USERPROFILE%",
    }
    for name, value in replacements.items():
        out = out.replace(f"${{{name}}}", value)
        out = out.replace(f"%{name}%", value)
    return out


def _normalize_target_dir_name(raw_path: str) -> str:
    return raw_path.strip().lstrip("/\\").rstrip("/\\")


def _detect_item_kind(raw_path: str, source_path: Path) -> str:
    if raw_path.rstrip() != raw_path:
        raw_path = raw_path.rstrip()
    if raw_path.endswith("/") or raw_path.endswith("\\"):
        return "dir"
    if source_path.is_dir():
        return "dir"
    if source_path.is_file():
        return "file"
    return "unknown"


def _is_ignored_stage_path(path: Path) -> bool:
    return any(part in IGNORED_STAGE_NAMES for part in path.parts)


def _list_item_files(source_path: Path, item_kind: str) -> list[str]:
    if not source_path.exists():
        return []
    if item_kind == "file":
        if _is_ignored_stage_path(Path(source_path.name)):
            return []
        return [source_path.name]
    if item_kind != "dir":
        return []
    files: list[str] = []
    for path in sorted(source_path.rglob("*")):
        rel = path.relative_to(source_path)
        if _is_ignored_stage_path(rel):
            continue
        if path.is_file():
            files.append(path.relative_to(source_path).as_posix())
    return files


def _build_item_record(
    *,
    item_key: str | None,
    raw_path: str,
    source_rootfs: Path,
    project_source_rootfs: Path | None = None,
) -> dict[str, Any]:
    target_dir_name = _normalize_target_dir_name(raw_path)
    source_path = source_rootfs / Path(target_dir_name)
    item_kind = _detect_item_kind(raw_path, source_path)
    record: dict[str, Any] = {
        "raw_path": raw_path,
        "target_dir_name": target_dir_name,
        "source_path": str(source_path),
        "source_exists": source_path.exists(),
        "item_kind": item_kind,
        "file_items": _list_item_files(source_path, item_kind),
    }
    if project_source_rootfs is not None:
        record["project_source_path"] = str(project_source_rootfs / Path(target_dir_name))
    if item_key is not None:
        record["key"] = item_key
    return record


def _append_unique_item(target_items: list[dict[str, Any]], item: dict[str, Any]) -> None:
    target_dir_name = str(item.get("target_dir_name", "")).strip()
    source_path = str(item.get("source_path", "")).strip()
    for existing in target_items:
        if (
            str(existing.get("target_dir_name", "")).strip() == target_dir_name
            and str(existing.get("source_path", "")).strip() == source_path
        ):
            return
    target_items.append(item)


def _rebase_item_to_source_root(item: dict[str, Any], source_rootfs: Path) -> dict[str, Any]:
    rebased = dict(item)
    target_dir_name = str(rebased.get("target_dir_name", "")).strip()
    source_path = source_rootfs / Path(target_dir_name)
    item_kind = _detect_item_kind(str(rebased.get("raw_path", "")), source_path)
    rebased["source_path"] = str(source_path)
    rebased["source_exists"] = source_path.exists()
    rebased["item_kind"] = item_kind
    rebased["file_items"] = _list_item_files(source_path, item_kind)
    return rebased


def _merge_install_project_items(target_project: dict[str, Any], source_project: dict[str, Any], *, source_rootfs: Path) -> None:
    for item_name in ("module_items", "data_items", "clean_items"):
        source_items = source_project.get(item_name, []) or []
        if not isinstance(source_items, list):
            continue
        target_items = target_project.setdefault(item_name, [])
        for item in source_items:
            if isinstance(item, dict):
                _append_unique_item(target_items, _rebase_item_to_source_root(item, source_rootfs))


def _project_metadata(path: Path, data: dict[str, Any]) -> dict[str, str]:
    return {
        "project_name": str(data.get("name", path.stem)),
        "project_path": str(path),
    }


def _build_install_projects_from_config(
    *,
    project_file: Path,
    data: dict[str, Any],
    build_root: Path,
    publish_root: Path,
) -> dict[str, dict[str, Any]]:
    project_base = (project_file.parent / str(data.get("base_dir", "."))).resolve()
    metadata = _project_metadata(project_file, data)
    install_projects: dict[str, dict[str, Any]] = {}

    for app_key, app_cfg_raw in (data.get("apps", {}) or {}).items():
        app_cfg = app_cfg_raw or {}
        if not isinstance(app_cfg, dict):
            raise RuntimeError(f"apps.{app_key} must be a map")
        source_rootfs = (project_base / str(app_cfg.get("rootfs", "rootfs/"))).resolve()
        default_target_raw = str(app_cfg.get("default_target_rootfs", "${BUCKYOS_ROOT}"))
        modules = app_cfg.get("modules", {}) or {}
        data_paths = app_cfg.get("data_paths", []) or []
        clean_paths = app_cfg.get("clean_paths", []) or []

        if not isinstance(modules, dict):
            raise RuntimeError(f"apps.{app_key}.modules must be a map")
        if not isinstance(data_paths, list):
            raise RuntimeError(f"apps.{app_key}.data_paths must be a list")
        if not isinstance(clean_paths, list):
            raise RuntimeError(f"apps.{app_key}.clean_paths must be a list")

        staged_source_rootfs = (publish_root / str(app_key)).resolve()

        def build_item(item_key: str | None, raw_path: str) -> dict[str, Any]:
            item = _build_item_record(
                item_key=item_key,
                raw_path=raw_path,
                source_rootfs=staged_source_rootfs,
                project_source_rootfs=source_rootfs,
            )
            item.update(
                {
                    "source_project": metadata["project_name"],
                    "source_project_path": metadata["project_path"],
                    "source_app": str(app_key),
                }
            )
            if item_key is not None:
                item["source_item_key"] = item_key
            return item

        install_projects[str(app_key)] = {
            "key": str(app_key),
            "kind": "app",
            "name": str(app_cfg.get("name", app_key)),
            "app_key": str(app_key),
            "source_rootfs": str(staged_source_rootfs),
            "project_source_rootfs": str(source_rootfs),
            "default_target_rootfs_raw": default_target_raw,
            "default_target_rootfs": _expand_project_vars(default_target_raw, build_root=build_root),
            "source_project": metadata["project_name"],
            "source_project_path": metadata["project_path"],
            "module_items": [
                build_item(item_key=str(module_key), raw_path=str(module_path))
                for module_key, module_path in modules.items()
            ],
            "data_items": [
                build_item(item_key=None, raw_path=str(item))
                for item in data_paths
            ],
            "clean_items": [
                build_item(item_key=None, raw_path=str(item))
                for item in clean_paths
            ],
            "platforms": {},
        }

    return install_projects


def _find_cyfs_project_file() -> Path | None:
    candidates = [
        CYFS_SRC_DIR / "bucky_project.yaml",
        CYFS_SRC_DIR / "bucky_project.yml",
        CYFS_SRC_DIR / "bucky_project.json",
    ]
    for candidate in candidates:
        if candidate.exists():
            return candidate.resolve()
    return None


def _resolve_component_source(
    *,
    component_key: str,
    component_src: str | None,
    app_publish_dir: Path,
) -> Path:
    if component_src:
        src_path = Path(component_src)
        if src_path.is_absolute():
            return src_path
        return app_publish_dir / component_key / component_src
    return app_publish_dir / component_key


def _build_project_manifest(project_path: Path, *, build_root: Path, app_publish_dir: Path | None = None) -> dict[str, Any]:
    project_file = project_path.expanduser().resolve()
    data = _load_project_config(project_file)
    project_base = (project_file.parent / str(data.get("base_dir", "."))).resolve()
    publish_root = (app_publish_dir or build_root).expanduser().resolve()

    install_projects = _build_install_projects_from_config(
        project_file=project_file,
        data=data,
        build_root=build_root,
        publish_root=publish_root,
    )
    platform_manifest: dict[str, dict[str, Any]] = {
        "linux": {"component_keys": []},
        "macos": {"component_keys": []},
        "windows": {"component_keys": []},
    }

    cyfs_project_file = _find_cyfs_project_file()
    if cyfs_project_file is not None:
        cyfs_data = _load_project_config(cyfs_project_file)
        cyfs_projects = _build_install_projects_from_config(
            project_file=cyfs_project_file,
            data=cyfs_data,
            build_root=build_root,
            publish_root=publish_root,
        )
        cyfs_buckyos = cyfs_projects.get("cyfs-gateway")
        if cyfs_buckyos is not None and "buckyos" in install_projects:
            target_source_root = Path(str(install_projects["buckyos"].get("source_rootfs", publish_root / "buckyos"))).resolve()
            _merge_install_project_items(install_projects["buckyos"], cyfs_buckyos, source_rootfs=target_source_root)
            merged_sources = install_projects["buckyos"].setdefault("merged_from_projects", [])
            merged_sources.append(
                {
                    "project_name": str(cyfs_data.get("name", cyfs_project_file.stem)),
                    "project_path": str(cyfs_project_file),
                    "app_key": "cyfs-gateway",
                }
            )

    platform_publish = {
        "macos": (((data.get("publish", {}) or {}).get("macos_pkg", {}) or {}).get("apps", {}) or {}),
        "windows": (((data.get("publish", {}) or {}).get("win_pkg", {}) or {}).get("apps", {}) or {}),
    }
    for platform_key, components in platform_publish.items():
        if not isinstance(components, dict):
            raise RuntimeError(f"publish.{platform_key}.apps must be a map")
        for component_key, component_cfg_raw in components.items():
            platform_manifest[platform_key]["component_keys"].append(str(component_key))
            component_cfg = component_cfg_raw or {}
            if not isinstance(component_cfg, dict):
                raise RuntimeError(f"publish.{platform_key}.apps.{component_key} must be a map")
            project_key = str(component_key)
            project_record = install_projects.setdefault(
                project_key,
                {
                    "key": project_key,
                    "kind": str(component_cfg.get("type", "bundle") or "bundle"),
                    "name": str(component_cfg.get("name", component_key)),
                    "app_key": None,
                    "source_rootfs": None,
                    "default_target_rootfs_raw": None,
                    "default_target_rootfs": None,
                    "module_items": [],
                    "data_items": [],
                    "clean_items": [],
                    "platforms": {},
                },
            )

            component_kind = str(component_cfg.get("type", "app") or "app")
            app_key = project_record.get("app_key")
            if component_kind == "app" and app_key is None and project_key in install_projects and install_projects[project_key].get("app_key"):
                app_key = project_key
            component_src = str(component_cfg.get("src", "")).strip() or None
            default_target_raw = str(component_cfg.get("default_target", "")).strip()
            if not default_target_raw:
                raise RuntimeError(f"publish.{platform_key}.apps.{component_key} missing default_target")
            system_service_raw = component_cfg.get("system_service", False)
            if isinstance(system_service_raw, str):
                system_service = system_service_raw.lower().strip().rstrip(",") == "true"
            else:
                system_service = bool(system_service_raw)

            project_record["kind"] = component_kind
            project_record["name"] = str(component_cfg.get("name", project_record.get("name", component_key)))
            project_record["platforms"][platform_key] = {
                "key": project_key,
                "name": str(component_cfg.get("name", component_key)),
                "kind": component_kind,
                "optional": bool(component_cfg.get("optional", False)),
                "src": component_src,
                "source_path": str(
                    _resolve_component_source(
                        component_key=project_key,
                        component_src=component_src,
                        app_publish_dir=publish_root,
                    )
                ),
                "default_target_raw": default_target_raw,
                "default_target": _expand_project_vars(default_target_raw, build_root=build_root),
                "system_service": system_service,
            }

    linux_target = _expand_project_vars("${BUCKYOS_ROOT}", build_root=build_root)
    if "buckyos" in install_projects:
        platform_manifest["linux"]["component_keys"] = ["buckyos"]
        install_projects["buckyos"]["platforms"].setdefault(
            "linux",
            {
                "key": "buckyos",
                "name": str(install_projects["buckyos"].get("name", "buckyos")),
                "kind": "app",
                "optional": False,
                "src": None,
                "source_path": str(publish_root / "buckyos"),
                "default_target_raw": "${BUCKYOS_ROOT}",
                "default_target": linux_target,
                "system_service": True,
            },
        )

    return {
        "schema_version": 1,
        "generated_at": datetime.now().isoformat(timespec="seconds"),
        "project_path": str(project_file),
        "project_base": str(project_base),
        "build_root": str(build_root),
        "app_publish_dir": str(publish_root),
        "platforms": platform_manifest,
        "install_projects": install_projects,
    }


def _write_project_manifest(
    *,
    project_path: Path,
    build_root: Path,
    app_publish_dir: Path | None = None,
    out_path: Path | None = None,
) -> Path:
    manifest = _build_project_manifest(project_path, build_root=build_root, app_publish_dir=app_publish_dir)
    if out_path is not None:
        manifest_path = out_path.expanduser().resolve()
        manifest_path.parent.mkdir(parents=True, exist_ok=True)
        manifest_path.write_text(json.dumps(manifest, indent=2, sort_keys=False) + "\n", encoding="utf-8")
        return manifest_path

    with tempfile.NamedTemporaryFile(prefix="buckyos-pkg-manifest-", suffix=".json", delete=False) as tmp:
        manifest_path = Path(tmp.name)
    manifest_path.write_text(json.dumps(manifest, indent=2, sort_keys=False) + "\n", encoding="utf-8")
    return manifest_path


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


def _node_daemon_candidates(build_root: Path, target: TargetScript) -> list[Path]:
    candidates = [build_root / "buckyos" / "bin" / "node-daemon" / "node_daemon"]
    if target.platform_key == "windows":
        candidates.insert(0, build_root / "buckyos" / "bin" / "node-daemon" / "node_daemon.exe")
    return candidates


def _extract_versions_from_text(version_text: str) -> tuple[str, str]:
    full_match = re.search(r"\d+(?:\.\d+)*\+build\S+", version_text)
    short_match = re.search(r"\d+(?:\.\d+)*\+build\d+", version_text)
    if short_match is None:
        raise RuntimeError(f"unable to parse short version from node_daemon --version output: {version_text!r}")
    full_version = full_match.group(0) if full_match is not None else short_match.group(0)
    short_version = short_match.group(0)
    return short_version, full_version


def _extract_versions_from_binary_strings(candidate: Path) -> tuple[str, str]:
    completed = subprocess.run(
        ["strings", "-a", str(candidate)],
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    strings_text = completed.stdout.strip()
    if completed.returncode != 0:
        err = completed.stderr.strip() or completed.stdout.strip()
        raise RuntimeError(f"strings failed with exit code {completed.returncode}: {err}")
    if not strings_text:
        raise RuntimeError("strings produced no output")
    return _extract_versions_from_text(strings_text)


def _version_from_node_daemon(target: TargetScript) -> tuple[str, str]:
    errors: list[str] = []
    for candidate in _node_daemon_candidates(target.build_root, target):
        if not candidate.exists():
            continue
        try:
            completed = subprocess.run(
                [str(candidate), "--version"],
                check=False,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
        except OSError as exc:
            try:
                return _extract_versions_from_binary_strings(candidate)
            except RuntimeError as strings_exc:
                errors.append(f"{candidate}: exec failed ({exc}); strings fallback failed: {strings_exc}")
                continue
        version_text = "\n".join(part for part in [completed.stdout.strip(), completed.stderr.strip()] if part).strip()
        if completed.returncode != 0:
            errors.append(f"{candidate}: exited with {completed.returncode}: {version_text}")
            continue
        if not version_text:
            errors.append(f"{candidate}: empty version output")
            continue
        try:
            return _extract_versions_from_text(version_text)
        except RuntimeError as exc:
            errors.append(f"{candidate}: {exc}")
    if errors:
        raise RuntimeError("failed to determine package version from node_daemon --version:\n- " + "\n- ".join(errors))
    raise RuntimeError(
        "failed to determine package version: node_daemon binary not found under "
        f"{target.build_root / 'buckyos' / 'bin' / 'node-daemon'}"
    )


def _python_executable() -> str:
    current = (sys.executable or "").strip()
    if current:
        if os.name != "nt" or "windowsapps" not in current.lower():
            return current

    if os.name == "nt":
        venv = (os.environ.get("VIRTUAL_ENV") or "").strip()
        if venv:
            venv_python = Path(venv) / "Scripts" / "python.exe"
            if venv_python.exists():
                return str(venv_python)

    return current or "python"


def _pnpm_command() -> list[str]:
    for name in ("pnpm", "pnpm.cmd", "pnpm.exe"):
        path = shutil.which(name)
        if path:
            return [path]

    for corepack_name in ("corepack", "corepack.cmd", "corepack.exe"):
        corepack_path = shutil.which(corepack_name)
        if corepack_path:
            return [corepack_path, "pnpm"]

    raise RuntimeError(
        "pnpm not found in PATH. "
        "Please install pnpm (or enable it via corepack) before building desktop app."
    )


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
        shutil.copytree(src, dst, ignore=shutil.ignore_patterns(*IGNORED_STAGE_NAMES))
    else:
        shutil.copy2(src, dst)


def _ensure_executable(path: Path, *, dry_run: bool) -> None:
    print("[chmod +x]", path)
    if dry_run or not path.exists():
        return
    path.chmod(path.stat().st_mode | 0o111)


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


def _desktop_target_triple(target: TargetScript) -> str | None:
    if target.platform_key == "windows":
        arch = "x86_64" if target.architecture == "amd64" else "aarch64"
        return f"{arch}-pc-windows-msvc"
    if target.platform_key == "macos":
        arch = "x86_64" if target.architecture == "amd64" else "aarch64"
        return f"{arch}-apple-darwin"
    return None


def _desktop_build_roots() -> list[Path]:
    rust_build_root = _rust_build_root()
    roots = [
        rust_build_root,
        rust_build_root / "target",
        DESKTOP_APP_REPO_DIR / "src-tauri" / "target",
    ]
    unique_roots: list[Path] = []
    for root in roots:
        if root not in unique_roots:
            unique_roots.append(root)
    return unique_roots


def _built_desktop_app_candidates(target: TargetScript) -> list[Path]:
    target_triple = _desktop_target_triple(target)

    if target.platform_key == "macos":
        candidates: list[Path] = []
        for root in _desktop_build_roots():
            candidates.append(root / "release" / "bundle" / "macos" / "BuckyOS.app")
            if target_triple:
                candidates.append(root / target_triple / "release" / "bundle" / "macos" / "BuckyOS.app")
        return candidates
    if target.platform_key == "windows":
        candidates = []
        for root in _desktop_build_roots():
            release_roots = [root / "release"]
            if target_triple:
                release_roots.append(root / target_triple / "release")
            for release_root in release_roots:
                candidates.extend(
                    [
                        release_root / "buckyosapp.exe",
                        release_root / "BuckyOS.exe",
                    ]
                )
        return candidates
    return []


def _build_desktop_app(target: TargetScript, *, dry_run: bool) -> Path | None:
    if target.platform_key not in ("macos", "windows"):
        return None
    if not DESKTOP_APP_REPO_DIR.exists():
        return None

    build_cmd = _pnpm_command() + ["tauri", "build"]
    if target.platform_key == "macos":
        build_cmd += ["--bundles", "app"]
    elif target.platform_key == "windows":
        build_arch = "x86_64" if target.architecture == "amd64" else "aarch64"
        build_cmd += ["--no-bundle", "--", "--target", f"{build_arch}-pc-windows-msvc"]
    _run_checked(build_cmd, cwd=DESKTOP_APP_REPO_DIR, dry_run=dry_run)
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

    searched = _built_desktop_app_candidates(target)
    searched_text = ", ".join(str(path) for path in searched)
    raise RuntimeError(
        "desktop app artifact is required but missing. "
        f"Expected {expected}, or place BuckyOSApp beside this repo, or pass --desktop-app <path>. "
        f"Searched: {searched_text}"
    )


def _prepare_common_build_root(
    *,
    target: TargetScript,
    dry_run: bool,
    skip_cargo_update: bool,
    skip_cyfs_gateway: bool,
    desktop_app: str | None,
    skip_desktop_app_build: bool,
    rust_target: str | None = None,
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
        cyfs_build_cmd = ["buckyos-build"]
        if rust_target:
            cyfs_build_cmd.append(f"--target={rust_target}")
        _run_checked(cyfs_build_cmd, cwd=CYFS_SRC_DIR, dry_run=dry_run)
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
    buckyos_build_cmd = ["buckyos-build"]
    if rust_target:
        buckyos_build_cmd.append(f"--target={rust_target}")
    _run_checked(buckyos_build_cmd, cwd=SRC_DIR, dry_run=dry_run)
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
    _ensure_executable(buckyos_root / "bin" / "stop_osx.sh", dry_run=dry_run)
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
    parser.add_argument("--rust-target", default=None, help="Pass explicit --target to internal buckyos-build commands")
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
        rust_target=args.rust_target,
    )
    return 0


def build_pkg(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(prog="make_local_pkg.py build-pkg")
    parser.add_argument("version", nargs="?", help="Package version, defaults to short_version parsed from node_daemon --version")
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
    parser.add_argument("--rust-target", default=None, help="Pass explicit --target to internal buckyos-build commands")
    parser.add_argument("--no-sync-scripts", action="store_true")
    parser.add_argument("--dry-run", action="store_true")
    args, forwarded = parser.parse_known_args(argv)

    build_root_override = args.build_root or args.app_publish_dir
    target = detect_target(args.arch, build_root_override)

    if not args.skip_prepare:
        _prepare_common_build_root(
            target=target,
            dry_run=bool(args.dry_run),
            skip_cargo_update=bool(args.skip_cargo_update),
            skip_cyfs_gateway=bool(args.skip_cyfs_gateway),
            desktop_app=args.desktop_app,
            skip_desktop_app_build=bool(args.skip_desktop_app_build),
            rust_target=args.rust_target,
        )
    else:
        print("[prepare] skipped by --skip-prepare")

    if args.version:
        version = args.version
    else:
        short_version, full_version = _version_from_node_daemon(target)
        version = short_version
        print(f"[version] full_version={full_version}")
        print(f"[version] short_version={short_version}")

    manifest_path = _write_project_manifest(
        project_path=Path(args.project),
        build_root=target.build_root,
        app_publish_dir=target.build_root,
    )
    try:
        cmd = [
            _python_executable(),
            str(target.script_path),
            "build-pkg",
            target.architecture,
            version,
            "--manifest",
            str(manifest_path),
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
    finally:
        manifest_path.unlink(missing_ok=True)


def verify_pkg(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(prog="make_local_pkg.py verify-pkg")
    parser.add_argument("pkg", help="Path to package file")
    parser.add_argument("--arch", default=None, help="Override detected architecture")
    parser.add_argument("--build-root", default=None, help="Override BUCKYOS_BUILD_ROOT")
    parser.add_argument("--project", default=str(SRC_DIR / "bucky_project.yaml"))
    args, forwarded = parser.parse_known_args(argv)

    target = detect_target(args.arch, args.build_root)
    manifest_path = _write_project_manifest(
        project_path=Path(args.project),
        build_root=target.build_root,
        app_publish_dir=target.build_root,
    )
    try:
        base_cmd = [_python_executable(), str(target.script_path)]
        if target.platform_key == "windows":
            cmd = base_cmd + ["verify-pkg", "--pkg", args.pkg, "--manifest", str(manifest_path)]
        else:
            cmd = base_cmd + ["verify-pkg", args.pkg, "--manifest", str(manifest_path)]
        cmd += forwarded
        return _run(cmd)
    finally:
        manifest_path.unlink(missing_ok=True)


def sync_scripts(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(prog="make_local_pkg.py sync-scripts")
    parser.add_argument("--arch", default=None, help="Override detected architecture")
    parser.add_argument("--build-root", default=None, help="Override BUCKYOS_BUILD_ROOT")
    parser.add_argument("--project", default=str(SRC_DIR / "bucky_project.yaml"))
    args, forwarded = parser.parse_known_args(argv)

    target = detect_target(args.arch, args.build_root)
    if target.platform_key == "linux":
        raise RuntimeError("sync-scripts is not supported for Linux packages")

    manifest_path = _write_project_manifest(
        project_path=Path(args.project),
        build_root=target.build_root,
        app_publish_dir=target.build_root,
    )
    if target.platform_key == "macos":
        cmd = [_python_executable(), str(target.script_path), "sync-macos-scripts", "--manifest", str(manifest_path)]
    else:
        cmd = [_python_executable(), str(target.script_path), "sync", "--manifest", str(manifest_path)]
    try:
        cmd += forwarded
        return _run(cmd)
    finally:
        manifest_path.unlink(missing_ok=True)


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

    manifest_path = _write_project_manifest(
        project_path=Path(args.project),
        build_root=target.build_root,
        app_publish_dir=target.build_root,
    )
    try:
        cmd = [_python_executable(), str(target.script_path), action, "--manifest", str(manifest_path)]
        if args.target:
            cmd += ["--target", args.target]
        if args.source:
            cmd += ["--source", args.source]
        if args.dry_run:
            cmd.append("--dry-run")
        cmd += forwarded
        return _run(cmd)
    finally:
        manifest_path.unlink(missing_ok=True)


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


def show_manifest(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(prog="make_local_pkg.py show-manifest")
    parser.add_argument("--arch", default=None, help="Override detected architecture")
    parser.add_argument("--build-root", default=None, help="Override BUCKYOS_BUILD_ROOT")
    parser.add_argument("--app-publish-dir", default=None, help="Override app publish dir in generated manifest")
    parser.add_argument("--project", default=str(SRC_DIR / "bucky_project.yaml"))
    parser.add_argument("--out", default=None, help="Write manifest JSON to file instead of stdout")
    args = parser.parse_args(argv)

    target = detect_target(args.arch, args.build_root)
    app_publish_dir = Path(args.app_publish_dir).expanduser() if args.app_publish_dir else target.build_root
    if args.out:
        manifest_path = _write_project_manifest(
            project_path=Path(args.project),
            build_root=target.build_root,
            app_publish_dir=app_publish_dir,
            out_path=Path(args.out),
        )
        print(manifest_path)
        return 0

    manifest = _build_project_manifest(
        Path(args.project),
        build_root=target.build_root,
        app_publish_dir=app_publish_dir,
    )
    print(json.dumps(manifest, indent=2, sort_keys=False))
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
            "  show-manifest  Print the generated project manifest JSON\n"
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
        if command == "show-manifest":
            return show_manifest(argv[2:])
        raise RuntimeError(f"unknown command: {command}")
    except RuntimeError as err:
        print(f"error: {err}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
