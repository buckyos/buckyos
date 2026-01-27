"""
Local Debian .deb builder + local installer helper.

This script supports:
- build-pkg: build a Debian .deb (single component payload)
- install:   clean + install_data + update (fresh install)
- update:    update only (overwrite modules, keep existing data_paths)
- uninstall: remove module paths + clean_paths

It reads:
- `apps.buckyos.*` for local install/update/uninstall on a directory.

Before making a deb, ensure you have built the latest buckyos rootfs.
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Dict, List

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
TMP_INSTALL_DIR = RESULT_ROOT_DIR / "deb-build"

DEB_TEMPLATE_DIR = Path(__file__).resolve().parent / "deb_template"
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


def _stage_buckyos_app_root(*, src_root: Path, dst_root: Path, layout: AppLayout) -> None:
    """
    Stage buckyos rootfs into dst_root.

    Semantics:
    - modules: always copied into real target paths (will be overwritten by package install)
    - data_paths: copied into `${BUCKYOS_ROOT}/.buckyos_installer_defaults/...`
      and postinst can copy to real paths only if missing (overwrite install behavior)
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


def _run(cmd: List[str], dry_run: bool, cwd: Path | None = None) -> None:
    print("+", " ".join(cmd))
    if dry_run:
        return
    subprocess.run(cmd, check=True, cwd=cwd)


def _normalize_deb_arch(arch: str) -> str:
    if arch == "x86_64":
        return "amd64"
    if arch == "aarch64":
        return "arm64"
    return arch


def _adjust_control_file(dest_dir: Path, new_version: str, architecture: str) -> None:
    control_file = dest_dir / "DEBIAN" / "control"
    content = control_file.read_text(encoding="utf-8")
    content = content.replace("{{package version here}}", new_version)
    content = content.replace("{{architecture}}", _normalize_deb_arch(architecture))
    control_file.write_text(content, encoding="utf-8")


AUTO_BEGIN = "# BEGIN AUTO-GENERATED:"
AUTO_END = "# END AUTO-GENERATED:"


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
                f'if [ -d "{defaults_var}/{rel_s}" ]; then',
                f'  if [ ! -d "{root_var}/{rel_s}" ]; then',
                f'    mkdir -p "{root_var}/{rel_s}"',
                "  fi",
                f'  if [ -z "$(ls -A "{root_var}/{rel_s}" 2>/dev/null)" ]; then',
                f'    cp -a "{defaults_var}/{rel_s}/." "{root_var}/{rel_s}/"',
                "  fi",
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
        appended = [begin] + [indent + l for l in new_lines] + [end]
        return text.rstrip() + "\n" + "\n".join(appended) + "\n"

    replaced = lines[: i0 + 1] + [indent + l for l in new_lines] + lines[i1:]
    return "\n".join(replaced).rstrip("\n") + "\n"


def sync_deb_scripts(project_yaml_path: Path, scripts_dir: Path) -> None:
    """
    Update debian maintainer scripts based on bucky_project.yaml.

    Only updates sections wrapped by markers:
      # BEGIN AUTO-GENERATED: <name>
      ...
      # END AUTO-GENERATED: <name>
    """
    layout = load_app_layout(project_yaml_path, "buckyos")

    preinst = scripts_dir / "preinst"
    if preinst.exists():
        txt = preinst.read_text(encoding="utf-8", errors="ignore")
        txt = _replace_marked_block(txt, "modules", _rm_lines("$BUCKYOS_ROOT", layout.module_paths))
        preinst.write_text(txt.rstrip("\n") + "\n", encoding="utf-8")

    postinst = scripts_dir / "postinst"
    if postinst.exists():
        txt = postinst.read_text(encoding="utf-8", errors="ignore")
        txt = _replace_marked_block(
            txt,
            "data_paths",
            _data_copy_lines("$BUCKYOS_ROOT", "$DEFAULTS_DIR", layout.data_paths),
            indent="  ",
        )
        postinst.write_text(txt.rstrip("\n") + "\n", encoding="utf-8")


def _resolve_buckyos_src(
    *,
    source_override: Path | None,
    app_publish_dir: Path,
    layout: AppLayout,
) -> Path:
    candidates: List[Path] = []
    if source_override:
        candidates.append(source_override)
    candidates.append(app_publish_dir / "buckyos")
    candidates.append(app_publish_dir / "publish" / "buckyos")
    candidates.append(layout.source_rootfs)
    for c in candidates:
        if c.exists():
            return c
    raise FileNotFoundError(
        "buckyos source rootfs not found. Tried: "
        + ", ".join(str(c) for c in candidates)
    )


def build_deb(
    *,
    architecture: str,
    version: str,
    project_yaml_path: Path,
    app_publish_dir: Path,
    out_dir: Path,
    source_rootfs: Path | None = None,
    dry_run: bool = False,
) -> Path:
    deb_arch = _normalize_deb_arch(architecture)

    work_root = TMP_INSTALL_DIR / "distbuild"
    deb_dir = work_root / deb_arch

    # Keep scripts in sync with bucky_project.yaml before building.
    if (not dry_run) and (not bool(os.environ.get("BUCKYOS_PKG_NO_SYNC_SCRIPTS"))):
        sync_deb_scripts(project_yaml_path, DEB_TEMPLATE_DIR / "DEBIAN")

    if deb_dir.exists() and not dry_run:
        shutil.rmtree(deb_dir, ignore_errors=True)

    if dry_run:
        print(f"[dry-run] copy template: {DEB_TEMPLATE_DIR} -> {deb_dir}")
    else:
        shutil.copytree(DEB_TEMPLATE_DIR, deb_dir, dirs_exist_ok=True)

    if not dry_run:
        _adjust_control_file(deb_dir, version, deb_arch)

    payload_root = deb_dir / "opt" / "buckyos"
    layout = load_buckyos_layout(project_yaml_path, target_override="/opt/buckyos")
    src_root = _resolve_buckyos_src(source_override=source_rootfs, app_publish_dir=app_publish_dir, layout=layout)

    if dry_run:
        print(f"[dry-run] stage buckyos: {src_root} -> {payload_root}")
    else:
        payload_root.mkdir(parents=True, exist_ok=True)
        _stage_buckyos_app_root(src_root=src_root, dst_root=payload_root, layout=layout)

    # Ensure maintainer scripts are executable
    for script_name in ("preinst", "postinst", "prerm", "postrm"):
        script_path = deb_dir / "DEBIAN" / script_name
        if script_path.exists():
            if dry_run:
                print(f"[dry-run] chmod 755 {script_path}")
            else:
                script_path.chmod(0o755)

    if dry_run:
        print(f"[dry-run] chmod -R 755 {deb_dir}")
    else:
        _run(["chmod", "-R", "755", str(deb_dir)], dry_run=False)

    out_dir.mkdir(parents=True, exist_ok=True)
    out_deb = out_dir / f"buckyos-linux-{deb_arch}-{version}.deb"

    if dry_run:
        print(f"[dry-run] dpkg-deb --build {deb_dir} {out_deb}")
    else:
        _run(["dpkg-deb", "--build", str(deb_dir), str(out_deb)], dry_run=False, cwd=work_root)
    return out_deb


def verify_pkg(
    *,
    pkg_path: Path,
    project_yaml_path: Path,
) -> int:
    """
    Verify a built Debian package using dpkg-deb.

    Checks:
    - File exists and is a valid .deb (basic extraction)
    - data_paths are staged under defaults, not in real locations
    """
    if not pkg_path.exists():
        print(f"VERIFY FAIL: .deb not found: {pkg_path}")
        return 1

    failures: List[str] = []

    # Try to find dpkg-deb
    try:
        subprocess.run(["dpkg-deb", "--version"], capture_output=True, check=True)
        dpkg_deb_ok = True
    except Exception:
        dpkg_deb_ok = False
        print("[verify] Warning: dpkg-deb not found, skipping payload inspection")

    if dpkg_deb_ok:
        with tempfile.TemporaryDirectory(prefix="buckyos-deb-verify-") as td:
            work = Path(td)
            extract_dir = work / "extract"
            extract_dir.mkdir(parents=True, exist_ok=True)
            try:
                subprocess.run(["dpkg-deb", "-x", str(pkg_path), str(extract_dir)], check=True)
            except subprocess.CalledProcessError as e:
                failures.append(f"dpkg-deb extract failed: {e}")
            else:
                layout = load_buckyos_layout(project_yaml_path, target_override="/opt/buckyos")
                root = extract_dir / "opt" / "buckyos"
                defaults_root = root / BUCKYOS_DEFAULTS_SUBDIR

                for rel in layout.data_paths:
                    rel_s = rel.strip().lstrip("/").rstrip("/")
                    if not rel_s:
                        continue
                    real_path = root / rel_s
                    defaults_path = defaults_root / rel_s
                    if real_path.exists():
                        failures.append(f"data_paths '{rel}' should NOT be in payload at '{real_path}'")
                    if not defaults_path.exists():
                        failures.append(f"data_paths '{rel}' missing from defaults payload at '{defaults_path}'")

    # Basic size sanity check
    file_size = pkg_path.stat().st_size
    if file_size < 1024 * 1024:
        failures.append(f"Package size suspiciously small: {file_size} bytes")
    print(f"[verify] Package size: {file_size / (1024 * 1024):.2f} MB")

    if failures:
        print("VERIFY FAIL:")
        for f in failures:
            print(f"- {f}")
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


def _legacy_build_main(argv: List[str]) -> int:
    # Backward compatibility:
    #   python make_local_deb.py <architecture> <version>
    subcommands = {"build-pkg", "install", "update", "uninstall"}
    if len(argv) == 3 and (argv[1] not in subcommands) and (not argv[1].startswith("-")):
        architecture = argv[1]
        version = argv[2]
        if architecture == "x86_64":
            architecture = "amd64"
        out_deb = build_deb(
            architecture=architecture,
            version=version,
            project_yaml_path=PROJECT_YAML,
            app_publish_dir=RESULT_ROOT_DIR,
            out_dir=Path.cwd() / "publish",
            dry_run=False,
        )
        print(f"make_local_deb.py completed: {out_deb}")
        return 0
    return 2


def main(argv: List[str]) -> int:
    legacy_rc = _legacy_build_main(argv)
    if legacy_rc != 2:
        return legacy_rc

    parser = argparse.ArgumentParser(prog="make_local_deb.py")
    sub = parser.add_subparsers(dest="cmd", required=True)

    p_build = sub.add_parser("build-pkg", help="Build Debian .deb package (single component payload)")
    p_build.add_argument("architecture", help="amd64|arm64 (x86_64 accepted)")
    p_build.add_argument("version", help="Version string")
    p_build.add_argument("--project", default=str(PROJECT_YAML), help="Path to bucky_project.yaml")
    p_build.add_argument(
        "--app-publish-dir",
        default=str(RESULT_ROOT_DIR),
        help="Base directory to resolve built rootfs (default: $BUCKYOS_BUILD_ROOT)",
    )
    p_build.add_argument(
        "--out-dir",
        default=str(Path.cwd() / "publish"),
        help='Output directory for the final .deb (default: "./publish")',
    )
    p_build.add_argument(
        "--no-sync-scripts",
        action="store_true",
        help="Do not auto-sync deb_template/DEBIAN scripts from bucky_project.yaml before build",
    )
    p_build.add_argument(
        "--source-rootfs",
        default=None,
        help="Override source rootfs path for buckyos payload",
    )
    p_build.add_argument("--dry-run", action="store_true", help="Print commands without executing them")

    p_verify = sub.add_parser("verify-pkg", help="Verify a built Debian .deb offline (no install)")
    p_verify.add_argument("pkg", help="Path to .deb")
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
        if args.no_sync_scripts:
            os.environ["BUCKYOS_PKG_NO_SYNC_SCRIPTS"] = "1"
        out_deb = build_deb(
            architecture=arch,
            version=args.version,
            project_yaml_path=Path(args.project),
            app_publish_dir=Path(args.app_publish_dir),
            out_dir=Path(args.out_dir),
            source_rootfs=Path(args.source_rootfs).resolve() if args.source_rootfs else None,
            dry_run=bool(args.dry_run),
        )
        print(f"deb built: {out_deb}")
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
