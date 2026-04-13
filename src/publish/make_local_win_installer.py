"""
Windows NSIS Installer Builder for BuckyOS.

This script supports:
- build-pkg:   Build a Windows .exe installer using NSIS
- verify-pkg:  Verify a built installer using 7zip
- verify:      Alias of verify-pkg for backward compatibility
- sync:        Regenerate PowerShell scripts from bucky_project.yaml

It reads:
- `apps.buckyos.*` for app layout configuration.
- `publish.win_pkg.apps.*` for Windows distribution package components.

Before building, ensure you have built the latest components:
- buckyos-build && buckyos-install --all --target-rootfs=C:\\opt\\buckyosci\\buckyos
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Dict, List, Optional

try:
    import yaml  # type: ignore
except ImportError:  # pragma: no cover
    yaml = None  # type: ignore[assignment]


SRC_DIR = Path(__file__).resolve().parent.parent
PROJECT_YAML = SRC_DIR / "bucky_project.yaml"

RESULT_ROOT_DIR = Path(os.environ.get("BUCKYOS_BUILD_ROOT", "C:\\opt\\buckyosci"))
TMP_INSTALL_DIR = RESULT_ROOT_DIR / "win-installer"

WIN_PKG_PROJECT_DIR = SRC_DIR / "publish" / "win_pkg"
BUCKYOS_DEFAULTS_SUBDIR = ".buckyos_installer_defaults"
IGNORED_STAGE_NAMES = {".DS_Store", "__pycache__"}


def yaml_load_file(path: Path) -> Dict[str, Any]:
    if yaml is None:
        raise ImportError(
            "PyYAML is required to read bucky_project.yaml. "
            "Use your project venv or install via `pip install pyyaml`."
        )
    data = yaml.safe_load(path.read_text(encoding="utf-8"))
    if data is None:
        return {}
    if not isinstance(data, dict):
        raise ValueError(f"YAML root must be a map: {path}")
    return data


def json_load_file(path: Path) -> Dict[str, Any]:
    data = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(data, dict):
        raise ValueError(f"JSON root must be a map: {path}")
    return data


def _expand_vars(s: str) -> str:
    """Expand environment variables in path strings."""
    out = s
    for name, default in [
        ("BUCKYOS_ROOT", "C:\\opt\\buckyos"),
        ("BUCKYOS_BUILD_ROOT", str(RESULT_ROOT_DIR)),
        ("APPDATA", os.environ.get("APPDATA") or "%APPDATA%"),
        ("LOCALAPPDATA", os.environ.get("LOCALAPPDATA") or "%LOCALAPPDATA%"),
        ("USERPROFILE", os.environ.get("USERPROFILE") or "%USERPROFILE%"),
    ]:
        val = os.environ.get(name, default)
        out = out.replace(f"${{{name}}}", val)
        out = out.replace(f"%{name}%", val)
    return out


def _manifest_install_project(manifest_path: Path, project_key: str) -> Dict[str, Any]:
    data = json_load_file(manifest_path)
    install_projects = data.get("install_projects", {}) or {}
    if not isinstance(install_projects, dict):
        raise ValueError("manifest.install_projects must be a map")
    project = install_projects.get(project_key)
    if not isinstance(project, dict):
        raise ValueError(f"manifest.install_projects.{project_key} missing or invalid")
    return project


@dataclass(frozen=True)
class AppLayout:
    source_rootfs: Path
    target_rootfs: Path
    module_paths: List[str]
    data_paths: List[str]
    clean_paths: List[str]


def _ignore_copy_entries(_: str, names: List[str]) -> List[str]:
    return [name for name in names if name in IGNORED_STAGE_NAMES]


def _copytree_filtered(src: Path, dst: Path) -> None:
    shutil.copytree(src, dst, dirs_exist_ok=True, ignore=_ignore_copy_entries)


def load_app_layout(
    project_yaml_path: Path,
    app_key: str,
    target_override: str | None = None,
) -> AppLayout:
    data = yaml_load_file(project_yaml_path)
    apps = data.get("apps", {}) or {}
    app_cfg = apps.get(app_key, {}) or {}

    base_dir = str(data.get("base_dir", "."))
    project_base = (project_yaml_path.parent / base_dir).resolve()

    rootfs_rel = str(app_cfg.get("rootfs", "rootfs/"))
    source_rootfs = (project_base / rootfs_rel).resolve()

    default_target = str(app_cfg.get("default_target_rootfs", "${BUCKYOS_ROOT}"))
    target_str = target_override if target_override else default_target
    target_rootfs = Path(_expand_vars(target_str)).resolve()

    modules = app_cfg.get("modules", {}) or {}
    module_paths = [str(p) for p in modules.values()]
    data_paths = [str(p) for p in (app_cfg.get("data_paths", []) or [])]
    clean_paths = [str(p) for p in (app_cfg.get("clean_paths", []) or [])]

    return AppLayout(
        source_rootfs=source_rootfs,
        target_rootfs=target_rootfs,
        module_paths=module_paths,
        data_paths=data_paths,
        clean_paths=clean_paths,
    )


def load_app_layout_from_manifest(
    manifest_path: Path,
    app_key: str,
    target_override: str | None = None,
) -> AppLayout:
    app_cfg = _manifest_install_project(manifest_path, app_key)
    source_rootfs_raw = app_cfg.get("source_rootfs")
    if not isinstance(source_rootfs_raw, str) or not source_rootfs_raw.strip():
        raise ValueError(f"manifest install project '{app_key}' missing source_rootfs")

    default_target = str(
        app_cfg.get("default_target_rootfs")
        or app_cfg.get("default_target_rootfs_raw")
        or "${BUCKYOS_ROOT}"
    )
    target_str = target_override if target_override else default_target

    def item_paths(name: str) -> List[str]:
        items = app_cfg.get(name, []) or []
        if not isinstance(items, list):
            raise ValueError(f"manifest install project '{app_key}'.{name} must be a list")
        return [
            str(item.get("raw_path") or item.get("target_dir_name") or "")
            for item in items
            if isinstance(item, dict) and str(item.get("raw_path") or item.get("target_dir_name") or "").strip()
        ]

    return AppLayout(
        source_rootfs=Path(source_rootfs_raw).resolve(),
        target_rootfs=Path(_expand_vars(target_str)).resolve(),
        module_paths=item_paths("module_items"),
        data_paths=item_paths("data_items"),
        clean_paths=item_paths("clean_items"),
    )


def resolve_app_layout(
    *,
    app_key: str,
    project_yaml_path: Path,
    manifest_path: Path | None = None,
    target_override: str | None = None,
) -> AppLayout:
    if manifest_path is not None:
        return load_app_layout_from_manifest(manifest_path, app_key, target_override=target_override)
    return load_app_layout(project_yaml_path, app_key, target_override=target_override)


def load_buckyos_layout(project_yaml_path: Path = PROJECT_YAML, target_override: str | None = None) -> AppLayout:
    return load_app_layout(project_yaml_path, "buckyos", target_override=target_override)


def _as_str(v: Any) -> str:
    if v is None:
        return ""
    return str(v)


def _sanitize_id(s: str) -> str:
    out = []
    for ch in s:
        if ch.isalnum() or ch in (".", "-", "_"):
            out.append(ch.lower())
        else:
            out.append("-")
    cleaned = "".join(out).strip("-")
    return cleaned or "component"


@dataclass(frozen=True)
class PublishComponent:
    key: str
    name: str
    kind: str  # "app" | "bundle"
    optional: bool
    src: str | None
    default_target: str
    system_service: bool


def load_win_pkg_components(project_yaml_path: Path) -> List[PublishComponent]:
    data = yaml_load_file(project_yaml_path)
    publish = data.get("publish", {}) or {}
    win_pkg = publish.get("win_pkg", {}) or {}
    apps = win_pkg.get("apps", {}) or {}
    if not isinstance(apps, dict):
        raise ValueError("publish.win_pkg.apps must be a map")

    components: List[PublishComponent] = []
    for key, cfg in apps.items():
        if not isinstance(cfg, dict):
            raise ValueError(f"publish.win_pkg.apps.{key} must be a map")
        name = _as_str(cfg.get("name", key)).strip() or key
        kind = _as_str(cfg.get("type", "")).strip() or "app"
        optional = bool(cfg.get("optional", False))
        src = _as_str(cfg.get("src", "")).strip() or None
        default_target = _as_str(cfg.get("default_target", "")).strip()
        if not default_target:
            raise ValueError(f"publish.win_pkg.apps.{key} missing default_target")
        # Handle 'true,' string (YAML parsing quirk)
        system_service_val = cfg.get("system_service", False)
        if isinstance(system_service_val, str):
            system_service = system_service_val.lower().strip().rstrip(",") == "true"
        else:
            system_service = bool(system_service_val)
        components.append(
            PublishComponent(
                key=_as_str(key),
                name=name,
                kind=kind,
                optional=optional,
                src=src,
                default_target=default_target,
                system_service=system_service,
            )
        )
    return components


def load_win_pkg_components_from_manifest(manifest_path: Path) -> List[PublishComponent]:
    data = json_load_file(manifest_path)
    install_projects = data.get("install_projects", {}) or {}
    platforms = data.get("platforms", {}) or {}
    win_cfg = platforms.get("windows", {}) or {}
    component_keys = win_cfg.get("component_keys", []) or []
    if not isinstance(install_projects, dict):
        raise ValueError("manifest.install_projects must be a map")
    if not isinstance(component_keys, list):
        raise ValueError("manifest.platforms.windows.component_keys must be a list")

    components: List[PublishComponent] = []
    for key in component_keys:
        project = install_projects.get(key)
        if not isinstance(project, dict):
            raise ValueError(f"manifest install project missing for Windows component: {key}")
        platform_cfg = (project.get("platforms", {}) or {}).get("windows")
        if not isinstance(platform_cfg, dict):
            raise ValueError(f"manifest install project '{key}' missing windows platform config")
        system_service_val = platform_cfg.get("system_service", False)
        if isinstance(system_service_val, str):
            system_service = system_service_val.lower().strip().rstrip(",") == "true"
        else:
            system_service = bool(system_service_val)
        components.append(
            PublishComponent(
                key=_as_str(platform_cfg.get("key", key)),
                name=_as_str(platform_cfg.get("name", project.get("name", key))).strip() or _as_str(key),
                kind=_as_str(platform_cfg.get("kind", project.get("kind", "app"))).strip() or "app",
                optional=bool(platform_cfg.get("optional", False)),
                src=_as_str(platform_cfg.get("src", "")).strip() or None,
                default_target=_as_str(platform_cfg.get("default_target", "")).strip(),
                system_service=system_service,
            )
        )
    return components


def _resolve_component_src(component: PublishComponent, app_publish_dir: Path) -> Path:
    """Resolve the source path for a component."""
    if component.src:
        p = Path(component.src)
        if p.is_absolute():
            return p
        return app_publish_dir / component.key / component.src
    return app_publish_dir / component.key


def _resolve_component_target(component: PublishComponent) -> str:
    """Return the target path (expanded) for a component."""
    return _expand_vars(component.default_target)


def _copy_dir_contents(src_dir: Path, dst_dir: Path) -> None:
    dst_dir.mkdir(parents=True, exist_ok=True)
    for child in src_dir.iterdir():
        if child.name in IGNORED_STAGE_NAMES:
            continue
        dst = dst_dir / child.name
        if child.is_dir():
            _copytree_filtered(child, dst)
        else:
            dst.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(child, dst)


def _iter_payload_file_basenames(root: Path, *, limit: int = 256) -> List[str]:
    names: List[str] = []
    if not root.exists():
        return names
    if root.is_file():
        return [root.name.lower()]

    for path in sorted(root.rglob("*")):
        if not path.is_file():
            continue
        rel = path.relative_to(root)
        if any(part in IGNORED_STAGE_NAMES for part in rel.parts):
            continue
        names.append(path.name.lower())
        if len(names) >= limit:
            break
    return names


def _append_unique_names(target: List[str], values: List[str]) -> None:
    seen = set(target)
    for value in values:
        normalized = value.strip().lower()
        if not normalized or normalized in seen:
            continue
        target.append(normalized)
        seen.add(normalized)


def _component_expected_filenames(
    *,
    comp: "PublishComponent",
    manifest_path: Path | None,
) -> List[str]:
    expected: List[str] = []

    if comp.src:
        _append_unique_names(expected, [Path(comp.src.replace("\\", "/")).name])

    if comp.key == "buckyos":
        _append_unique_names(
            expected,
            [
                "node_daemon.exe",
                "seed_defaults.ps1",
                "ensure_firewall_rules.ps1",
                "uninstall_cleanup.ps1",
                "node_daemon_loader.ps1",
                "node_daemon_loader.vbs",
            ],
        )

    if manifest_path is None:
        return expected

    try:
        project = _manifest_install_project(manifest_path, comp.key)
    except Exception:
        return expected

    platform_cfg = (project.get("platforms", {}) or {}).get("windows", {}) or {}
    source_path_raw = str(platform_cfg.get("source_path") or "").strip()
    if source_path_raw:
        _append_unique_names(expected, _iter_payload_file_basenames(Path(source_path_raw)))

    for item_name in ("module_items", "data_items"):
        items = project.get(item_name, []) or []
        if not isinstance(items, list):
            continue
        for item in items:
            if not isinstance(item, dict):
                continue
            file_items = item.get("file_items", []) or []
            if isinstance(file_items, list):
                _append_unique_names(
                    expected,
                    [Path(str(file_item).replace("\\", "/")).name for file_item in file_items],
                )

    return expected


def _stage_buckyos_app_root(*, src_root: Path, dst_root: Path, layout: AppLayout) -> None:
    """
    Stage buckyos rootfs into dst_root.

    Semantics:
    - modules: always copied into real target paths (will be overwritten by installer)
    - data_paths: copied into `${INSTALL_DIR}\\.buckyos_installer_defaults\\...`
      and postinstall will copy to real paths only if missing
    """
    # modules -> real target
    for rel in layout.module_paths:
        rel_s = rel.strip()
        if rel_s.startswith("/") or rel_s.startswith("\\"):
            rel_s = rel_s[1:]
        rel_s = rel_s.rstrip("/\\")
        s = src_root / rel_s
        d = dst_root / rel_s
        if s.is_dir():
            _copytree_filtered(s, d)
        elif s.exists():
            d.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(s, d)

    # data_paths -> defaults area
    defaults_root = dst_root / BUCKYOS_DEFAULTS_SUBDIR
    for rel in layout.data_paths:
        rel_s = rel.strip()
        if rel_s.startswith("/") or rel_s.startswith("\\"):
            rel_s = rel_s[1:]
        rel_s = rel_s.rstrip("/\\")
        s = src_root / rel_s
        d = defaults_root / rel_s
        if not s.exists():
            print(f"[warn] data_paths source missing: '{rel}' -> '{s}', skipping")
            continue
        if s.is_dir():
            _copytree_filtered(s, d)
        else:
            d.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(s, d)


def _run(cmd: List[str], dry_run: bool, cwd: Path | None = None) -> int:
    print("+", " ".join(cmd))
    if dry_run:
        return 0
    result = subprocess.run(cmd, cwd=cwd)
    return result.returncode


def _escape_nsis_path(s: str) -> str:
    """Escape path for NSIS."""
    return s.replace("\\", "\\\\")


def _escape_nsis_string(s: str) -> str:
    """Escape string for NSIS."""
    return s.replace('"', '$\\"').replace("\\", "\\\\")


def generate_nsis_script(
    *,
    title: str,
    version: str,
    architecture: str,
    components: List[PublishComponent],
    payload_dir: Path,
    out_path: Path,
    license_file: Path | None = None,
    bundled_vcredist: Path | None = None,
) -> None:
    """Generate the NSIS installer script."""
    
    # Map architecture
    if architecture in ("amd64", "x86_64"):
        nsis_arch = "x64"
        allow_arch = "x64"
    elif architecture in ("arm64", "aarch64"):
        nsis_arch = "arm64"
        allow_arch = "arm64"
    else:
        nsis_arch = "x86"
        allow_arch = ""

    launch_bundle = next((comp for comp in components if comp.kind == "bundle"), None)

    lines: List[str] = []
    
    # Header and definitions
    lines.append("; BuckyOS Windows Installer - AUTO-GENERATED")
    lines.append(f"; Version: {version}")
    lines.append(f"; Architecture: {architecture}")
    lines.append("")
    lines.append("!include \"MUI2.nsh\"")
    lines.append("!include \"LogicLib.nsh\"")
    lines.append("!include \"FileFunc.nsh\"")
    lines.append("!include \"nsDialogs.nsh\"")
    lines.append("!include \"WinMessages.nsh\"")
    
    # Include x64 support for 64-bit installers
    if nsis_arch == "x64":
        lines.append("!include \"x64.nsh\"")
    lines.append("")
    
    lines.append(f'!define PRODUCT_NAME "{title}"')
    lines.append(f'!define PRODUCT_VERSION "{version}"')
    lines.append('!define PRODUCT_PUBLISHER "BuckyOS"')
    lines.append('!define PRODUCT_WEB_SITE "https://github.com/buckyos"')
    lines.append(f'!define PRODUCT_ARCH "{architecture}"')
    lines.append("")
    
    # Installer attributes
    lines.append(f'Name "${{PRODUCT_NAME}} ${{PRODUCT_VERSION}}"')
    lines.append(f'OutFile "buckyos-win-{architecture}-{version}.exe"')
    
    # Set default install directory based on architecture
    if nsis_arch == "x64":
        lines.append('InstallDir "$PROGRAMFILES64\\BuckyOS"')
    else:
        lines.append('InstallDir "$PROGRAMFILES\\BuckyOS"')
    
    lines.append('InstallDirRegKey HKCU "Software\\BuckyOS" "InstallDir"')
    lines.append("RequestExecutionLevel admin")
    lines.append("ShowInstDetails show")
    lines.append("ShowUninstDetails show")
    lines.append("SetCompressor /SOLID lzma")
    lines.append("")
    
    # MUI settings
    lines.append("; MUI Settings")
    lines.append("!define MUI_ABORTWARNING")
    lines.append("!define MUI_ICON \"${NSISDIR}\\Contrib\\Graphics\\Icons\\modern-install.ico\"")
    lines.append("!define MUI_UNICON \"${NSISDIR}\\Contrib\\Graphics\\Icons\\modern-uninstall.ico\"")
    if launch_bundle is not None:
        lines.append("!define MUI_FINISHPAGE_RUN")
        lines.append(f'!define MUI_FINISHPAGE_RUN_TEXT "Launch {launch_bundle.name}"')
        lines.append('!define MUI_FINISHPAGE_RUN_FUNCTION "LaunchInstalledApplication"')
    lines.append("")

    # Variables for each component's install directory
    lines.append("; Variables for component install directories")
    lines.append("Var DockerCheckCode")
    lines.append("Var PortCheckCode")
    lines.append("Var PortCheckPort")
    lines.append("Var VCRedistInstalled")
    lines.append("Var VCRedistVersion")
    lines.append("Var ExistingBuckyRoot")
    lines.append("Var BestInstallDrive")
    lines.append("Var LaunchAfterInstall")
    for comp in components:
        var_name = f"InstDir_{_sanitize_id(comp.key).replace('-', '_')}"
        lines.append(f"Var {var_name}")
    lines.append("")
    
    # Custom directory page variables
    lines.append("; Custom directory page variables")
    lines.append("Var Dialog")
    lines.append("Var Label")
    for comp in components:
        var_name = f"DirReq_{_sanitize_id(comp.key).replace('-', '_')}"
        lines.append(f"Var {var_name}")
    lines.append("")
    
    # Pages - use custom directory page
    lines.append("; Installer pages")
    lines.append("!insertmacro MUI_PAGE_WELCOME")
    if license_file and license_file.exists():
        lines.append(f'!insertmacro MUI_PAGE_LICENSE "{license_file}"')
    lines.append("!insertmacro MUI_PAGE_COMPONENTS")
    lines.append("Page custom DirectoryPageCreate DirectoryPageLeave")
    lines.append("!insertmacro MUI_PAGE_INSTFILES")
    lines.append("!insertmacro MUI_PAGE_FINISH")
    lines.append("")
    lines.append("; Uninstaller pages")
    lines.append("!insertmacro MUI_UNPAGE_CONFIRM")
    lines.append("!insertmacro MUI_UNPAGE_INSTFILES")
    lines.append("")
    lines.append("; Language")
    lines.append('!insertmacro MUI_LANGUAGE "English"')
    lines.append("")
    
    # Custom directory page function
    lines.append("; Custom directory page - allows selecting install path for each component")
    lines.append("Function DirectoryPageCreate")
    lines.append('  !insertmacro MUI_HEADER_TEXT "Choose Install Locations" "Choose the folder in which to install each component."')
    lines.append("  nsDialogs::Create 1018")
    lines.append("  Pop $Dialog")
    lines.append('  ${If} $Dialog == error')
    lines.append("    Abort")
    lines.append('  ${EndIf}')
    lines.append("")
    
    # Create UI elements for each component
    y_pos = 0
    for idx, comp in enumerate(components):
        var_name = f"InstDir_{_sanitize_id(comp.key).replace('-', '_')}"
        dir_req_var = f"DirReq_{_sanitize_id(comp.key).replace('-', '_')}"
        section_id = f"SEC_{_sanitize_id(comp.key).upper()}"
        
        lines.append(f'  ; {comp.name} directory selection')
        lines.append(f'  ${{NSD_CreateLabel}} 0 {y_pos}u 100% 12u "{comp.name} Install Location:"')
        lines.append(f'  Pop $Label')
        lines.append(f'  ${{NSD_CreateDirRequest}} 0 {y_pos + 14}u 280u 12u "${var_name}"')
        lines.append(f'  Pop ${dir_req_var}')
        lines.append(f'  ${{NSD_CreateBrowseButton}} 285u {y_pos + 13}u 60u 14u "Browse..."')
        lines.append(f'  Pop $0')
        lines.append(f'  ${{NSD_OnClick}} $0 OnBrowse{idx}')
        lines.append("")
        y_pos += 40
    
    lines.append("  nsDialogs::Show")
    lines.append("FunctionEnd")
    lines.append("")
    
    # Browse button callbacks for each component
    for idx, comp in enumerate(components):
        var_name = f"InstDir_{_sanitize_id(comp.key).replace('-', '_')}"
        dir_req_var = f"DirReq_{_sanitize_id(comp.key).replace('-', '_')}"
        
        lines.append(f"Function OnBrowse{idx}")
        lines.append(f'  nsDialogs::SelectFolderDialog "Select Install Location for {comp.name}" ${var_name}')
        lines.append(f'  Pop $0')
        lines.append(f'  ${{If}} $0 != error')
        lines.append(f'    StrCpy ${var_name} $0')
        lines.append(f'    ${{NSD_SetText}} ${dir_req_var} ${var_name}')
        lines.append(f'  ${{EndIf}}')
        lines.append("FunctionEnd")
        lines.append("")
    
    # Directory page leave function - save values
    lines.append("Function DirectoryPageLeave")
    for comp in components:
        var_name = f"InstDir_{_sanitize_id(comp.key).replace('-', '_')}"
        dir_req_var = f"DirReq_{_sanitize_id(comp.key).replace('-', '_')}"
        lines.append(f'  ${{NSD_GetText}} ${dir_req_var} ${var_name}')
    lines.append("FunctionEnd")
    lines.append("")

    # Runtime dependency checks
    lines.append("; Runtime dependency checks")
    lines.append("Function CheckDockerDesktopForUser")
    lines.append('  StrCpy $DockerCheckCode "0"')
    lines.append('  nsExec::Exec \'cmd /c "docker version >nul 2>nul"\'')
    lines.append("  Pop $0")
    lines.append('  ${If} $0 != 0')
    lines.append('    StrCpy $DockerCheckCode "1"')
    lines.append("    Return")
    lines.append("  ${EndIf}")
    lines.append("FunctionEnd")
    lines.append("")

    lines.append("Function CheckRequiredPorts")
    lines.append('  StrCpy $PortCheckCode "0"')
    lines.append('  StrCpy $PortCheckPort ""')
    for port in ("3180", "80", "443"):
        lines.append(f'  nsExec::ExecToStack \'powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "$$p={port}; try {{ $$listener=[System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Any,$$p); $$listener.Start(); $$listener.Stop(); exit 0 }} catch {{ exit 1 }}"\'')
        lines.append("  Pop $0")
        lines.append('  ${If} $0 != 0')
        lines.append('    StrCpy $PortCheckCode "1"')
        lines.append(f'    StrCpy $PortCheckPort "{port}"')
        lines.append("    Return")
        lines.append("  ${EndIf}")
    lines.append("FunctionEnd")
    lines.append("")

    lines.append("Function IsVCRedistInstalled")
    lines.append('  StrCpy $VCRedistInstalled "0"')
    lines.append('  StrCpy $VCRedistVersion ""')
    lines.append('  ReadRegDWORD $0 HKLM "SOFTWARE\\Microsoft\\VisualStudio\\14.0\\VC\\Runtimes\\x64" "Installed"')
    lines.append('  ${If} $0 == 1')
    lines.append('    StrCpy $VCRedistInstalled "1"')
    lines.append('    ReadRegStr $VCRedistVersion HKLM "SOFTWARE\\Microsoft\\VisualStudio\\14.0\\VC\\Runtimes\\x64" "Version"')
    lines.append("    Return")
    lines.append("  ${EndIf}")
    lines.append('  ReadRegDWORD $0 HKLM "SOFTWARE\\WOW6432Node\\Microsoft\\VisualStudio\\14.0\\VC\\Runtimes\\x64" "Installed"')
    lines.append('  ${If} $0 == 1')
    lines.append('    StrCpy $VCRedistInstalled "1"')
    lines.append('    ReadRegStr $VCRedistVersion HKLM "SOFTWARE\\WOW6432Node\\Microsoft\\VisualStudio\\14.0\\VC\\Runtimes\\x64" "Version"')
    lines.append("    Return")
    lines.append("  ${EndIf}")
    lines.append("FunctionEnd")
    lines.append("")

    if bundled_vcredist and bundled_vcredist.exists():
        lines.append("Function ExtractBundledVCRedist")
        lines.append("  InitPluginsDir")
        lines.append('  SetOutPath "$PLUGINSDIR"')
        lines.append(f'  File /oname=vcredist_x64.exe "{_escape_nsis_path(str(bundled_vcredist))}"')
        lines.append("FunctionEnd")
        lines.append("")

        lines.append("Function TryInstallVCRedist")
        lines.append('  IfFileExists "$PLUGINSDIR\\vcredist_x64.exe" 0 no_vcredist')
        lines.append('  MessageBox MB_YESNO|MB_ICONQUESTION "Visual C++ 2015-2022 (x64) runtime is not registered.$\\r$\\nInstall or repair it now?" IDYES +2')
        lines.append("  Return")
        lines.append('  ExecWait \'"$PLUGINSDIR\\vcredist_x64.exe" /quiet /norestart\' $0')
        lines.append("  StrCmp $0 0 vcredist_ok")
        lines.append('  MessageBox MB_OK|MB_ICONSTOP "Visual C++ Redistributable installation failed, return code: $0. Installation aborted."')
        lines.append("  Return")
        lines.append("vcredist_ok:")
        lines.append('  MessageBox MB_OK|MB_ICONINFORMATION "Visual C++ runtime install/repair completed. Please re-run setup."')
        lines.append("  Return")
        lines.append("no_vcredist:")
        lines.append('  MessageBox MB_OK|MB_ICONSTOP "Visual C++ runtime is required. Please install Microsoft Visual C++ 2015-2022 (x64) and retry."')
        lines.append("FunctionEnd")
        lines.append("")

    lines.append("Function StopExistingBuckyOS")
    lines.append('  StrCpy $ExistingBuckyRoot ""')
    lines.append('  ReadRegStr $ExistingBuckyRoot HKCU "Environment" "BUCKYOS_ROOT"')
    lines.append('  ${If} $ExistingBuckyRoot == ""')
    lines.append('    ReadRegStr $ExistingBuckyRoot HKCU "Software\\BuckyOS" "InstallDir"')
    lines.append('  ${EndIf}')
    lines.append('  ${If} $ExistingBuckyRoot == ""')
    lines.append('    ; Backward compatibility with previous system-service installations')
    lines.append('    ReadRegStr $ExistingBuckyRoot HKLM "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment" "BUCKYOS_ROOT"')
    lines.append('  ${EndIf}')
    lines.append('  ${If} $ExistingBuckyRoot == ""')
    lines.append('    ReadRegStr $ExistingBuckyRoot HKLM "Software\\BuckyOS" "BuckyOSServiceDir"')
    lines.append('  ${EndIf}')
    lines.append('  nsExec::ExecToLog \'schtasks /Delete /TN "BuckyOSNodeDaemonKeepAlive" /F\'')
    lines.append('  DeleteRegValue HKCU "Software\\Microsoft\\Windows\\CurrentVersion\\Run" "BuckyOSDaemon"')
    lines.append('  nsExec::ExecToLog \'taskkill /F /IM node_daemon.exe\'')
    lines.append('  ${If} $ExistingBuckyRoot != ""')
    lines.append('    ; Backward compatibility: stop/remove legacy Windows service if present')
    lines.append('    nsExec::ExecToLog \'sc stop buckyos\'')
    lines.append("    Sleep 2000")
    lines.append('    nsExec::ExecToLog \'sc delete buckyos\'')
    lines.append('  ${EndIf}')
    lines.append("FunctionEnd")
    lines.append("")

    lines.append("Function SelectBestInstallDrive")
    lines.append('  StrCpy $BestInstallDrive "C:"')
    lines.append('  Delete "$TEMP\\buckyos_best_drive.txt"')
    lines.append(
        '  nsExec::ExecToLog \'powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "$$d = Get-CimInstance Win32_LogicalDisk | Where-Object { $$_.DriveType -eq 3 -and $$_.FreeSpace -ne $$null } | Sort-Object FreeSpace -Descending | Select-Object -First 1 -ExpandProperty DeviceID; if ([string]::IsNullOrWhiteSpace($$d)) { $$d = \'\'C:\'\' }; Set-Content -Path \'\'$$env:TEMP\\\\buckyos_best_drive.txt\'\' -Value $$d -Encoding ASCII"\''
    )
    lines.append('  IfFileExists "$TEMP\\buckyos_best_drive.txt" +2 0')
    lines.append("    Return")
    lines.append('  FileOpen $0 "$TEMP\\buckyos_best_drive.txt" r')
    lines.append("  IfErrors select_best_drive_done_read")
    lines.append("  FileRead $0 $1")
    lines.append("  FileClose $0")
    lines.append("select_best_drive_done_read:")
    lines.append('  Delete "$TEMP\\buckyos_best_drive.txt"')
    lines.append('  StrCpy $1 $1 2')
    lines.append('  ${If} $1 != ""')
    lines.append('    StrCpy $BestInstallDrive $1')
    lines.append('  ${EndIf}')
    lines.append("FunctionEnd")
    lines.append("")

    # .onInit function
    lines.append("; Functions")
    lines.append("Function .onInit")
    
    # Add 64-bit runtime check for x64 installers
    if nsis_arch == "x64":
        lines.append('  ; Check if running on 64-bit Windows')
        lines.append('  ${IfNot} ${RunningX64}')
        lines.append('    MessageBox MB_OK|MB_ICONSTOP "This installer requires 64-bit Windows."')
        lines.append('    Abort')
        lines.append('  ${EndIf}')
        lines.append('  ; Use 64-bit registry view')
        lines.append('  SetRegView 64')
        lines.append("")
    
    # Initialize install directories with default values
    lines.append("  ; Pick the local disk with largest free space and use X:\\buckyos\\ as default")
    lines.append("  Call SelectBestInstallDrive")
    lines.append('  ; Initialize default install directories')
    for comp in components:
        var_name = f"InstDir_{_sanitize_id(comp.key).replace('-', '_')}"
        lines.append(f'  StrCpy ${var_name} "$BestInstallDrive\\buckyos\\"')
    lines.append('  StrCpy $LaunchAfterInstall ""')
    lines.append("")
    
    lines.append('  ; Check Docker Desktop availability for current user context')
    lines.append("  Call CheckDockerDesktopForUser")
    lines.append('  ${If} $DockerCheckCode != "0"')
    lines.append('    MessageBox MB_YESNO|MB_ICONSTOP "Docker Desktop is required and must be usable by the current user.$\\r$\\nExpected: docker CLI and Docker Engine reachable.$\\r$\\nError code: $DockerCheckCode.$\\r$\\n$\\r$\\nOpen Docker Desktop install guide?" IDYES +2')
    lines.append("    Abort")
    lines.append('    ExecShell "open" "https://docs.docker.com/desktop/setup/install/windows-install/"')
    lines.append("    Abort")
    lines.append("  ${EndIf}")
    lines.append("")

    lines.append("  Call CheckRequiredPorts")
    lines.append('  ${If} $PortCheckCode != "0"')
    lines.append('    MessageBox MB_OK|MB_ICONSTOP "Required port $PortCheckPort cannot be bound.$\\r$\\nPlease free ports 3180, 80, and 443 (or stop conflicting services) before installing."')
    lines.append("    Abort")
    lines.append("  ${EndIf}")
    lines.append("")

    lines.append("  Call IsVCRedistInstalled")
    lines.append('  ${If} $VCRedistInstalled != "1"')
    if bundled_vcredist and bundled_vcredist.exists():
        lines.append("    Call ExtractBundledVCRedist")
        lines.append("    Call TryInstallVCRedist")
        lines.append("    Abort")
    else:
        lines.append('    MessageBox MB_YESNO|MB_ICONSTOP "Visual C++ 2015-2022 (x64) runtime is not registered.$\\r$\\nOpen Microsoft download page?" IDYES +2')
        lines.append("    Abort")
        lines.append('    ExecShell "open" "https://aka.ms/vs/17/release/vc_redist.x64.exe"')
        lines.append("    Abort")
    lines.append('  ${EndIf}')
    lines.append("FunctionEnd")
    lines.append("")

    if launch_bundle is not None:
        lines.append("Function LaunchInstalledApplication")
        lines.append('  ${If} $LaunchAfterInstall == ""')
        lines.append("    Return")
        lines.append('  ${EndIf}')
        lines.append('  IfFileExists "$LaunchAfterInstall" +2 0')
        lines.append("    Return")
        lines.append('  ExecShell "open" "$LaunchAfterInstall"')
        lines.append("FunctionEnd")
        lines.append("")
    
    # Sections for each component - all selected by default (no /o flag for optional)
    has_service = False
    has_bundle = False
    for idx, comp in enumerate(components):
        section_id = f"SEC_{_sanitize_id(comp.key).upper()}"
        var_name = f"InstDir_{_sanitize_id(comp.key).replace('-', '_')}"
        
        # All components selected by default, but optional ones can be deselected
        # Using empty flags means selected by default
        lines.append(f'Section "{comp.name}" {section_id}')
        
        # Source files - use component-specific install directory
        comp_payload = payload_dir / comp.key
        if comp_payload.exists():
            if comp.system_service:
                lines.append("  ; Stop existing service and running processes before overwrite")
                lines.append("  Call StopExistingBuckyOS")
            lines.append(f'  SetOutPath "${var_name}"')
            lines.append(f'  File /r "{comp_payload}\\*.*"')
        
        # For bundle type (UI executable) - create shortcuts
        if comp.kind == "bundle":
            has_bundle = True
            # Determine the executable name from src or default
            exe_name = comp.src if comp.src else f"{comp.key}.exe"
            # If src contains path, extract just the filename
            if "/" in exe_name or "\\" in exe_name:
                exe_name = exe_name.replace("/", "\\").split("\\")[-1]
            
            lines.append("")
            lines.append("  ; Create Start Menu shortcut")
            lines.append('  CreateDirectory "$SMPROGRAMS\\BuckyOS"')
            lines.append(f'  CreateShortcut "$SMPROGRAMS\\BuckyOS\\{comp.name}.lnk" "${var_name}\\{exe_name}" "" "${var_name}\\{exe_name}" 0')
            lines.append("")
            lines.append("  ; Create Desktop shortcut")
            lines.append(f'  CreateShortcut "$DESKTOP\\{comp.name}.lnk" "${var_name}\\{exe_name}" "" "${var_name}\\{exe_name}" 0')
            lines.append("")
            lines.append("  ; Launch the UI app from the finish page after installation completes")
            lines.append(f'  StrCpy $LaunchAfterInstall "${var_name}\\{exe_name}"')
        
        # For buckyos service component - set BUCKYOS_ROOT env var
        if comp.system_service:
            has_service = True
            lines.append("")
            lines.append("  ; Set BUCKYOS_ROOT environment variable")
            lines.append(f'  WriteRegStr HKCU "Environment" "BUCKYOS_ROOT" "${var_name}"')
            lines.append(f'  WriteRegStr HKCU "Software\\BuckyOS" "InstallDir" "${var_name}"')
            lines.append('  ; Broadcast environment change')
            lines.append('  SendMessage ${HWND_BROADCAST} ${WM_WININICHANGE} 0 "STR:Environment" /TIMEOUT=5000')
            lines.append("")
            lines.append("  ; Run seed defaults script")
            lines.append(f'  nsExec::ExecToLog \'powershell.exe -ExecutionPolicy Bypass -File "${var_name}\\scripts\\seed_defaults.ps1"\'')
            lines.append("  ; Allow installed service executables through Windows Firewall before first launch")
            lines.append(f'  nsExec::ExecToLog \'powershell.exe -NoProfile -ExecutionPolicy Bypass -File "${var_name}\\scripts\\ensure_firewall_rules.ps1"\'')
            lines.append("")
            lines.append("  ; Create node_daemon keepalive scheduled task (every 1 minute) via hidden VBS wrapper")
            lines.append('  nsExec::ExecToLog \'schtasks /Delete /TN "BuckyOSNodeDaemonKeepAlive" /F\'')
            lines.append(
                f'  nsExec::ExecToLog \'schtasks /Create /TN "BuckyOSNodeDaemonKeepAlive" /SC MINUTE /MO 1 /F /TR "wscript.exe //B //NoLogo $\\"${var_name}\\scripts\\node_daemon_loader.vbs$\\" $\\"${var_name}\\bin\\node-daemon\\node_daemon.exe$\\""\'' 
            )
            lines.append('  nsExec::ExecToLog \'schtasks /Run /TN "BuckyOSNodeDaemonKeepAlive"\'')
            lines.append("")
            lines.append("  ; Register current-user startup via the same hidden wrapper")
            lines.append(
                f'  WriteRegStr HKCU "Software\\Microsoft\\Windows\\CurrentVersion\\Run" "BuckyOSDaemon" \'wscript.exe //B //NoLogo $\\"${var_name}\\scripts\\node_daemon_loader.vbs$\\" $\\"${var_name}\\bin\\node-daemon\\node_daemon.exe$\\"\''
            )
            lines.append("")
            lines.append(f'  ; Save install directory to registry')
            lines.append(f'  WriteRegStr HKCU "Software\\BuckyOS" "BuckyOSUserDir" "${var_name}"')
        
        # Save each component's install directory to registry for uninstall
        lines.append(f'  WriteRegStr HKCU "Software\\BuckyOS" "InstDir_{comp.key}" "${var_name}"')
        
        lines.append("SectionEnd")
        lines.append("")
    
    # Section descriptions
    lines.append("; Section descriptions")
    lines.append("!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN")
    for comp in components:
        section_id = f"SEC_{_sanitize_id(comp.key).upper()}"
        desc = f"Install {comp.name}"
        if comp.optional:
            desc += " (Optional)"
        lines.append(f'  !insertmacro MUI_DESCRIPTION_TEXT ${{{section_id}}} "{desc}"')
    lines.append("!insertmacro MUI_FUNCTION_DESCRIPTION_END")
    lines.append("")
    
    # Post-install section
    lines.append('Section "-PostInstall"')
    lines.append('  ; Write registry keys')
    lines.append('  WriteRegStr HKCU "Software\\BuckyOS" "Version" "${PRODUCT_VERSION}"')
    lines.append("")
    lines.append('  ; Create uninstaller in first component directory')
    first_var = f"InstDir_{_sanitize_id(components[0].key).replace('-', '_')}"
    lines.append(f'  SetOutPath "${first_var}"')
    lines.append(f'  WriteUninstaller "${first_var}\\uninstall.exe"')
    lines.append("")
    lines.append('  ; Add/Remove Programs entry')
    lines.append('  WriteRegStr HKCU "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\BuckyOS" "DisplayName" "${PRODUCT_NAME}"')
    lines.append(f'  WriteRegStr HKCU "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\BuckyOS" "UninstallString" "${first_var}\\uninstall.exe"')
    lines.append('  WriteRegStr HKCU "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\BuckyOS" "DisplayVersion" "${PRODUCT_VERSION}"')
    lines.append('  WriteRegStr HKCU "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\BuckyOS" "Publisher" "${PRODUCT_PUBLISHER}"')
    lines.append('  WriteRegStr HKCU "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\BuckyOS" "URLInfoAbout" "${PRODUCT_WEB_SITE}"')
    lines.append('  WriteRegDWORD HKCU "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\BuckyOS" "NoModify" 1')
    lines.append('  WriteRegDWORD HKCU "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\BuckyOS" "NoRepair" 1')
    lines.append("SectionEnd")
    lines.append("")
    
    # Uninstaller section
    lines.append("Section Uninstall")
    
    # Add 64-bit registry view for uninstaller
    if nsis_arch == "x64":
        lines.append('  SetRegView 64')
    
    lines.append('  ; Stop old service (for backward compatibility) and current-user daemon')
    lines.append('  nsExec::ExecToLog \'sc stop buckyos\'')
    lines.append("  Sleep 3000")
    lines.append('  nsExec::ExecToLog \'sc delete buckyos\'')
    lines.append('  nsExec::ExecToLog \'schtasks /Delete /TN "BuckyOSNodeDaemonKeepAlive" /F\'')
    lines.append('  DeleteRegValue HKCU "Software\\Microsoft\\Windows\\CurrentVersion\\Run" "BuckyOSDaemon"')
    lines.append('  nsExec::ExecToLog \'taskkill /F /IM node_daemon.exe\'')
    lines.append("")
    
    # Remove shortcuts for bundle components
    lines.append('  ; Remove shortcuts')
    for comp in components:
        if comp.kind == "bundle":
            lines.append(f'  Delete "$SMPROGRAMS\\BuckyOS\\{comp.name}.lnk"')
            lines.append(f'  Delete "$DESKTOP\\{comp.name}.lnk"')
    lines.append('  RMDir "$SMPROGRAMS\\BuckyOS"')
    lines.append("")
    
    # Read install directories from registry and remove files
    lines.append('  ; Read install directories from registry and remove files')
    for comp in components:
        var_name = f"InstDir_{_sanitize_id(comp.key).replace('-', '_')}"
        lines.append(f'  ReadRegStr $0 HKCU "Software\\BuckyOS" "InstDir_{comp.key}"')
        lines.append(f'  ${{If}} $0 != ""')
        if comp.system_service:
            lines.append(f'    ; Stop running processes and old service for service component')
            lines.append(f'    nsExec::ExecToLog \'schtasks /Delete /TN "BuckyOSNodeDaemonKeepAlive" /F\'')
            lines.append(f'    DeleteRegValue HKCU "Software\\Microsoft\\Windows\\CurrentVersion\\Run" "BuckyOSDaemon"')
            lines.append(f'    nsExec::ExecToLog \'taskkill /F /IM node_daemon.exe\'')
            lines.append(f'    nsExec::ExecToLog \'sc stop buckyos\'')
            lines.append("    Sleep 3000")
            lines.append(f'    nsExec::ExecToLog \'sc delete buckyos\'')
            lines.append(f'    ; Run cleanup script for service component')
            lines.append(f'    nsExec::ExecToLog \'powershell.exe -ExecutionPolicy Bypass -File "$0\\scripts\\uninstall_cleanup.ps1"\'')
            lines.append(f'    RMDir /r "$0\\.buckyos_installer_defaults"')
            lines.append(f'    RMDir /r "$0\\scripts"')
            lines.append('    MessageBox MB_YESNO|MB_ICONQUESTION "Do you want to delete your data and identity?" IDYES +2')
            lines.append("    Goto +2")
            lines.append(f'    RMDir /r "$0"')
        else:
            lines.append(f'    RMDir /r "$0"')
        lines.append(f'  ${{EndIf}}')
        lines.append("")
    
    lines.append('  ; Remove registry keys')
    lines.append('  DeleteRegKey HKCU "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\BuckyOS"')
    lines.append('  DeleteRegKey HKCU "Software\\BuckyOS"')
    lines.append('  DeleteRegValue HKCU "Environment" "BUCKYOS_ROOT"')
    lines.append('  ; Broadcast environment change')
    lines.append('  SendMessage ${HWND_BROADCAST} ${WM_WININICHANGE} 0 "STR:Environment" /TIMEOUT=5000')
    lines.append("SectionEnd")
    
    # Write with UTF-8 BOM for NSIS
    content = "\r\n".join(lines) + "\r\n"
    out_path.write_text(content, encoding="utf-8-sig")


def _print_file_tree(path: Path, prefix: str = "", is_last: bool = True) -> None:
    """Print a file tree for dry-run output."""
    connector = "└── " if is_last else "├── "
    print(f"{prefix}{connector}{path.name}")
    
    if path.is_dir():
        children = [child for child in path.iterdir() if child.name not in IGNORED_STAGE_NAMES]
        children.sort(key=lambda x: (not x.is_dir(), x.name.lower()))
        for i, child in enumerate(children[:20]):  # Limit to 20 items
            extension = "    " if is_last else "│   "
            _print_file_tree(child, prefix + extension, i == len(children) - 1)
        if len(children) > 20:
            extension = "    " if is_last else "│   "
            print(f"{prefix}{extension}... and {len(children) - 20} more items")


def build_win_installer(
    *,
    architecture: str,
    version: str,
    project_yaml_path: Path,
    manifest_path: Path | None,
    app_publish_dir: Path,
    out_dir: Path,
    dry_run: bool = False,
) -> Path:
    """Build the Windows NSIS installer."""

    components = (
        load_win_pkg_components_from_manifest(manifest_path)
        if manifest_path is not None
        else load_win_pkg_components(project_yaml_path)
    )
    
    work_dir = TMP_INSTALL_DIR / "distbuild"
    payload_dir = work_dir / "payload"
    nsi_file = work_dir / "installer.nsi"
    
    # Keep scripts in sync with bucky_project.yaml before building
    if not dry_run and not bool(os.environ.get("BUCKYOS_PKG_NO_SYNC_SCRIPTS")):
        sync_win_scripts(project_yaml_path, WIN_PKG_PROJECT_DIR / "scripts", manifest_path=manifest_path)
    
    if work_dir.exists() and not dry_run:
        shutil.rmtree(work_dir, ignore_errors=True)
    if not dry_run:
        work_dir.mkdir(parents=True, exist_ok=True)
        payload_dir.mkdir(parents=True, exist_ok=True)
    
    print(f"[build] Staging components to {payload_dir}")
    
    for comp in components:
        src = _resolve_component_src(comp, app_publish_dir)
        
        # Try fallback paths
        if not src.exists():
            alt = app_publish_dir / "publish" / comp.key
            if alt.exists():
                src = alt
        
        if dry_run:
            print(f"\n[dry-run] Component: {comp.name} ({comp.key})")
            print(f"  Type: {comp.kind}")
            print(f"  Optional: {comp.optional}")
            print(f"  System Service: {comp.system_service}")
            print(f"  Source: {src}")
            print(f"  Target: {comp.default_target}")
            if src.exists():
                print(f"  Source exists: YES")
                if src.is_dir():
                    print("  Files:")
                    _print_file_tree(src, "    ")
            else:
                print(f"  Source exists: NO (will fail during actual build)")
            continue
        
        if not src.exists():
            raise FileNotFoundError(f"component source not found: {comp.key} -> {src}")
        
        comp_payload = payload_dir / comp.key
        comp_payload.mkdir(parents=True, exist_ok=True)
        
        if comp.key == "buckyos":
            # Special staging for buckyos with data_paths semantics
            layout = resolve_app_layout(
                app_key="buckyos",
                project_yaml_path=project_yaml_path,
                manifest_path=manifest_path,
                target_override="C:\\opt\\buckyos",
            )
            _stage_buckyos_app_root(src_root=src, dst_root=comp_payload, layout=layout)
            
            # Copy scripts to payload
            scripts_src = WIN_PKG_PROJECT_DIR / "scripts"
            scripts_dst = comp_payload / "scripts"
            if scripts_src.exists():
                _copytree_filtered(scripts_src, scripts_dst)
        else:
            if src.is_dir():
                _copy_dir_contents(src, comp_payload)
            else:
                shutil.copy2(src, comp_payload / src.name)

    if dry_run:
        print(f"\n[dry-run] Would generate NSIS script: {nsi_file}")
        print(f"[dry-run] Would compile installer to: {out_dir / f'buckyos-win-{architecture}-{version}.exe'}")
        return out_dir / f"buckyos-win-{architecture}-{version}.exe"
    
    # Generate NSIS script
    license_file = WIN_PKG_PROJECT_DIR / "license.txt"
    generate_nsis_script(
        title="BuckyOS",
        version=version,
        architecture=architecture,
        components=components,
        payload_dir=payload_dir,
        out_path=nsi_file,
        license_file=license_file if license_file.exists() else None,
        bundled_vcredist=(WIN_PKG_PROJECT_DIR / "vcredist_x64.exe"),
    )
    print(f"[build] Generated NSIS script: {nsi_file}")
    
    # Compile with NSIS
    out_dir.mkdir(parents=True, exist_ok=True)
    
    # Try to find makensis
    makensis_paths = [
        "makensis",
        "C:\\Program Files (x86)\\NSIS\\makensis.exe",
        "C:\\Program Files\\NSIS\\makensis.exe",
    ]
    
    makensis_cmd = None
    for path in makensis_paths:
        try:
            result = subprocess.run([path, "/VERSION"], capture_output=True)
            if result.returncode == 0:
                makensis_cmd = path
                break
        except FileNotFoundError:
            continue
    
    if not makensis_cmd:
        raise RuntimeError(
            "makensis not found. Please install NSIS from https://nsis.sourceforge.io/ "
            "and ensure it's in your PATH."
        )
    
    cmd = [makensis_cmd, "/V3", str(nsi_file)]
    print(f"[build] Compiling installer with NSIS...")
    rc = _run(cmd, dry_run=False, cwd=work_dir)
    
    if rc != 0:
        raise RuntimeError(f"NSIS compilation failed with code {rc}")
    
    # Move output to target directory
    built_exe = work_dir / f"buckyos-win-{architecture}-{version}.exe"
    out_exe = out_dir / f"buckyos-win-{architecture}-{version}.exe"
    
    if built_exe.exists():
        shutil.move(str(built_exe), str(out_exe))
        print(f"[build] Installer created: {out_exe}")
        return out_exe
    else:
        raise RuntimeError(f"Expected output not found: {built_exe}")


def verify_pkg(
    *,
    pkg_path: Path,
    project_yaml_path: Path,
    manifest_path: Path | None = None,
) -> int:
    """
    Verify a built Windows installer using 7zip.

    Checks:
    - File exists and is valid archive
    - Expected components are present
    - Metadata matches YAML configuration
    """
    if not pkg_path.exists():
        print(f"VERIFY FAIL: Installer not found: {pkg_path}")
        return 1
    
    components = (
        load_win_pkg_components_from_manifest(manifest_path)
        if manifest_path is not None
        else load_win_pkg_components(project_yaml_path)
    )
    failures: List[str] = []
    
    # Try to find 7z
    sz_paths = [
        "7z",
        "C:\\Program Files\\7-Zip\\7z.exe",
        "C:\\Program Files (x86)\\7-Zip\\7z.exe",
    ]
    
    sz_cmd = None
    for path in sz_paths:
        try:
            result = subprocess.run([path], capture_output=True)
            sz_cmd = path
            break
        except FileNotFoundError:
            continue
    
    if not sz_cmd:
        print("[verify] Warning: 7-Zip not found, skipping archive inspection")
        print("[verify] Install 7-Zip from https://www.7-zip.org/ for full verification")
    else:
        with tempfile.TemporaryDirectory(prefix="buckyos-verify-") as td:
            work = Path(td)
            extract_dir = work / "extracted"
            
            # Extract installer
            cmd = [sz_cmd, "x", f"-o{extract_dir}", str(pkg_path), "-y"]
            print(f"[verify] Extracting installer...")
            result = subprocess.run(cmd, capture_output=True)
            
            if result.returncode != 0:
                failures.append(f"Failed to extract installer: {result.stderr.decode()}")
            else:
                extracted_basenames = {
                    path.name.lower()
                    for path in extract_dir.rglob("*")
                    if path.is_file()
                }

                # NSIS extraction does not preserve our logical component directories,
                # so verify by payload marker files instead of folder names.
                for comp in components:
                    expected_names = _component_expected_filenames(comp=comp, manifest_path=manifest_path)
                    if expected_names and not any(name in extracted_basenames for name in expected_names):
                        failures.append(
                            f"Component payload not detected: {comp.key} "
                            f"(expected files like {', '.join(expected_names[:5])})"
                        )

                for script in (
                    "seed_defaults.ps1",
                    "ensure_firewall_rules.ps1",
                    "uninstall_cleanup.ps1",
                    "node_daemon_loader.ps1",
                    "node_daemon_loader.vbs",
                ):
                    if script.lower() not in extracted_basenames:
                        failures.append(f"Missing required script: {script}")
    
    # Verify file size is reasonable
    file_size = pkg_path.stat().st_size
    if file_size < 1024 * 1024:  # Less than 1MB is suspicious
        failures.append(f"Installer size suspiciously small: {file_size} bytes")
    
    print(f"[verify] Installer size: {file_size / (1024*1024):.2f} MB")
    
    if failures:
        print("VERIFY FAIL:")
        for f in failures:
            print(f"  - {f}")
        return 1
    
    print("VERIFY PASS")
    return 0


def _ps_rm_lines(root_var: str, rel_paths: List[str]) -> List[str]:
    """Generate PowerShell Remove-Item lines."""
    out: List[str] = []
    for rel in rel_paths:
        rel_s = rel.strip().lstrip("/\\").rstrip("/\\")
        if not rel_s:
            continue
        # Normalize path separators for Windows
        rel_s = rel_s.replace("/", "\\")
        out.append(f'Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -Path (Join-Path {root_var} "{rel_s}")')
    return out


def _ps_data_copy_lines(root_var: str, defaults_var: str, rel_paths: List[str]) -> List[str]:
    """Generate PowerShell data copy lines for seed_defaults.ps1."""
    out: List[str] = []
    for rel in rel_paths:
        rel_s = rel.strip().lstrip("/\\")
        if not rel_s:
            continue
        # Normalize path separators
        rel_s = rel_s.replace("/", "\\")
        
        if rel_s.endswith("/") or rel_s.endswith("\\"):
            rel_s = rel_s.rstrip("/\\")
            # Directory copy
            out += [
                f'$src = Join-Path {defaults_var} "{rel_s}"',
                f'$dst = Join-Path {root_var} "{rel_s}"',
                "$shouldCopy = $false",
                'if (-not (Test-Path $dst)) { $shouldCopy = $true }',
                'elseif (-not (Get-ChildItem -LiteralPath $dst -Force -ErrorAction SilentlyContinue | Select-Object -First 1)) { $shouldCopy = $true }',
                'if ($shouldCopy -and (Test-Path $src)) {',
                '  New-Item -ItemType Directory -Force -Path $dst | Out-Null',
                '  Copy-Item -Recurse -Force -Path (Join-Path $src "*") -Destination $dst',
                '}',
            ]
        else:
            # File copy
            out += [
                f'$src = Join-Path {defaults_var} "{rel_s}"',
                f'$dst = Join-Path {root_var} "{rel_s}"',
                'if (-not (Test-Path $dst)) {',
                '  if (Test-Path $src) {',
                '    New-Item -ItemType Directory -Force -Path (Split-Path $dst -Parent) | Out-Null',
                '    Copy-Item -Force -Path $src -Destination $dst',
                '  }',
                '}',
            ]
    return out


def _render_seed_defaults_script(data_paths: List[str]) -> str:
    """Render the full seed_defaults.ps1 script content."""
    lines: List[str] = [
        '$ErrorActionPreference = "Stop"',
        "",
        "# AUTO-GENERATED by make_local_win_installer.py (sync_win_scripts)",
        "# Do not edit this file manually.",
        "",
        "$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path",
        "$Root = Split-Path -Parent $ScriptDir",
        '$DefaultsDir = Join-Path $Root ".buckyos_installer_defaults"',
        "",
    ]
    lines.extend(_ps_data_copy_lines("$Root", "$DefaultsDir", data_paths))
    return "\n".join(lines).rstrip("\n") + "\n"


def _render_firewall_setup_script() -> str:
    """Render the Windows Firewall setup script for installed service executables."""
    lines: List[str] = [
        '$ErrorActionPreference = "Continue"',
        "",
        "# AUTO-GENERATED by make_local_win_installer.py (sync_win_scripts)",
        "# Do not edit this file manually.",
        "",
        "$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path",
        "$Root = Split-Path -Parent $ScriptDir",
        '$BinDir = Join-Path $Root "bin"',
        '$RulePrefix = "BuckyOS Service"',
        "",
        "function Get-RuleName {",
        "  param([string]$ProgramPath)",
        "  $relative = $ProgramPath.Substring($Root.Length).TrimStart('\\\\')",
        '  return "$RulePrefix - $relative"',
        "}",
        "",
        "if (-not (Test-Path $BinDir)) {",
        "  exit 0",
        "}",
        "",
        '$hasNetSecurity = $null -ne (Get-Command New-NetFirewallRule -ErrorAction SilentlyContinue)',
        "$programs = Get-ChildItem -Path $BinDir -Recurse -File -Filter *.exe -ErrorAction SilentlyContinue | Sort-Object FullName",
        "foreach ($program in $programs) {",
        "  $programPath = $program.FullName",
        "  $ruleName = Get-RuleName -ProgramPath $programPath",
        "  if ($hasNetSecurity) {",
        "    Get-NetFirewallRule -DisplayName $ruleName -ErrorAction SilentlyContinue | Remove-NetFirewallRule -ErrorAction SilentlyContinue | Out-Null",
        '    New-NetFirewallRule -DisplayName $ruleName -Group "BuckyOS" -Direction Inbound -Action Allow -Program $programPath -Profile Any | Out-Null',
        "    continue",
        "  }",
        '  & netsh advfirewall firewall delete rule name="$ruleName" program="$programPath" > $null 2>&1',
        '  & netsh advfirewall firewall add rule name="$ruleName" dir=in action=allow program="$programPath" enable=yes profile=any > $null',
        "}",
        "",
    ]
    return "\n".join(lines).rstrip("\n") + "\n"


def _render_uninstall_cleanup_script(module_paths: List[str], clean_paths: List[str]) -> str:
    """Render the full uninstall_cleanup.ps1 script content."""
    lines: List[str] = [
        '$ErrorActionPreference = "Continue"',
        "",
        "# AUTO-GENERATED by make_local_win_installer.py (sync_win_scripts)",
        "# Do not edit this file manually.",
        "",
        "$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path",
        "$Root = Split-Path -Parent $ScriptDir",
        '$DefaultsDir = Join-Path $Root ".buckyos_installer_defaults"',
        '$BinDir = Join-Path $Root "bin"',
        '$RulePrefix = "BuckyOS Service"',
        "",
        "Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -Path $DefaultsDir",
        "",
        "function Get-RuleName {",
        "  param([string]$ProgramPath)",
        "  $relative = $ProgramPath.Substring($Root.Length).TrimStart('\\\\')",
        '  return "$RulePrefix - $relative"',
        "}",
        "",
        "if (Test-Path $BinDir) {",
        '  $hasNetSecurity = $null -ne (Get-Command Get-NetFirewallRule -ErrorAction SilentlyContinue)',
        "  $programs = Get-ChildItem -Path $BinDir -Recurse -File -Filter *.exe -ErrorAction SilentlyContinue | Sort-Object FullName",
        "  foreach ($program in $programs) {",
        "    $programPath = $program.FullName",
        "    $ruleName = Get-RuleName -ProgramPath $programPath",
        "    if ($hasNetSecurity) {",
        "      Get-NetFirewallRule -DisplayName $ruleName -ErrorAction SilentlyContinue | Remove-NetFirewallRule -ErrorAction SilentlyContinue | Out-Null",
        "      continue",
        "    }",
        '    & netsh advfirewall firewall delete rule name="$ruleName" program="$programPath" > $null 2>&1',
        "  }",
        "}",
    ]
    clean_lines = _ps_rm_lines("$Root", clean_paths)
    module_lines = _ps_rm_lines("$Root", module_paths)
    if clean_lines:
        lines.extend(["", *clean_lines])
    if module_lines:
        lines.extend(["", *module_lines])
    return "\n".join(lines).rstrip("\n") + "\n"


def sync_win_scripts(
    project_yaml_path: Path,
    scripts_dir: Path,
    *,
    manifest_path: Path | None = None,
) -> None:
    """
    Update PowerShell scripts based on bucky_project.yaml.

    - seed_defaults.ps1: full file is auto-generated.
    - ensure_firewall_rules.ps1: full file is auto-generated.
    - uninstall_cleanup.ps1: full file is auto-generated.
    
    Note: Currently only syncs buckyos app scripts, as the PowerShell
    scripts are specifically designed for the buckyos component.
    """
    # Ensure scripts directory exists
    scripts_dir.mkdir(parents=True, exist_ok=True)
    
    # Only process buckyos app layout for the main scripts
    layout = resolve_app_layout(
        app_key="buckyos",
        project_yaml_path=project_yaml_path,
        manifest_path=manifest_path,
    )
    
    # Re-generate full seed_defaults.ps1
    seed_script = scripts_dir / "seed_defaults.ps1"
    seed_script.write_text(
        _render_seed_defaults_script(layout.data_paths),
        encoding="utf-8-sig",
    )
    print(f"[sync] Updated: {seed_script}")

    firewall_script = scripts_dir / "ensure_firewall_rules.ps1"
    firewall_script.write_text(
        _render_firewall_setup_script(),
        encoding="utf-8-sig",
    )
    print(f"[sync] Updated: {firewall_script}")
    
    # Re-generate full uninstall_cleanup.ps1
    uninstall_script = scripts_dir / "uninstall_cleanup.ps1"
    uninstall_script.write_text(
        _render_uninstall_cleanup_script(layout.module_paths, layout.clean_paths),
        encoding="utf-8-sig",
    )
    print(f"[sync] Updated: {uninstall_script}")


def main(argv: List[str]) -> int:
    parser = argparse.ArgumentParser(
        prog="make_local_win_installer.py",
        description="Build Windows NSIS installer for BuckyOS"
    )
    sub = parser.add_subparsers(dest="cmd", required=True)
    
    # build-pkg command
    p_build = sub.add_parser("build-pkg", help="Build Windows .exe installer using NSIS")
    p_build.add_argument("architecture", help="amd64|arm64")
    p_build.add_argument("version", help="Version string (e.g., 0.5.1+build260114)")
    p_build.add_argument("--project", default=str(PROJECT_YAML), help="Path to bucky_project.yaml")
    p_build.add_argument("--manifest", default=None, help="Path to generated project manifest JSON")
    p_build.add_argument(
        "--app-publish-dir",
        default=str(RESULT_ROOT_DIR),
        help="Base directory to resolve publish.win_pkg.apps sources (default: $BUCKYOS_BUILD_ROOT)"
    )
    p_build.add_argument(
        "--out-dir",
        default=str(Path.cwd() / "publish"),
        help='Output directory for the final .exe (default: "./publish")'
    )
    p_build.add_argument(
        "--no-sync-scripts",
        action="store_true",
        help="Do not auto-sync win_pkg/scripts from bucky_project.yaml before build"
    )
    p_build.add_argument("--dry-run", action="store_true", help="Preview build without executing NSIS")
    
    # verify command
    p_verify = sub.add_parser("verify-pkg", aliases=["verify"], help="Verify a built installer using 7zip")
    p_verify.add_argument("--pkg", required=True, help="Path to .exe installer")
    p_verify.add_argument("--project", default=str(PROJECT_YAML), help="Path to bucky_project.yaml")
    p_verify.add_argument("--manifest", default=None, help="Path to generated project manifest JSON")

    # sync command
    p_sync = sub.add_parser("sync", help="Regenerate PowerShell scripts from bucky_project.yaml")
    p_sync.add_argument("--project", default=str(PROJECT_YAML), help="Path to bucky_project.yaml")
    p_sync.add_argument("--manifest", default=None, help="Path to generated project manifest JSON")
    
    args = parser.parse_args(argv[1:])
    
    if args.cmd == "build-pkg":
        arch = args.architecture
        if arch == "x86_64":
            arch = "amd64"
        if args.no_sync_scripts:
            os.environ["BUCKYOS_PKG_NO_SYNC_SCRIPTS"] = "1"
        
        out_exe = build_win_installer(
            architecture=arch,
            version=args.version,
            project_yaml_path=Path(args.project),
            manifest_path=Path(args.manifest).resolve() if args.manifest else None,
            app_publish_dir=Path(args.app_publish_dir),
            out_dir=Path(args.out_dir),
            dry_run=bool(args.dry_run),
        )
        print(f"Installer built: {out_exe}")
        return 0
    
    if args.cmd in ("verify", "verify-pkg"):
        return verify_pkg(
            pkg_path=Path(args.pkg).expanduser().resolve(),
            project_yaml_path=Path(args.project),
            manifest_path=Path(args.manifest).resolve() if args.manifest else None,
        )

    if args.cmd == "sync":
        sync_win_scripts(
            Path(args.project),
            WIN_PKG_PROJECT_DIR / "scripts",
            manifest_path=Path(args.manifest).resolve() if args.manifest else None,
        )
        print("win_pkg scripts synced.")
        return 0
    
    return 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
