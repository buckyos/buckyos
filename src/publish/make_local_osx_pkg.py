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
import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Tuple

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


def load_buckyos_layout(project_yaml_path: Path = PROJECT_YAML, target_override: str | None = None) -> AppLayout:
    data = yaml_load_file(project_yaml_path)
    apps = data.get("apps", {})
    buckyos = apps.get("buckyos", {})

    base_dir = str(data.get("base_dir", "."))
    project_base = (project_yaml_path.parent / base_dir).resolve()

    rootfs_rel = str(buckyos.get("rootfs", "rootfs/"))
    source_rootfs = (project_base / rootfs_rel).resolve()

    default_target = str(buckyos.get("default_target_rootfs", "${BUCKYOS_ROOT}"))
    target_str = target_override if target_override else default_target
    target_rootfs = Path(_expand_vars(target_str)).resolve()

    modules = buckyos.get("modules", {}) or {}
    module_paths = [str(p) for p in modules.values()]
    data_paths = [str(p) for p in (buckyos.get("data_paths", []) or [])]
    clean_paths = [str(p) for p in (buckyos.get("clean_paths", []) or [])]

    return AppLayout(
        source_rootfs=source_rootfs,
        target_rootfs=target_rootfs,
        module_paths=module_paths,
        data_paths=data_paths,
        clean_paths=clean_paths,
    )


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
        if s.is_dir():
            shutil.copytree(s, d, dirs_exist_ok=True)
        elif s.exists():
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

        # Attach scripts only for buckyos (service setup + overwrite semantics).
        if comp.key == "buckyos":
            layout = load_buckyos_layout(project_yaml_path, target_override="/opt/buckyos")
            templates_dir = resources_dir / "scripts"
            # Ensure component templates exist (generate if missing/outdated).
            generate_macos_scripts(layout, templates_dir)
            scripts_dir = work_dir / "scripts" / "buckyos"
            if scripts_dir.exists() and not dry_run:
                shutil.rmtree(scripts_dir, ignore_errors=True)
            if not dry_run:
                _materialize_pkg_scripts_from_templates("buckyos", templates_dir, scripts_dir)
                cmd = cmd[:-1] + ["--scripts", str(scripts_dir)] + cmd[-1:]

        _run(cmd, dry_run=dry_run)
        built.append((comp, pkg_id, pkg_filename))

    if not dry_run:
        # Keep welcome page aligned with current component install locations.
        generate_welcome_html(components, resources_dir / "welcome.html")
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
    """
    Generate scripts under `src/publish/macos_pkg/scripts/`.

    Naming convention:
    - Component-scoped templates: `buckyos_preinstall`, `buckyos_postinstall`, `buckyos_uninstall`
    - Backward-compatible entrypoints: `preinstall`, `postinstall`, `uninstall` (wrappers)
    """
    scripts_dir.mkdir(parents=True, exist_ok=True)
    target_var = '${BUCKYOS_ROOT:-/opt/buckyos}'

    def norm(p: str) -> str:
        p = p.strip()
        if p.startswith("/"):
            p = p[1:]
        return p

    def is_dir(p: str) -> bool:
        return p.endswith("/")

    def strip_dir(p: str) -> str:
        return p.rstrip("/")

    # buckyos_preinstall: remove module paths only (keep data_paths)
    buckyos_preinstall = [
        "#!/bin/zsh",
        "set -e",
        f'BUCKYOS_ROOT="{target_var}"',
        'echo "[buckyos] preinstall: remove module paths (keep data)"',
    ]
    for rel in sorted(set(layout.module_paths)):
        buckyos_preinstall.append(f'rm -rf "$BUCKYOS_ROOT/{strip_dir(norm(rel))}"')
    (scripts_dir / "buckyos_preinstall").write_text("\n".join(buckyos_preinstall) + "\n", encoding="utf-8")

    # buckyos_postinstall: setup service plist (only meaningful when buckyos component installed)
    buckyos_postinstall = [
        "#!/bin/zsh",
        "set -e",
        f'BUCKYOS_ROOT="{target_var}"',
        f'DEFAULTS_DIR="$BUCKYOS_ROOT/{BUCKYOS_DEFAULTS_SUBDIR}"',
        'PLIST="/Library/LaunchAgents/buckyos.service.plist"',
        'mkdir -p "/Library/LaunchAgents"',
        'cat > "$PLIST" << EOF',
        '<?xml version="1.0" encoding="UTF-8"?>',
        '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">',
        '<plist version="1.0">',
        "<dict>",
        "    <key>Label</key>",
        "    <string>buckyos.service</string>",
        "    <key>ProgramArguments</key>",
        "    <array>",
        "        <string>${BUCKYOS_ROOT}/bin/node-daemon/node-daemon</string>",
        "        <string>--enable_active</string>",
        "    </array>",
        "    <key>RunAtLoad</key>",
        "    <true/>",
        "    <key>KeepAlive</key>",
        "    <true/>",
        "    <key>StandardErrorPath</key>",
        "    <string>/var/log/buckyos-node-daemon.err</string>",
        "    <key>StandardOutPath</key>",
        "    <string>/var/log/buckyos-node-daemon.log</string>",
        "</dict>",
        "</plist>",
        "EOF",
        'chown root:wheel "$PLIST" || true',
        'chmod 644 "$PLIST"',
        'launchctl stop buckyos.service || true',
        'launchctl unload "$PLIST" || true',
        'launchctl load "$PLIST"',
        'echo "BuckyOS install success, open http://127.0.0.1:3182/index.html to start, ENJOY!"',
        'echo "[buckyos] postinstall: install data_paths if missing (overwrite install semantics)"',
        'if [ -d "$DEFAULTS_DIR" ]; then',
    ]
    for rel in (layout.data_paths or []):
        rel_s = rel.strip()
        if rel_s.startswith("/"):
            rel_s = rel_s[1:]
        if rel_s.endswith("/"):
            rel_s = rel_s.rstrip("/")
            buckyos_postinstall += [
                f'  if [ ! -d "$BUCKYOS_ROOT/{rel_s}" ] && [ -d "$DEFAULTS_DIR/{rel_s}" ]; then',
                f'    mkdir -p "$(dirname "$BUCKYOS_ROOT/{rel_s}")"',
                f'    ditto "$DEFAULTS_DIR/{rel_s}" "$BUCKYOS_ROOT/{rel_s}"',
                "  fi",
            ]
        else:
            buckyos_postinstall += [
                f'  if [ ! -e "$BUCKYOS_ROOT/{rel_s}" ] && [ -e "$DEFAULTS_DIR/{rel_s}" ]; then',
                f'    mkdir -p "$(dirname "$BUCKYOS_ROOT/{rel_s}")"',
                f'    cp -p "$DEFAULTS_DIR/{rel_s}" "$BUCKYOS_ROOT/{rel_s}"',
                "  fi",
            ]
    buckyos_postinstall += [
        '  rm -rf "$DEFAULTS_DIR" || true',
        "fi",
    ]
    (scripts_dir / "buckyos_postinstall").write_text("\n".join(buckyos_postinstall) + "\n", encoding="utf-8")

    # buckyos_uninstall: remove modules + clean_paths
    buckyos_uninstall = [
        "#!/bin/zsh",
        "set -e",
        f'BUCKYOS_ROOT="{target_var}"',
        'PLIST="/Library/LaunchAgents/buckyos.service.plist"',
        'echo "[buckyos] uninstall: stopping service"',
        "launchctl stop buckyos.service || true",
        'launchctl unload "$PLIST" || true',
        'rm -f "$PLIST" || true',
        'echo "[buckyos] uninstall: removing modules + clean_paths"',
    ]
    for rel in sorted(set(layout.module_paths)):
        buckyos_uninstall.append(f'rm -rf "$BUCKYOS_ROOT/{strip_dir(norm(rel))}"')

    # Clean paths: remove as-is (even if it overlaps with data_paths).
    for rel in sorted(set(layout.clean_paths)):
        rel_n = norm(rel)
        buckyos_uninstall.append(f'rm -rf "$BUCKYOS_ROOT/{strip_dir(rel_n)}"')
    (scripts_dir / "buckyos_uninstall").write_text("\n".join(buckyos_uninstall) + "\n", encoding="utf-8")

    # Backward-compatible wrappers (single-pkg workflows).
    wrapper = [
        "#!/bin/zsh",
        "set -e",
        'DIR="$(cd "$(dirname "$0")" && pwd)"',
    ]
    (scripts_dir / "preinstall").write_text("\n".join(wrapper + ['zsh "$DIR/buckyos_preinstall"']) + "\n", encoding="utf-8")
    (scripts_dir / "postinstall").write_text("\n".join(wrapper + ['zsh "$DIR/buckyos_postinstall"']) + "\n", encoding="utf-8")
    (scripts_dir / "uninstall").write_text("\n".join(wrapper + ['zsh "$DIR/buckyos_uninstall"']) + "\n", encoding="utf-8")


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


def build_pkg(architecture: str, version: str, **kwargs: Any) -> None:
    # Kept for backward compatibility with legacy callers.
    _ = build_macos_distribution_pkg(architecture=architecture, version=version, **kwargs)


def _legacy_build_main(argv: List[str]) -> int:
    # Backward compatibility:
    #   python make_local_osx_pkg.py <architecture> <version>
    subcommands = {"build-pkg", "sync-macos-scripts", "install", "update", "uninstall"}
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
        "--extra-bundle",
        action="append",
        default=[],
        help="Extra .app bundle path to include as an optional component (repeatable)",
    )
    p_build.add_argument("--dry-run", action="store_true", help="Print commands without executing them")

    p_sync = sub.add_parser("sync-macos-scripts", help="Regenerate macos_pkg preinstall/postinstall/uninstall from bucky_project.yaml")
    p_sync.add_argument("--project", default=str(PROJECT_YAML), help="Path to bucky_project.yaml")

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
        layout = load_buckyos_layout(Path(args.project), target_override="/opt/buckyos")
        generate_macos_scripts(layout, SRC_DIR / "publish" / "macos_pkg" / "scripts")
        print("macos_pkg scripts regenerated.")
        return 0

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