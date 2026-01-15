"""
Local macOS pkg builder + local installer helper.

This script supports:
- build-pkg: build a macOS .pkg (distribution with component choices)
- install:   clean + install_data + update (fresh install)
- update:    update only (overwrite modules, keep existing data_paths)
- uninstall: remove module paths + clean_paths

It reads:
- `apps.buckyos.*` for local install/update/uninstall on a directory.
- `publish.macos_pkg.apps.*` for macOS distribution package components.

Before make pkg,make sure already build the latest buckyos-app, buckycli and buckyos.
- buckyos-build && buckyos-install --all --target-rootfs=/opt/buckyosci/buckyos && python3 make_config.py --rootfs /opt/buckyosci/buckyos release
- copy BuckyOS.app to /opt/buckyosci/BuckyOSApp/BuckyOS.app
"""

from __future__ import annotations

import argparse
import json
import os
import platform
import shutil
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Tuple

import re

try:
    import yaml  # type: ignore
except ImportError as e:
    raise ImportError(
        "PyYAML is required. Use your project venv (e.g. `./venv/bin/python ...`), "
        "or install buckyos-devkit (recommended) / `pip install pyyaml`."
    ) from e


SRC_DIR = Path(__file__).resolve().parent.parent
PROJECT_YAML = SRC_DIR / "bucky_project.yaml"

RESULT_ROOT_DIR = Path(os.environ.get("BUCKYOS_BUILD_ROOT", "/opt/buckyosci"))
TMP_INSTALL_DIR = RESULT_ROOT_DIR / "macos-pkg"

MACOS_PKG_PROJECT_DIR = SRC_DIR / "publish" / "macos_pkg"
BUCKYOS_DEFAULTS_SUBDIR = ".buckyos_installer_defaults"


def yaml_load_file(path: Path) -> Dict[str, Any]:
    data = yaml.safe_load(path.read_text(encoding="utf-8"))
    if data is None:
        return {}
    if not isinstance(data, dict):
        raise ValueError(f"YAML root must be a map: {path}")
    return data


def _expand_vars(s: str) -> str:
    # Expand ${VAR} and ${VAR:-default} very lightly; enough for ${BUCKYOS_ROOT}.
    out = s
    for name, default in [("BUCKYOS_ROOT", "/opt/buckyos"), ("BUCKYOS_BUILD_ROOT", str(RESULT_ROOT_DIR))]:
        val = os.environ.get(name, default)
        out = out.replace(f"${{{name}}}", val)
    return os.path.expanduser(out)


@dataclass(frozen=True)
class AppLayout:
    source_rootfs: Path
    target_rootfs: Path
    module_paths: List[str]
    data_paths: List[str]
    clean_paths: List[str]


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


def load_buckyos_layout(project_yaml_path: Path = PROJECT_YAML, target_override: str | None = None) -> AppLayout:
    # Backward compatibility wrapper
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


def load_macos_pkg_components(project_yaml_path: Path) -> List[PublishComponent]:
    data = yaml_load_file(project_yaml_path)
    publish = data.get("publish", {}) or {}
    macos_pkg = publish.get("macos_pkg", {}) or {}
    apps = macos_pkg.get("apps", {}) or {}
    if not isinstance(apps, dict):
        raise ValueError("publish.macos_pkg.apps must be a map")

    components: List[PublishComponent] = []
    for key, cfg in apps.items():
        if not isinstance(cfg, dict):
            raise ValueError(f"publish.macos_pkg.apps.{key} must be a map")
        name = _as_str(cfg.get("name", key)).strip() or key
        kind = _as_str(cfg.get("type", "")).strip() or "app"
        optional = bool(cfg.get("optional", False))
        src = _as_str(cfg.get("src", "")).strip() or None
        default_target = _as_str(cfg.get("default_target", "")).strip()
        if not default_target:
            raise ValueError(f"publish.macos_pkg.apps.{key} missing default_target")
        components.append(
            PublishComponent(
                key=_as_str(key),
                name=name,
                kind=kind,
                optional=optional,
                src=src,
                default_target=default_target,
            )
        )
    return components


def _resolve_component_src(component: PublishComponent, app_publish_dir: Path) -> Path:
    # Resolution rules:
    # - absolute "src" -> use directly
    # - relative "src" -> {app_publish_dir}/{component.key}/{src}
    # - no "src"      -> {app_publish_dir}/{component.key}
    if component.src:
        p = Path(component.src)
        if p.is_absolute():
            return p
        return app_publish_dir / component.key / component.src
    return app_publish_dir / component.key


def _resolve_component_target(component: PublishComponent) -> Path:
    # NOTE: "~" will be expanded at build time, which is machine/user-specific.
    # Prefer absolute paths in bucky_project.yaml for reproducible packages.
    return Path(_expand_vars(component.default_target))


def _copy_dir_contents(src_dir: Path, dst_dir: Path) -> None:
    dst_dir.mkdir(parents=True, exist_ok=True)
    for child in src_dir.iterdir():
        dst = dst_dir / child.name
        if child.is_dir():
            shutil.copytree(child, dst, dirs_exist_ok=True)
        else:
            dst.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(child, dst)


def _stage_buckyos_app_root(*, src_root: Path, dst_root: Path, layout: AppLayout) -> None:
    """
    Stage buckyos rootfs into dst_root.

    Semantics:
    - modules: always copied into real target paths (will be overwritten by pkg install)
    - data_paths: copied into `${BUCKYOS_ROOT}/.buckyos_installer_defaults/...`
      and postinstall will copy to real paths only if missing (overwrite install behavior)
    """
    # modules -> real target
    for rel in layout.module_paths:
        rel_s = rel.strip()
        if rel_s.startswith("/"):
            rel_s = rel_s[1:]
        rel_s = rel_s.rstrip("/")
        s = src_root / rel_s
        d = dst_root / rel_s
        if s.is_dir():
            shutil.copytree(s, d, dirs_exist_ok=True)
        elif s.exists():
            d.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(s, d)

    # data_paths -> defaults area
    defaults_root = dst_root / BUCKYOS_DEFAULTS_SUBDIR
    for rel in layout.data_paths:
        rel_s = rel.strip()
        if rel_s.startswith("/"):
            rel_s = rel_s[1:]
        rel_s = rel_s.rstrip("/")
        s = src_root / rel_s
        d = defaults_root / rel_s
        if not s.exists():
            raise FileNotFoundError(
                f"data_paths source missing: '{rel}' -> '{s}'. "
                f"Please ensure it exists under the buckyos publish root ({src_root}), "
                "or remove it from apps.buckyos.data_paths."
            )
        if s.is_dir():
            shutil.copytree(s, d, dirs_exist_ok=True)
        else:
            d.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(s, d)

def _run(cmd: List[str], dry_run: bool) -> None:
    print("+", " ".join(cmd))
    if dry_run:
        return
    subprocess.run(cmd, check=True)


def _xml_escape(s: str) -> str:
    return (
        s.replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
        .replace('"', "&quot;")
        .replace("'", "&apos;")
    )


def generate_welcome_html(components: Iterable["PublishComponent"], out_path: Path) -> None:
    rows = []
    for c in components:
        rows.append(
            "<tr>"
            f"<td><b>{_xml_escape(c.name)}</b></td>"
            f"<td><code>{_xml_escape(c.default_target)}</code></td>"
            f"<td>{'Optional' if c.optional else 'Required'}</td>"
            "</tr>"
        )

    html = """<!doctype html>
<html>
  <head>
    <meta charset="utf-8" />
    <style>
      body { font-family: -apple-system, system-ui, sans-serif; font-size: 13px; }
      table { border-collapse: collapse; width: 100%%; }
      th, td { border: 1px solid #ddd; padding: 8px; vertical-align: top; }
      th { background: #f6f6f6; text-align: left; }
      code { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; }
    </style>
  </head>
  <body>
    <p>This installer contains multiple components. Install locations are shown below.</p>
    <p><b>Note:</b> <code>~</code> means the current user's home directory.</p>
    <table>
      <thead>
        <tr><th>Component</th><th>Install location</th><th>Required</th></tr>
      </thead>
      <tbody>
        %s
      </tbody>
    </table>
  </body>
</html>
""" % ("\n        ".join(rows))
    out_path.write_text(html + "\n", encoding="utf-8")


def _write_distribution_xml(
    *,
    title: str,
    version: str,
    components: List[Tuple[PublishComponent, str, str]],
    resources_dir: Path,
    out_path: Path,
) -> None:
    # components: (component, pkg_identifier, pkg_filename)
    # Build a simple Distribution XML with a customization screen.
    lines: List[str] = []
    lines.append('<?xml version="1.0" encoding="utf-8"?>')
    lines.append('<installer-gui-script minSpecVersion="1">')
    lines.append(f"  <title>{_xml_escape(title)}</title>")
    lines.append('  <options customize="always" />')

    # Optional HTML screens (if present in resources).
    for tag, filename in (("welcome", "welcome.html"), ("license", "license.html"), ("conclusion", "conclusion.html")):
        if (resources_dir / filename).exists():
            lines.append(f'  <{tag} file="{filename}" />')

    lines.append("  <choices-outline>")
    # Required first, then optional.
    ordered = sorted(components, key=lambda x: (x[0].optional, x[0].key.lower()))
    for comp, _, _ in ordered:
        choice_id = f"choice.{_sanitize_id(comp.key)}"
        lines.append(f'    <line choice="{choice_id}" />')
    lines.append("  </choices-outline>")

    for comp, pkg_id, _pkg_filename in ordered:
        choice_id = f"choice.{_sanitize_id(comp.key)}"
        # In practice, `required="true"` alone may still allow the checkbox to be toggled
        # in some Installer versions. We lock required choices by also setting enabled="false".
        required_attr = ' required="true" enabled="false"' if not comp.optional else ' required="false" enabled="true"'
        # Optional components are selected by default but can be deselected.
        start_selected_attr = ' start_selected="true"'
        desc = f"Will be installed to: {comp.default_target}"
        desc_attr = f' description="{_xml_escape(desc)}" description-mime-type="text/plain"'
        lines.append(
            f'  <choice id="{choice_id}" title="{_xml_escape(comp.name)}"{desc_attr}{start_selected_attr}{required_attr}>'
        )
        lines.append(f'    <pkg-ref id="{pkg_id}" />')
        lines.append("  </choice>")

    for comp, pkg_id, pkg_filename in ordered:
        _ = comp
        lines.append(f'  <pkg-ref id="{pkg_id}" version="{version}">{pkg_filename}</pkg-ref>')

    lines.append("</installer-gui-script>")
    out_path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def build_macos_distribution_pkg(
    *,
    architecture: str,
    version: str,
    project_yaml_path: Path,
    app_publish_dir: Path,
    out_dir: Path,
    extra_bundles: Optional[List[Path]] = None,
    dry_run: bool = False,
) -> Path:
    components = load_macos_pkg_components(project_yaml_path)

    # Inject extra bundle apps (not defined in bucky_project.yaml).
    extras: List[PublishComponent] = []
    for p in (extra_bundles or []):
        p = p.expanduser().resolve()
        if p.suffix != ".app":
            raise ValueError(f"extra bundle must be a .app directory: {p}")
        extras.append(
            PublishComponent(
                key=f"bundle-{p.stem}",
                name=p.stem,
                kind="bundle",
                optional=True,
                src=str(p),
                default_target=str(Path("/Applications") / p.name),
            )
        )

    components = components + extras

    work_dir = TMP_INSTALL_DIR / "distbuild"
    roots_dir = work_dir / "roots"
    pkgs_dir = work_dir / "pkgs"
    resources_dir = MACOS_PKG_PROJECT_DIR
    dist_xml = work_dir / "distribution.xml"

    # Keep project scripts in sync with bucky_project.yaml before building.
    # This updates only marked AUTO-GENERATED blocks in existing scripts.
    if (not dry_run) and (not bool(os.environ.get("BUCKYOS_PKG_NO_SYNC_SCRIPTS"))):
        sync_macos_scripts(project_yaml_path, resources_dir / "scripts")

    if work_dir.exists() and not dry_run:
        shutil.rmtree(work_dir, ignore_errors=True)
    if not dry_run:
        roots_dir.mkdir(parents=True, exist_ok=True)
        pkgs_dir.mkdir(parents=True, exist_ok=True)

    base_identifier = "com.github.buckyos.pkg"

    built: List[Tuple[PublishComponent, str, str]] = []
    for comp in components:
        src = _resolve_component_src(comp, app_publish_dir)
        if not src.exists():
            # fallback: some builds may place outputs under {build_root}/publish/{key}
            alt = app_publish_dir / "publish" / comp.key
            if alt.exists():
                src = alt
        if not src.exists():
            raise FileNotFoundError(f"component source not found: {comp.key} -> {src}")

        target = _resolve_component_target(comp)
        target_rel = str(target).lstrip("/")
        component_root = roots_dir / comp.key
        if dry_run:
            print(f"[dry-run] stage component '{comp.key}': {src} -> {target}")
        else:
            if component_root.exists():
                shutil.rmtree(component_root, ignore_errors=True)
            component_root.mkdir(parents=True, exist_ok=True)

            dst = component_root / target_rel
            if comp.key == "buckyos":
                # Special staging to honor data_paths overwrite-install semantics.
                layout = load_buckyos_layout(project_yaml_path, target_override="/opt/buckyos")
                _stage_buckyos_app_root(src_root=src, dst_root=dst, layout=layout)
            else:
                if src.is_dir():
                    if str(target).endswith(".app"):
                        shutil.copytree(src, dst, dirs_exist_ok=True)
                    else:
                        _copy_dir_contents(src, dst)
                else:
                    # File payload
                    if str(target).endswith("/"):
                        dst = component_root / target_rel / src.name
                    dst.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(src, dst)

        pkg_id = f"{base_identifier}.{_sanitize_id(comp.key)}"
        pkg_filename = f"{_sanitize_id(comp.key)}.pkg"
        pkg_path = pkgs_dir / pkg_filename

        cmd = [
            "pkgbuild",
            "--root",
            str(component_root),
            "--identifier",
            pkg_id,
            "--version",
            version,
            "--install-location",
            "/",
            str(pkg_path),
        ]

        # Attach scripts for any component that provides templates in publish/macos_pkg/scripts/.
        templates_dir = resources_dir / "scripts"
        has_templates = any(
            (templates_dir / f"{comp.key}_{name}").exists() for name in ("preinstall", "postinstall", "uninstall")
        )
        if has_templates:
            scripts_dir = work_dir / "scripts" / comp.key
            if scripts_dir.exists() and not dry_run:
                shutil.rmtree(scripts_dir, ignore_errors=True)
            if not dry_run:
                _materialize_pkg_scripts_from_templates(comp.key, templates_dir, scripts_dir)
                cmd = cmd[:-1] + ["--scripts", str(scripts_dir)] + cmd[-1:]

        _run(cmd, dry_run=dry_run)
        built.append((comp, pkg_id, pkg_filename))

    if not dry_run:
        # Do not auto-generate/overwrite project HTML resources here.
        _write_distribution_xml(
            title="BuckyOS",
            version=version,
            components=built,
            resources_dir=resources_dir,
            out_path=dist_xml,
        )
    else:
        print(f"[dry-run] would write distribution XML: {dist_xml}")

    if not dry_run:
        out_dir.mkdir(parents=True, exist_ok=True)
    out_pkg = out_dir / f"buckyos-apple-{architecture}-{version}.pkg"

    product_cmd = [
        "productbuild",
        "--distribution",
        str(dist_xml),
        "--resources",
        str(resources_dir),
        "--package-path",
        str(pkgs_dir),
        str(out_pkg),
    ]
    _run(product_cmd, dry_run=dry_run)
    return out_pkg


def _pkgutil_expand(pkg_path: Path, out_dir: Path) -> None:
    out_dir.parent.mkdir(parents=True, exist_ok=True)
    if out_dir.exists():
        shutil.rmtree(out_dir, ignore_errors=True)
    subprocess.run(["pkgutil", "--expand", str(pkg_path), str(out_dir)], check=True)


def _pkgutil_expand_full(pkg_path: Path, out_dir: Path) -> None:
    """
    Expand and extract payload for a (flat) pkg.

    Works for both the top-level product archive and embedded component pkgs.
    """
    out_dir.parent.mkdir(parents=True, exist_ok=True)
    if out_dir.exists():
        shutil.rmtree(out_dir, ignore_errors=True)
    subprocess.run(["pkgutil", "--expand-full", str(pkg_path), str(out_dir)], check=True)


def _pkgutil_flatten(pkg_dir: Path, out_pkg_path: Path) -> None:
    """
    Convert an expanded package directory into a flat .pkg file.

    Note: when a product archive is expanded with `pkgutil --expand`, embedded
    component packages appear as directories (e.g. `buckyos.pkg/` containing
    `Bom`, `Payload`, `PackageInfo`, ...). `pkgutil --expand-full` only accepts
    a *flat* package file, so we flatten first.
    """
    out_pkg_path.parent.mkdir(parents=True, exist_ok=True)
    if out_pkg_path.exists():
        out_pkg_path.unlink(missing_ok=True)
    subprocess.run(["pkgutil", "--flatten", str(pkg_dir), str(out_pkg_path)], check=True)


def _host_arch() -> str:
    m = (platform.machine() or "").lower()
    if m in ("amd64",):
        return "x86_64"
    if m in ("aarch64",):
        return "arm64"
    return m or "unknown"


def _is_macho_candidate(rel_path: str, p: Path) -> bool:
    # Heuristics to avoid calling `file` on every single file.
    # We still want to catch dylibs/node addons/app binaries even if not +x.
    rp = rel_path.replace("\\", "/")
    if "/.dSYM/" in rp:
        return False
    if rp.startswith("bin/") or "/bin/" in rp:
        return True
    if rp.endswith((".dylib", ".so", ".node")):
        return True
    if "/Contents/MacOS/" in rp:
        return True
    try:
        return os.access(p, os.X_OK)
    except Exception:
        return False


def _parse_macho_arches(file_output: str) -> Optional[List[str]]:
    """
    Parse `file` output and return arch list for Mach-O files, else None.
    """
    s = file_output.strip()
    if "Mach-O" not in s:
        return None
    # Universal binary: ... [x86_64:...] [arm64:...]
    if "universal binary" in s:
        arches = re.findall(r"\[([A-Za-z0-9_]+):", s)
        return list(dict.fromkeys([a.lower() for a in arches])) or ["unknown"]
    arches: List[str] = []
    for a in ("arm64", "x86_64"):
        if a in s:
            arches.append(a)
    return arches or ["unknown"]


def _pkg_payload_files(pkg_component_dir: Path) -> List[str]:
    """
    Return payload file list for an embedded component package.

    When verifying an expanded product archive, embedded component packages are directories
    that contain a `Bom` file. In that case, use `lsbom` to list entries.
    """
    if pkg_component_dir.is_dir():
        bom = pkg_component_dir / "Bom"
        if bom.exists():
            out = subprocess.check_output(["lsbom", "-s", str(bom)])
            return [line.strip() for line in out.decode("utf-8", errors="ignore").splitlines() if line.strip()]

    # Fallback for unexpanded/flat component packages.
    out = subprocess.check_output(["pkgutil", "--payload-files", str(pkg_component_dir)])
    return [line.strip() for line in out.decode("utf-8", errors="ignore").splitlines() if line.strip()]


def verify_pkg(
    *,
    pkg_path: Path,
    project_yaml_path: Path,
) -> int:
    """
    Offline verification for the built macOS .pkg.

    Checks:
    - Distribution choices exist for all publish.macos_pkg.apps components
    - optional:false => required=true and enabled=false
    - Per-component scripts are attached iff templates exist in publish/macos_pkg/scripts/
    - buckyos payload contains data_paths under .buckyos_installer_defaults and not at real paths
    - Mach-O binaries inside payload are runnable on this host (avoid Rosetta surprise)
    """
    components = load_macos_pkg_components(project_yaml_path)
    expected_keys = {c.key for c in components}
    by_choice_id: Dict[str, PublishComponent] = {f"choice.{_sanitize_id(c.key)}": c for c in components}

    failures: List[str] = []

    with tempfile.TemporaryDirectory(prefix="buckyos-pkg-verify-") as td:
        work = Path(td)
        expanded = work / "expanded"
        _pkgutil_expand(pkg_path, expanded)

        dist_file = expanded / "Distribution"
        if not dist_file.exists():
            failures.append("missing Distribution file")
        else:
            xml = dist_file.read_text(encoding="utf-8", errors="ignore")
            try:
                root = ET.fromstring(xml)
            except ET.ParseError as e:
                failures.append(f"Distribution XML parse error: {e}")
                root = None

            if root is not None:
                # Collect choices
                choices = {c.attrib.get("id", ""): c for c in root.findall(".//choice")}
                for choice_id, comp in by_choice_id.items():
                    if choice_id not in choices:
                        failures.append(f"missing choice for component {comp.key} (expected id={choice_id})")
                        continue
                    elem = choices[choice_id]
                    required = elem.attrib.get("required")
                    enabled = elem.attrib.get("enabled")
                    if not comp.optional:
                        if required != "true":
                            failures.append(f"component {comp.key} should be required=true, got {required}")
                        if enabled != "false":
                            failures.append(f"component {comp.key} should be enabled=false (locked), got {enabled}")
                    else:
                        if required not in (None, "false"):
                            failures.append(f"component {comp.key} should be required=false, got {required}")

        # Verify component packages exist and scripts attachment.
        # Embedded component packages are named by sanitized key: {sanitize(key)}.pkg
        templates_dir = MACOS_PKG_PROJECT_DIR / "scripts"
        for comp in components:
            subpkg_dir = expanded / f"{_sanitize_id(comp.key)}.pkg"
            if not subpkg_dir.exists():
                failures.append(f"missing embedded component package: {subpkg_dir.name}")
                continue

            scripts_dir = subpkg_dir / "Scripts"
            expects_scripts = any(
                (templates_dir / f"{comp.key}_{name}").exists() for name in ("preinstall", "postinstall", "uninstall")
            )
            if expects_scripts and not scripts_dir.exists():
                failures.append(f"component {comp.key} should have Scripts/ but it is missing")
            if (not expects_scripts) and scripts_dir.exists():
                failures.append(f"component {comp.key} should NOT have Scripts/ (no templates provided)")

        # Verify data_paths payload staging for buckyos.
        buckyos_pkg_dir = expanded / "buckyos.pkg"
        if buckyos_pkg_dir.exists():
            layout = load_buckyos_layout(project_yaml_path, target_override="/opt/buckyos")
            payload_files = set(_pkg_payload_files(buckyos_pkg_dir))

            def normalize_payload_path(p: str) -> str:
                # pkgutil lists paths without leading '/', relative to install-location (/)
                return p.lstrip("./").lstrip("/")

            for rel in layout.data_paths:
                rel_s = rel.strip().lstrip("/").rstrip("/")
                real_prefix = f"opt/buckyos/{rel_s}"
                defaults_prefix = f"opt/buckyos/{BUCKYOS_DEFAULTS_SUBDIR}/{rel_s}"

                real_present = any(normalize_payload_path(p).startswith(real_prefix) for p in payload_files)
                defaults_present = any(normalize_payload_path(p).startswith(defaults_prefix) for p in payload_files)
                if real_present:
                    failures.append(f"data_paths '{rel}' should NOT be in payload at '{real_prefix}' (would overwrite)")
                if not defaults_present:
                    failures.append(f"data_paths '{rel}' missing from defaults payload at '{defaults_prefix}'")

        else:
            failures.append("missing embedded buckyos.pkg (cannot verify data_paths semantics)")

        # Verify payload Mach-O architectures (best-effort, offline).
        host_arch = _host_arch()
        embedded_pkgs = sorted([p for p in expanded.iterdir() if p.is_dir() and p.name.endswith(".pkg")])
        macho_findings: List[Tuple[str, str]] = []  # (payload_rel_path, file_output)
        foreign_arch: List[Tuple[str, List[str]]] = []  # (payload_rel_path, arches)

        for subpkg_dir in embedded_pkgs:
            extract_dir = work / "payloads" / subpkg_dir.name
            try:
                flat_pkg = work / "payloads_flat" / f"{subpkg_dir.name}.flat.pkg"
                _pkgutil_flatten(subpkg_dir, flat_pkg)
                _pkgutil_expand_full(flat_pkg, extract_dir)
            except subprocess.CalledProcessError as e:
                failures.append(f"pkgutil extract payload failed for {subpkg_dir.name}: {e}")
                continue

            payload_root = extract_dir / "Payload"
            if not payload_root.exists():
                # Some pkgutil versions may extract payload directly under extract_dir.
                payload_root = extract_dir

            for p in payload_root.rglob("*"):
                if not p.is_file():
                    continue
                try:
                    rel = p.relative_to(payload_root).as_posix()
                except Exception:
                    rel = p.name
                if not _is_macho_candidate(rel, p):
                    continue
                try:
                    out = subprocess.check_output(["file", str(p)]).decode("utf-8", errors="ignore").strip()
                except Exception:
                    continue
                arches = _parse_macho_arches(out)
                if arches is None:
                    continue
                macho_findings.append((rel, out))
                # If host arch is known, ensure it's present in the Mach-O arches list.
                if host_arch not in ("unknown", "") and host_arch not in arches and "unknown" not in arches:
                    foreign_arch.append((rel, arches))

        if macho_findings:
            print(f"[verify] Mach-O files found: {len(macho_findings)} (host_arch={host_arch})")
        else:
            print(f"[verify] No Mach-O files found in payloads (host_arch={host_arch})")

        if foreign_arch:
            # Common Rosetta trigger on Apple Silicon: x86_64-only payload binaries.
            # Fail with actionable paths.
            for rel, arches in foreign_arch[:200]:
                failures.append(f"payload Mach-O not runnable on host ({host_arch}): {rel} arches={arches}")
            if len(foreign_arch) > 200:
                failures.append(f"... and {len(foreign_arch) - 200} more foreign-arch Mach-O files")

    if failures:
        print("VERIFY FAIL:")
        for f in failures:
            print("-", f)
        return 1

    print("VERIFY PASS")
    return 0


def _safe_join(root: Path, rel: str) -> Path:
    rel = rel.strip()
    if rel.startswith("/"):
        rel = rel[1:]
    # prevent escaping root
    candidate = (root / rel).resolve()
    if root.resolve() not in candidate.parents and candidate != root.resolve():
        raise ValueError(f"Refusing to operate outside target root: {candidate} (root={root})")
    return candidate


def _remove_path(path: Path, dry_run: bool) -> None:
    if not path.exists() and not path.is_symlink():
        return
    if dry_run:
        print(f"[dry-run] remove: {path}")
        return
    if path.is_symlink() or path.is_file():
        path.unlink(missing_ok=True)
        return
    shutil.rmtree(path, ignore_errors=True)


def _copy_path(src: Path, dst: Path, overwrite: bool, dry_run: bool) -> None:
    if not src.exists() and not src.is_symlink():
        print(f"[warn] source missing, skip: {src}")
        return
    if dry_run:
        mode = "overwrite" if overwrite else "no-overwrite"
        print(f"[dry-run] copy({mode}): {src} -> {dst}")
        return
    dst.parent.mkdir(parents=True, exist_ok=True)
    if overwrite and (dst.exists() or dst.is_symlink()):
        _remove_path(dst, dry_run=False)
    if src.is_dir():
        shutil.copytree(src, dst, dirs_exist_ok=True)
    else:
        shutil.copy2(src, dst)


def _is_dir_path(rel: str) -> bool:
    return rel.endswith("/")


def action_update(layout: AppLayout, dry_run: bool = False) -> None:
    layout.target_rootfs.mkdir(parents=True, exist_ok=True)
    # overwrite modules
    for rel in layout.module_paths:
        src = _safe_join(layout.source_rootfs, rel)
        dst = _safe_join(layout.target_rootfs, rel)
        _copy_path(src, dst, overwrite=True, dry_run=dry_run)

    # ensure data paths exist, but never overwrite existing
    for rel in layout.data_paths:
        src = _safe_join(layout.source_rootfs, rel)
        dst = _safe_join(layout.target_rootfs, rel)
        if dst.exists() or dst.is_symlink():
            continue
        if _is_dir_path(rel):
            if dry_run:
                print(f"[dry-run] mkdir: {dst}")
            else:
                dst.mkdir(parents=True, exist_ok=True)
            # if source dir exists, copy its initial contents once
            if src.exists():
                _copy_path(src, dst, overwrite=False, dry_run=dry_run)
        else:
            if src.exists():
                _copy_path(src, dst, overwrite=False, dry_run=dry_run)
            else:
                if dry_run:
                    print(f"[dry-run] skip missing data template: {src}")
                else:
                    dst.parent.mkdir(parents=True, exist_ok=True)


def action_install(layout: AppLayout, dry_run: bool = False) -> None:
    action_uninstall(layout, dry_run=dry_run)
    action_update(layout, dry_run=dry_run)


def action_uninstall(layout: AppLayout, dry_run: bool = False) -> None:
    if not layout.target_rootfs.exists():
        return

    # remove module outputs first
    for rel in layout.module_paths:
        dst = _safe_join(layout.target_rootfs, rel)
        _remove_path(dst, dry_run=dry_run)

    # then clean paths
    for rel in layout.clean_paths:
        dst = _safe_join(layout.target_rootfs, rel)
        _remove_path(dst, dry_run=dry_run)


def generate_macos_scripts(layout: AppLayout, scripts_dir: Path) -> None:
    # This repository treats install/uninstall scripts as project-owned assets.
    # This function is retained only for backwards compatibility with older workflows,
    # but MUST NOT generate/overwrite any scripts.
    _ = layout
    _ = scripts_dir
    return None


def _materialize_pkg_scripts_from_templates(component_key: str, templates_dir: Path, out_scripts_dir: Path) -> None:
    """
    Convert component templates (<key>_preinstall/<key>_postinstall) into
    pkgbuild-recognized script names (preinstall/postinstall).
    """
    out_scripts_dir.mkdir(parents=True, exist_ok=True)
    mapping = {
        "preinstall": templates_dir / f"{component_key}_preinstall",
        "postinstall": templates_dir / f"{component_key}_postinstall",
    }
    for dst_name, src_path in mapping.items():
        if not src_path.exists():
            continue
        shutil.copy2(src_path, out_scripts_dir / dst_name)
        (out_scripts_dir / dst_name).chmod(0o755)
    # Also ship the uninstall helper (not auto-executed by Installer).
    uninstall_tpl = templates_dir / f"{component_key}_uninstall"
    if uninstall_tpl.exists():
        shutil.copy2(uninstall_tpl, out_scripts_dir / "uninstall")
        (out_scripts_dir / "uninstall").chmod(0o755)


AUTO_BEGIN = "# BEGIN AUTO-GENERATED:"
AUTO_END = "# END AUTO-GENERATED:"


def _detect_root_var(script_text: str) -> str:
    # Try to detect a var like BUCKYOS_ROOT=... or TARGET_ROOT=...
    for line in script_text.splitlines()[:80]:
        m = re.match(r"\s*([A-Z0-9_]+_ROOT)\s*=", line)
        if m:
            return f"${m.group(1)}"
    return "$BUCKYOS_ROOT"


def _detect_defaults_var(script_text: str) -> str:
    for line in script_text.splitlines()[:120]:
        m = re.match(r"\s*(DEFAULTS_DIR)\s*=", line)
        if m:
            return f"${m.group(1)}"
    return '$DEFAULTS_DIR'


def _rm_lines(root_var: str, rel_paths: List[str]) -> List[str]:
    out: List[str] = []
    for rel in rel_paths:
        rel_s = rel.strip().lstrip("/").rstrip("/")
        if not rel_s:
            continue
        out.append(f'rm -rf "{root_var}/{rel_s}"')
    return out


def _data_copy_lines(root_var: str, defaults_var: str, rel_paths: List[str]) -> List[str]:
    out: List[str] = []
    for rel in rel_paths:
        rel_s = rel.strip().lstrip("/")
        if not rel_s:
            continue
        if rel_s.endswith("/"):
            rel_s = rel_s.rstrip("/")
            out += [
                # If the destination dir already exists but is empty (common when payload creates it),
                # treat it as "missing" and seed it once from defaults.
                f'if [ -d "{defaults_var}/{rel_s}" ]; then',
                f'  ditto "{defaults_var}/{rel_s}" "{root_var}/{rel_s}"',
                "fi",
            ]
        else:
            out += [
                f'if [ ! -e "{root_var}/{rel_s}" ] && [ -e "{defaults_var}/{rel_s}" ]; then',
                f'  mkdir -p "$(dirname "{root_var}/{rel_s}")"',
                f'  cp -p "{defaults_var}/{rel_s}" "{root_var}/{rel_s}"',
                "fi",
            ]
    return out


def _replace_marked_block(text: str, block_name: str, new_lines: List[str], indent: str = "") -> str:
    begin = f"{AUTO_BEGIN} {block_name}"
    end = f"{AUTO_END} {block_name}"
    lines = text.splitlines()
    try:
        i0 = next(i for i, l in enumerate(lines) if l.strip() == begin)
        i1 = next(i for i, l in enumerate(lines) if l.strip() == end and i > i0)
    except StopIteration:
        # If missing markers, append at end.
        appended = [begin] + [indent + l for l in new_lines] + [end]
        return text.rstrip() + "\n" + "\n".join(appended) + "\n"

    replaced = lines[: i0 + 1] + [indent + l for l in new_lines] + lines[i1:]
    # Always ensure a trailing newline on writeback.
    return "\n".join(replaced).rstrip("\n") + "\n"


def sync_macos_scripts(project_yaml_path: Path, scripts_dir: Path) -> None:
    """
    Project helper: update *existing* scripts in publish/macos_pkg/scripts/ based on bucky_project.yaml.

    It only updates sections wrapped by markers:
      # BEGIN AUTO-GENERATED: <name>
      ...
      # END AUTO-GENERATED: <name>
    """
    data = yaml_load_file(project_yaml_path)
    apps = data.get("apps", {}) or {}

    for app_key in apps.keys():
        layout = load_app_layout(project_yaml_path, app_key)

        pre = scripts_dir / f"{app_key}_preinstall"
        if pre.exists():
            txt = pre.read_text(encoding="utf-8", errors="ignore")
            root_var = _detect_root_var(txt)
            txt = _replace_marked_block(txt, "modules", _rm_lines(root_var, layout.module_paths))
            pre.write_text(txt.rstrip("\n") + "\n", encoding="utf-8")

        post = scripts_dir / f"{app_key}_postinstall"
        if post.exists():
            txt = post.read_text(encoding="utf-8", errors="ignore")
            root_var = _detect_root_var(txt)
            defaults_var = _detect_defaults_var(txt)
            # Most postinstall templates place the block inside `if [ -d "$DEFAULTS_DIR" ]; then`
            txt = _replace_marked_block(txt, "data_paths", _data_copy_lines(root_var, defaults_var, layout.data_paths), indent="  ")
            post.write_text(txt.rstrip("\n") + "\n", encoding="utf-8")

        un = scripts_dir / f"{app_key}_uninstall"
        if un.exists():
            txt = un.read_text(encoding="utf-8", errors="ignore")
            root_var = _detect_root_var(txt)
            txt = _replace_marked_block(txt, "modules", _rm_lines(root_var, layout.module_paths))
            txt = _replace_marked_block(txt, "clean_paths", _rm_lines(root_var, layout.clean_paths))
            un.write_text(txt.rstrip("\n") + "\n", encoding="utf-8")


def build_pkg(architecture: str, version: str, **kwargs: Any) -> None:
    # Kept for backward compatibility with legacy callers.
    _ = build_macos_distribution_pkg(architecture=architecture, version=version, **kwargs)


def _legacy_build_main(argv: List[str]) -> int:
    # Backward compatibility:
    #   python make_local_osx_pkg.py <architecture> <version>
    subcommands = {"build-pkg", "sync-macos-scripts", "verify-pkg", "install", "update", "uninstall"}
    if len(argv) == 3 and (argv[1] not in subcommands) and (not argv[1].startswith("-")):
        architecture = argv[1]
        version = argv[2]
        if architecture == "x86_64":
            architecture = "amd64"
        build_pkg(
            architecture,
            version,
            project_yaml_path=PROJECT_YAML,
            app_publish_dir=RESULT_ROOT_DIR,
            out_dir=Path.cwd() / "publish",
            dry_run=False,
        )
        print(f"make_local_osx_pkg.py completed for {architecture} version {version}")
        return 0
    return 2


def main(argv: List[str]) -> int:
    legacy_rc = _legacy_build_main(argv)
    if legacy_rc != 2:
        return legacy_rc

    parser = argparse.ArgumentParser(prog="make_local_osx_pkg.py")
    sub = parser.add_subparsers(dest="cmd", required=True)

    p_build = sub.add_parser("build-pkg", help="Build macOS .pkg (distribution with component choices)")
    p_build.add_argument("architecture", help="amd64|aarch64 (x86_64 accepted)")
    p_build.add_argument("version", help="Version string")
    p_build.add_argument("--project", default=str(PROJECT_YAML), help="Path to bucky_project.yaml")
    p_build.add_argument(
        "--app-publish-dir",
        default=str(RESULT_ROOT_DIR),
        help="Base directory to resolve publish.macos_pkg.apps.* sources (default: $BUCKYOS_BUILD_ROOT)",
    )
    p_build.add_argument(
        "--out-dir",
        default=str(Path.cwd() / "publish"),
        help='Output directory for the final .pkg (default: "./publish")',
    )
    p_build.add_argument(
        "--no-sync-scripts",
        action="store_true",
        help="Do not auto-sync publish/macos_pkg/scripts from bucky_project.yaml before build",
    )
    p_build.add_argument(
        "--extra-bundle",
        action="append",
        default=[],
        help="Extra .app bundle path to include as an optional component (repeatable)",
    )
    p_build.add_argument("--dry-run", action="store_true", help="Print commands without executing them")

    p_sync = sub.add_parser("sync-macos-scripts", help="Regenerate macos_pkg preinstall/postinstall/uninstall from bucky_project.yaml")
    p_sync.add_argument("--project", default=str(PROJECT_YAML), help="Path to bucky_project.yaml")

    p_verify = sub.add_parser("verify-pkg", help="Verify a built macOS .pkg offline (no install)")
    p_verify.add_argument("pkg", help="Path to .pkg")
    p_verify.add_argument("--project", default=str(PROJECT_YAML), help="Path to bucky_project.yaml")

    for name in ("install", "update", "uninstall"):
        p = sub.add_parser(name, help=f"Local filesystem action: {name}")
        p.add_argument("--project", default=str(PROJECT_YAML), help="Path to bucky_project.yaml")
        p.add_argument("--target", default=None, help="Override target rootfs (default from bucky_project.yaml)")
        p.add_argument("--source", default=None, help="Override source rootfs (default from bucky_project.yaml)")
        p.add_argument("--dry-run", action="store_true", help="Print actions without changing filesystem")

    args = parser.parse_args(argv[1:])

    if args.cmd == "build-pkg":
        arch = args.architecture
        if arch == "x86_64":
            arch = "amd64"
        extra_bundles = [Path(p) for p in (args.extra_bundle or [])]
        if args.no_sync_scripts:
            os.environ["BUCKYOS_PKG_NO_SYNC_SCRIPTS"] = "1"
        out_pkg = build_macos_distribution_pkg(
            architecture=arch,
            version=args.version,
            project_yaml_path=Path(args.project),
            app_publish_dir=Path(args.app_publish_dir),
            out_dir=Path(args.out_dir),
            extra_bundles=extra_bundles,
            dry_run=bool(args.dry_run),
        )
        print(f"pkg built: {out_pkg}")
        return 0

    if args.cmd == "sync-macos-scripts":
        sync_macos_scripts(Path(args.project), SRC_DIR / "publish" / "macos_pkg" / "scripts")
        print("macos_pkg scripts synced.")
        return 0

    if args.cmd == "verify-pkg":
        return verify_pkg(pkg_path=Path(args.pkg).expanduser().resolve(), project_yaml_path=Path(args.project))

    layout = load_buckyos_layout(Path(args.project), target_override=args.target)
    if args.source:
        layout = AppLayout(
            source_rootfs=Path(args.source).resolve(),
            target_rootfs=layout.target_rootfs,
            module_paths=layout.module_paths,
            data_paths=layout.data_paths,
            clean_paths=layout.clean_paths,
        )

    if args.cmd == "install":
        action_install(layout, dry_run=args.dry_run)
        return 0
    if args.cmd == "update":
        action_update(layout, dry_run=args.dry_run)
        return 0
    if args.cmd == "uninstall":
        action_uninstall(layout, dry_run=args.dry_run)
        return 0

    return 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))