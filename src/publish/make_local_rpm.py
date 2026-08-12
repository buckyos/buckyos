"""
Local Fedora/RPM package builder for BuckyOS.

This script supports:
- build-pkg: build a Fedora-style .rpm payload
- verify-pkg: basic offline metadata/package validation

It uses the same manifest and app layout semantics as the Linux Debian builder:
- modules are installed into /opt/buckyos and overwrite existing program files
- data_paths are staged under /opt/buckyos/.buckyos_installer_defaults
"""

from __future__ import annotations

import argparse
import os
import re
import shlex
import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any, List

import package_common as common


SRC_DIR = Path(__file__).resolve().parent.parent
PROJECT_YAML = SRC_DIR / "bucky_project.yaml"

RESULT_ROOT_DIR = Path(os.environ.get("BUCKYOS_BUILD_ROOT", "/opt/buckyosci"))
TMP_INSTALL_DIR = RESULT_ROOT_DIR / "rpm-build"

RPM_PKG_DIR = Path(__file__).resolve().parent / "rpm_pkg"
RPM_SPEC_TEMPLATE = RPM_PKG_DIR / "buckyos.spec"
DEB_PKG_DIR = Path(__file__).resolve().parent / "deb_pkg"
BUCKYOS_DEFAULTS_SUBDIR = ".buckyos_installer_defaults"
IGNORED_STAGE_NAMES = {".DS_Store", "__pycache__"}


@dataclass(frozen=True)
class AppLayout:
    source_rootfs: Path
    target_rootfs: Path
    module_paths: List[str]
    data_paths: List[str]
    clean_paths: List[str]
    module_source_paths: dict[str, str]
    data_source_paths: dict[str, str]


def yaml_load_file(path: Path) -> dict[str, Any]:
    return common.yaml_load_file(path)


def _expand_vars(s: str) -> str:
    out = s
    for name, default in [("BUCKYOS_ROOT", "/opt/buckyos"), ("BUCKYOS_BUILD_ROOT", str(RESULT_ROOT_DIR))]:
        val = os.environ.get(name, default)
        out = out.replace(f"${{{name}}}", val)
    return os.path.expanduser(out)


def _manifest_install_project(manifest_path: Path, app_key: str) -> dict[str, Any]:
    return common.manifest_install_project(manifest_path, app_key)


def load_app_layout(
    project_yaml_path: Path,
    app_key: str,
    target_override: str | None = None,
) -> AppLayout:
    data = yaml_load_file(project_yaml_path)
    apps = data.get("apps", {}) or {}
    if not isinstance(apps, dict):
        raise ValueError("apps must be a map")
    app_cfg = apps.get(app_key)
    if not isinstance(app_cfg, dict):
        raise ValueError(f"apps.{app_key} missing or invalid")

    base_dir = str(data.get("base_dir", "."))
    project_base = (project_yaml_path.parent / base_dir).resolve()

    rootfs_rel = str(app_cfg.get("rootfs", "rootfs/"))
    source_rootfs = (project_base / rootfs_rel).resolve()

    default_target = str(app_cfg.get("default_target_rootfs", "${BUCKYOS_ROOT}"))
    target_str = target_override if target_override else default_target
    target_rootfs = Path(_expand_vars(target_str)).resolve()

    modules = app_cfg.get("modules", {}) or {}
    data_paths_raw = app_cfg.get("data_paths", []) or []
    clean_paths_raw = app_cfg.get("clean_paths", []) or []
    if not isinstance(modules, dict):
        raise ValueError(f"apps.{app_key}.modules must be a map")
    if not isinstance(data_paths_raw, list):
        raise ValueError(f"apps.{app_key}.data_paths must be a list")
    if not isinstance(clean_paths_raw, list):
        raise ValueError(f"apps.{app_key}.clean_paths must be a list")

    module_paths = [str(p) for p in modules.values()]
    data_paths = [str(p) for p in data_paths_raw]
    clean_paths = [str(p) for p in clean_paths_raw]

    return AppLayout(
        source_rootfs=source_rootfs,
        target_rootfs=target_rootfs,
        module_paths=module_paths,
        data_paths=data_paths,
        clean_paths=clean_paths,
        module_source_paths={rel: str((source_rootfs / rel.strip().lstrip("/")).resolve()) for rel in module_paths},
        data_source_paths={rel: str((source_rootfs / rel.strip().lstrip("/")).resolve()) for rel in data_paths},
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

    return AppLayout(
        source_rootfs=Path(source_rootfs_raw).resolve(),
        target_rootfs=Path(_expand_vars(target_str)).resolve(),
        module_paths=common.item_paths(app_cfg, "module_items", project_key=app_key),
        data_paths=common.item_paths(app_cfg, "data_items", project_key=app_key),
        clean_paths=common.item_paths(app_cfg, "clean_items", project_key=app_key),
        module_source_paths=common.item_source_paths(app_cfg, "module_items", project_key=app_key),
        data_source_paths=common.item_source_paths(app_cfg, "data_items", project_key=app_key),
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


def _ignore_copy_entries(_: str, names: List[str]) -> List[str]:
    return [name for name in names if name in IGNORED_STAGE_NAMES]


def _copytree_filtered(src: Path, dst: Path) -> None:
    shutil.copytree(src, dst, dirs_exist_ok=True, ignore=_ignore_copy_entries)


def _source_path_for(
    layout: AppLayout,
    rel: str,
    *,
    item_kind: str,
    source_root_override: Path | None = None,
) -> Path:
    mapping = layout.module_source_paths if item_kind == "module" else layout.data_source_paths
    return common.source_path_for(
        source_rootfs=layout.source_rootfs,
        rel=rel,
        item_source_paths=mapping,
        source_root_override=source_root_override,
    )


def _stage_buckyos_app_root(*, src_root: Path, dst_root: Path, layout: AppLayout) -> None:
    for rel in layout.module_paths:
        rel_s = common.normalize_item_relpath(rel)
        if not rel_s:
            continue
        src = _source_path_for(layout, rel, item_kind="module", source_root_override=src_root)
        dst = dst_root / rel_s
        if not src.exists():
            raise FileNotFoundError(
                f"module source missing: '{rel}' -> '{src}'. "
                f"Please ensure it exists under the buckyos publish root ({src_root}), "
                "or remove it from apps.buckyos.modules."
            )
        if src.is_dir():
            _copytree_filtered(src, dst)
        else:
            dst.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(src, dst)

    defaults_root = dst_root / BUCKYOS_DEFAULTS_SUBDIR
    for rel in layout.data_paths:
        rel_s = common.normalize_item_relpath(rel)
        if not rel_s:
            continue
        src = _source_path_for(layout, rel, item_kind="data", source_root_override=src_root)
        dst = defaults_root / rel_s
        if not src.exists():
            raise FileNotFoundError(
                f"data_paths source missing: '{rel}' -> '{src}'. "
                f"Please ensure it exists under the buckyos publish root ({src_root}), "
                "or remove it from apps.buckyos.data_paths."
            )
        if src.is_dir():
            _copytree_filtered(src, dst)
        else:
            dst.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(src, dst)


def _normalize_tree_modes(root: Path) -> None:
    if not root.exists():
        return
    for path in [root, *sorted(root.rglob("*"))]:
        if path.is_symlink():
            continue
        if path.is_dir():
            path.chmod(0o755)
        elif path.is_file():
            is_executable = bool(path.stat().st_mode & 0o111)
            path.chmod(0o755 if is_executable else 0o644)


def _resolve_buckyos_src(
    *,
    source_override: Path | None,
    app_publish_dir: Path,
    layout: AppLayout,
    allow_missing: bool = False,
) -> Path:
    candidates: List[Path] = []
    if source_override:
        candidates.append(source_override)
    candidates.append(app_publish_dir / "buckyos")
    candidates.append(app_publish_dir / "publish" / "buckyos")
    candidates.append(layout.source_rootfs)
    for candidate in candidates:
        if candidate.exists():
            return candidate
    if allow_missing:
        return candidates[0]
    raise FileNotFoundError(
        "buckyos source rootfs not found. Tried: "
        + ", ".join(str(candidate) for candidate in candidates)
    )


def _rpm_arch(arch: str) -> str:
    canonical = common.canonical_arch(arch)
    if canonical == "amd64":
        return "x86_64"
    if canonical == "arm64":
        return "aarch64"
    raise ValueError(f"unsupported rpm architecture: {arch}")


def _rpm_token(raw: str, fallback: str) -> str:
    cleaned = re.sub(r"[^A-Za-z0-9._~]+", "_", raw.strip())
    cleaned = cleaned.strip("._")
    return cleaned or fallback


def _rpm_version_release(version: str) -> tuple[str, str]:
    if "+" in version:
        rpm_version_raw, rpm_release_raw = version.split("+", 1)
        return _rpm_token(rpm_version_raw, "0"), _rpm_token(rpm_release_raw, "1")
    return _rpm_token(version, "0"), "1"


def _run(cmd: List[str], dry_run: bool, cwd: Path | None = None) -> None:
    print("+", " ".join(cmd))
    if dry_run:
        return
    subprocess.run(cmd, check=True, cwd=cwd)


LINUX_REQUIRED_DEPS = ("python3", "curl", "openssl", "psmisc")
LINUX_DOCKER_DEPS = ("docker.io", "docker-ce", "moby-engine", "docker-engine")


def _dependency_present(dep_text: str, dep_name: str) -> bool:
    return re.search(rf"(^|[,\s|()]){re.escape(dep_name)}([,\s|()<>:=]|$)", dep_text) is not None


def _verify_linux_dependencies(dep_text: str, failures: List[str], *, package_kind: str) -> None:
    for dep_name in LINUX_REQUIRED_DEPS:
        if not _dependency_present(dep_text, dep_name):
            failures.append(f"{package_kind} dependency missing: {dep_name}")
    if not any(_dependency_present(dep_text, dep_name) for dep_name in LINUX_DOCKER_DEPS):
        failures.append(f"{package_kind} dependency missing Docker provider alternative")


def _linux_payload_allowlist(layout: AppLayout, *, include_systemd_service: bool) -> tuple[List[str], List[str]]:
    root = "opt/buckyos"
    defaults_root = f"{root}/{BUCKYOS_DEFAULTS_SUBDIR}"
    allowed_prefixes: List[str] = []
    allowed_exact = ["opt", root, defaults_root]

    def add_prefix(prefix: str) -> None:
        allowed_prefixes.append(prefix)
        parts = prefix.split("/")[:-1]
        while parts:
            allowed_exact.append("/".join(parts))
            parts = parts[:-1]

    for rel in layout.module_paths:
        rel_s = common.normalize_item_relpath(rel)
        if rel_s:
            add_prefix(f"{root}/{rel_s}")
    for rel in layout.data_paths:
        rel_s = common.normalize_item_relpath(rel)
        if rel_s:
            add_prefix(f"{defaults_root}/{rel_s}")
    if include_systemd_service:
        allowed_exact.extend(
            [
                "etc",
                "etc/systemd",
                "etc/systemd/system",
                "etc/systemd/system/buckyos.service",
            ]
        )
    return allowed_prefixes, allowed_exact


def _verify_linux_component_keys(component_keys: List[str], failures: List[str], *, package_kind: str) -> None:
    if "buckyos" not in component_keys:
        failures.append(f"{package_kind} manifest must include linux component 'buckyos'")
    for forbidden_key in ("BuckyOSApp", "buckycli"):
        if forbidden_key in component_keys:
            failures.append(f"{package_kind} linux package must not expose desktop component '{forbidden_key}'")


def _verify_linux_payload_contract(
    *,
    payload_paths: List[str],
    layout: AppLayout,
    failures: List[str],
    package_kind: str,
    include_systemd_service: bool,
) -> None:
    normalized_paths = [common.normalize_payload_path(path) for path in payload_paths]
    allowed_prefixes, allowed_exact = _linux_payload_allowlist(
        layout,
        include_systemd_service=include_systemd_service,
    )
    unexpected = common.unexpected_payload_paths(
        normalized_paths,
        allowed_prefixes=allowed_prefixes,
        allowed_exact=allowed_exact,
    )
    if unexpected:
        shown = ", ".join(unexpected[:20])
        suffix = f" ... and {len(unexpected) - 20} more" if len(unexpected) > 20 else ""
        failures.append(f"{package_kind} payload contains undeclared paths: {shown}{suffix}")

    for bad_name in ("BuckyOS.app", "buckyosapp.exe"):
        if any(bad_name.lower() in path.lower() for path in normalized_paths):
            failures.append(f"{package_kind} payload must not include desktop app artifact: {bad_name}")

    root = "opt/buckyos"
    defaults_root = f"{root}/{BUCKYOS_DEFAULTS_SUBDIR}"
    for rel in layout.data_paths:
        rel_s = common.normalize_item_relpath(rel)
        if not rel_s:
            continue
        real_prefix = f"{root}/{rel_s}"
        defaults_prefix = f"{defaults_root}/{rel_s}"
        real_present = any(common.payload_path_matches_prefix(path, real_prefix) for path in normalized_paths)
        defaults_present = any(common.payload_path_matches_prefix(path, defaults_prefix) for path in normalized_paths)
        if real_present:
            failures.append(f"data_paths '{rel}' should NOT be in {package_kind} payload at '{real_prefix}'")
        if not defaults_present:
            failures.append(f"data_paths '{rel}' missing from {package_kind} defaults payload at '{defaults_prefix}'")


def _rm_lines(root_var: str, rel_paths: List[str]) -> List[str]:
    out: List[str] = []
    for rel in rel_paths:
        rel_s = common.normalize_item_relpath(rel)
        if rel_s:
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


def _linux_component_keys(project_yaml_path: Path, manifest_path: Path | None) -> List[str]:
    if manifest_path is not None:
        return common.manifest_component_keys(manifest_path, "linux")

    data = yaml_load_file(project_yaml_path)
    publish = data.get("publish", {}) or {}
    linux_pkg = (publish.get("linux_pkg", {}) or {}) if isinstance(publish, dict) else {}
    apps = (linux_pkg.get("apps", {}) or {}) if isinstance(linux_pkg, dict) else {}
    if isinstance(apps, dict) and apps:
        return [str(key) for key in apps.keys()]
    return ["buckyos"]


def _discover_linux_hook(component_key: str, step: str) -> Path | None:
    return common.discover_component_hook(
        scripts_dirs=(RPM_PKG_DIR, DEB_PKG_DIR),
        component_key=component_key,
        step=step,
        extensions=("", ".sh"),
    )


AUTO_BEGIN = "# BEGIN AUTO-GENERATED:"
AUTO_END = "# END AUTO-GENERATED:"


def _replace_marked_block(text: str, block_name: str, new_lines: List[str], indent: str = "") -> str:
    begin = f"{AUTO_BEGIN} {block_name}"
    end = f"{AUTO_END} {block_name}"
    lines = text.splitlines()
    try:
        i0 = next(i for i, line in enumerate(lines) if line.strip() == begin)
        i1 = next(i for i, line in enumerate(lines) if line.strip() == end and i > i0)
    except StopIteration:
        appended = [begin] + [indent + line for line in new_lines] + [end]
        return text.rstrip() + "\n" + "\n".join(appended) + "\n"

    replaced = lines[: i0 + 1] + [indent + line for line in new_lines] + lines[i1:]
    return "\n".join(replaced).rstrip("\n") + "\n"


def _render_linux_hook_text(*, hook_path: Path, component_key: str, step: str, layout: AppLayout) -> str:
    hook_text = hook_path.read_text(encoding="utf-8", errors="ignore")
    if component_key == "buckyos" and step == "preinstall":
        hook_text = _replace_marked_block(hook_text, "modules", _rm_lines("$BUCKYOS_ROOT", layout.module_paths))
    if component_key == "buckyos" and step == "postinstall":
        hook_text = _replace_marked_block(
            hook_text,
            "data_paths",
            _data_copy_lines("$BUCKYOS_ROOT", "$DEFAULTS_DIR", layout.data_paths),
            indent="  ",
        )
    return hook_text.rstrip("\n")


def _escape_spec_body_line(line: str) -> str:
    return line.replace("%", "%%")


def _shell_hook_lines(component_keys: List[str], step: str, *, layout: AppLayout) -> List[str]:
    out: List[str] = []
    for component_key in component_keys:
        hook_path = _discover_linux_hook(component_key, step)
        if hook_path is None:
            continue
        hook_text = _render_linux_hook_text(
            hook_path=hook_path,
            component_key=component_key,
            step=step,
            layout=layout,
        )
        out.extend(
            [
                f'echo "[buckyos] running {component_key}_{step} hook"',
                f"# BEGIN COMPONENT HOOK: {component_key}_{step} ({hook_path})",
                "(",
            ]
        )
        out.extend(_escape_spec_body_line(line) for line in hook_text.rstrip("\n").splitlines())
        out.extend(
            [
                ")",
                f"# END COMPONENT HOOK: {component_key}_{step}",
            ]
        )
    return out or [":"]


def _verify_expected_linux_hooks(
    *,
    script_text: str,
    component_keys: List[str],
    step: str,
    failures: List[str],
    package_kind: str,
) -> None:
    for component_key in component_keys:
        hook_path = _discover_linux_hook(component_key, step)
        if hook_path is None:
            continue
        marker = f"BEGIN COMPONENT HOOK: {component_key}_{step}"
        if marker not in script_text:
            failures.append(f"{package_kind} {step} script missing expected component hook: {hook_path}")


def _scriptlet(lines: List[str]) -> str:
    return "\n".join(lines).rstrip("\n") + "\n"


def _render_template(template_path: Path, replacements: dict[str, str]) -> str:
    text = template_path.read_text(encoding="utf-8")
    for key, value in replacements.items():
        text = text.replace("{{" + key + "}}", value)
    unresolved = sorted(set(re.findall(r"{{[A-Za-z0-9_]+}}", text)))
    if unresolved:
        raise ValueError(f"unresolved rpm spec template placeholders: {', '.join(unresolved)}")
    return text


def _render_spec(
    *,
    rpm_version: str,
    rpm_release: str,
    payload_tree: Path,
    layout: AppLayout,
    component_keys: List[str],
) -> str:
    pre_lines = [
        "set -e",
        *_shell_hook_lines(component_keys, "preinstall", layout=layout),
        "exit 0",
    ]

    post_lines = [
        "set -e",
        *_shell_hook_lines(component_keys, "postinstall", layout=layout),
        "exit 0",
    ]

    preun_lines = [
        'if [ "$1" -eq 0 ]; then',
        "  systemctl disable --now buckyos.service >/dev/null 2>&1 || true",
        "fi",
        "exit 0",
    ]
    postun_lines = [
        "systemctl daemon-reload >/dev/null 2>&1 || true",
        "exit 0",
    ]

    payload_arg = shlex.quote(str(payload_tree))
    return _render_template(
        RPM_SPEC_TEMPLATE,
        {
            "rpm_version": rpm_version,
            "rpm_release": rpm_release,
            "payload_tree": payload_arg,
            "pre_script": _scriptlet(pre_lines),
            "post_script": _scriptlet(post_lines),
            "preun_script": _scriptlet(preun_lines),
            "postun_script": _scriptlet(postun_lines),
        },
    )


def _find_built_rpm(rpmbuild_root: Path, rpm_version: str, rpm_release: str) -> Path:
    rpm_root = rpmbuild_root / "RPMS"
    candidates = sorted(rpm_root.glob(f"**/buckyos-{rpm_version}-{rpm_release}*.rpm"))
    if not candidates:
        candidates = sorted(rpm_root.glob("**/*.rpm"))
    if not candidates:
        raise FileNotFoundError(f"rpmbuild did not produce an rpm under {rpm_root}")
    return candidates[0]


def build_rpm(
    *,
    architecture: str,
    version: str,
    project_yaml_path: Path,
    manifest_path: Path | None,
    app_publish_dir: Path,
    out_dir: Path,
    source_rootfs: Path | None = None,
    dry_run: bool = False,
) -> Path:
    canonical_arch = common.canonical_arch(architecture)
    rpm_architecture = _rpm_arch(canonical_arch)
    rpm_version, rpm_release = _rpm_version_release(version)

    work_root = TMP_INSTALL_DIR / "distbuild" / rpm_architecture
    rpmbuild_root = work_root / "rpmbuild"
    payload_tree = work_root / "payload"
    spec_path = rpmbuild_root / "SPECS" / "buckyos.spec"

    if work_root.exists() and not dry_run:
        shutil.rmtree(work_root, ignore_errors=True)

    layout = resolve_app_layout(
        app_key="buckyos",
        project_yaml_path=project_yaml_path,
        manifest_path=manifest_path,
        target_override="/opt/buckyos",
    )
    src_root = _resolve_buckyos_src(
        source_override=source_rootfs,
        app_publish_dir=app_publish_dir,
        layout=layout,
        allow_missing=dry_run,
    )
    payload_root = payload_tree / "opt" / "buckyos"
    component_keys = _linux_component_keys(project_yaml_path, manifest_path)

    if dry_run:
        print(f"[dry-run] rpm Version={rpm_version} Release={rpm_release} TargetArch={rpm_architecture}")
        print(f"[dry-run] stage buckyos: {src_root} -> {payload_root}")
    else:
        for subdir in ("BUILD", "RPMS", "SOURCES", "SPECS", "SRPMS", "BUILDROOT"):
            (rpmbuild_root / subdir).mkdir(parents=True, exist_ok=True)
        payload_root.mkdir(parents=True, exist_ok=True)
        _stage_buckyos_app_root(src_root=src_root, dst_root=payload_root, layout=layout)
        _normalize_tree_modes(payload_tree / "opt")

    spec_text = _render_spec(
        rpm_version=rpm_version,
        rpm_release=rpm_release,
        payload_tree=payload_tree,
        layout=layout,
        component_keys=component_keys,
    )
    build_cmd = [
        "rpmbuild",
        "--define",
        f"_topdir {rpmbuild_root}",
        "--define",
        "_enable_debug_packages 0",
        "--target",
        rpm_architecture,
        "-bb",
        str(spec_path),
    ]

    out_dir.mkdir(parents=True, exist_ok=True)
    out_rpm = out_dir / common.package_filename(
        platform_key="linux",
        architecture=canonical_arch,
        version=version,
        package_format="rpm",
    )

    if dry_run:
        print(f"[dry-run] write spec: {spec_path}")
        print(f"[dry-run] {' '.join(build_cmd)}")
        return out_rpm

    if shutil.which("rpmbuild") is None:
        raise FileNotFoundError("rpmbuild not found. Install rpm-build before building Fedora rpm packages.")

    spec_path.parent.mkdir(parents=True, exist_ok=True)
    spec_path.write_text(spec_text, encoding="utf-8")
    _run(build_cmd, dry_run=False, cwd=work_root)
    built_rpm = _find_built_rpm(rpmbuild_root, rpm_version, rpm_release)
    shutil.copy2(built_rpm, out_rpm)
    return out_rpm


def _rpm_query(pkg_path: Path, args: List[str]) -> str:
    result = subprocess.run(
        ["rpm", *args, str(pkg_path)],
        check=True,
        capture_output=True,
        text=True,
    )
    return result.stdout


def verify_pkg(
    *,
    pkg_path: Path,
    project_yaml_path: Path,
    manifest_path: Path | None = None,
) -> int:
    if not pkg_path.exists():
        print(f"VERIFY FAIL: .rpm not found: {pkg_path}")
        return 1
    if pkg_path.suffix != ".rpm":
        print(f"VERIFY FAIL: expected .rpm file: {pkg_path}")
        return 1

    failures: List[str] = []
    layout = resolve_app_layout(
        app_key="buckyos",
        project_yaml_path=project_yaml_path,
        manifest_path=manifest_path,
        target_override="/opt/buckyos",
    )
    component_keys = _linux_component_keys(project_yaml_path, manifest_path)
    _verify_linux_component_keys(component_keys, failures, package_kind="rpm")

    rpm_cmd = shutil.which("rpm")
    if rpm_cmd is None:
        print("[verify] Warning: rpm command not found, skipping rpm metadata inspection")
    else:
        try:
            metadata = _rpm_query(pkg_path, ["-qp", "--qf", "%{NAME}\n%{VERSION}\n%{RELEASE}\n%{ARCH}\n"])
            meta_lines = metadata.splitlines()
            name = meta_lines[0] if len(meta_lines) > 0 else ""
            version = meta_lines[1] if len(meta_lines) > 1 else ""
            release = meta_lines[2] if len(meta_lines) > 2 else ""
            arch = meta_lines[3] if len(meta_lines) > 3 else ""
            if name != "buckyos":
                failures.append(f"rpm Name should be buckyos, got {name!r}")
            if not version:
                failures.append("rpm Version is empty")
            if not release:
                failures.append("rpm Release is empty")
            if arch not in ("x86_64", "aarch64"):
                failures.append(f"rpm Arch should be x86_64|aarch64, got {arch!r}")
        except subprocess.CalledProcessError as err:
            failures.append(err.stderr.strip() or "rpm metadata query failed")

        try:
            requires_text = _rpm_query(pkg_path, ["-qp", "--requires"])
            _verify_linux_dependencies(requires_text, failures, package_kind="rpm")
        except subprocess.CalledProcessError as err:
            failures.append(err.stderr.strip() or "rpm requires query failed")

        try:
            payload_paths = _rpm_query(pkg_path, ["-qlp"]).splitlines()
            _verify_linux_payload_contract(
                payload_paths=payload_paths,
                layout=layout,
                failures=failures,
                package_kind="rpm",
                include_systemd_service=False,
            )
        except subprocess.CalledProcessError as err:
            failures.append(err.stderr.strip() or "rpm payload listing failed")

        try:
            scripts_text = _rpm_query(pkg_path, ["-qp", "--scripts"])
            required_script_snippets = [
                'rm -rf "$BUCKYOS_ROOT/bin/"',
                "systemctl stop buckyos.service >/dev/null 2>&1 || true",
                "systemctl daemon-reload",
                "systemctl enable buckyos.service",
                "systemctl start buckyos.service",
                "systemctl disable --now buckyos.service >/dev/null 2>&1 || true",
            ]
            for snippet in required_script_snippets:
                if snippet not in scripts_text:
                    failures.append(f"rpm scriptlets missing required snippet: {snippet}")
            _verify_expected_linux_hooks(
                script_text=scripts_text,
                component_keys=component_keys,
                step="preinstall",
                failures=failures,
                package_kind="rpm",
            )
            _verify_expected_linux_hooks(
                script_text=scripts_text,
                component_keys=component_keys,
                step="postinstall",
                failures=failures,
                package_kind="rpm",
            )
        except subprocess.CalledProcessError as err:
            failures.append(err.stderr.strip() or "rpm scriptlet query failed")

    file_size = pkg_path.stat().st_size
    print(f"[verify] Package size: {file_size / (1024 * 1024):.2f} MB")
    if file_size <= 0:
        failures.append("Package is empty")

    if failures:
        print("VERIFY FAIL:")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("VERIFY PASS")
    return 0


def _legacy_build_main(argv: List[str]) -> int:
    subcommands = {"build-pkg", "verify-pkg"}
    if len(argv) == 3 and (argv[1] not in subcommands) and (not argv[1].startswith("-")):
        out_rpm = build_rpm(
            architecture=argv[1],
            version=argv[2],
            project_yaml_path=PROJECT_YAML,
            manifest_path=None,
            app_publish_dir=RESULT_ROOT_DIR,
            out_dir=Path.cwd() / "publish",
            dry_run=False,
        )
        print(f"make_local_rpm.py completed: {out_rpm}")
        return 0
    return 2


def main(argv: List[str]) -> int:
    legacy_rc = _legacy_build_main(argv)
    if legacy_rc != 2:
        return legacy_rc

    parser = argparse.ArgumentParser(prog="make_local_rpm.py")
    sub = parser.add_subparsers(dest="cmd", required=True)

    p_build = sub.add_parser("build-pkg", help="Build Fedora .rpm package")
    p_build.add_argument("architecture", help="amd64|arm64 (x86_64/aarch64 accepted)")
    p_build.add_argument("version", help="External package version string")
    p_build.add_argument("--project", default=str(PROJECT_YAML), help="Path to bucky_project.yaml")
    p_build.add_argument("--manifest", default=None, help="Path to generated project manifest JSON")
    p_build.add_argument(
        "--app-publish-dir",
        default=str(RESULT_ROOT_DIR),
        help="Base directory to resolve built rootfs (default: $BUCKYOS_BUILD_ROOT)",
    )
    p_build.add_argument(
        "--out-dir",
        default=str(Path.cwd() / "publish"),
        help='Output directory for the final .rpm (default: "./publish")',
    )
    p_build.add_argument("--source-rootfs", default=None, help="Override source rootfs path for buckyos payload")
    p_build.add_argument("--dry-run", action="store_true", help="Print commands without executing them")

    p_verify = sub.add_parser("verify-pkg", help="Verify a built Fedora .rpm offline")
    p_verify.add_argument("pkg", help="Path to .rpm")
    p_verify.add_argument("--project", default=str(PROJECT_YAML), help="Path to bucky_project.yaml")
    p_verify.add_argument("--manifest", default=None, help="Path to generated project manifest JSON")

    args = parser.parse_args(argv[1:])

    if args.cmd == "build-pkg":
        out_rpm = build_rpm(
            architecture=args.architecture,
            version=args.version,
            project_yaml_path=Path(args.project),
            manifest_path=Path(args.manifest).resolve() if args.manifest else None,
            app_publish_dir=Path(args.app_publish_dir),
            out_dir=Path(args.out_dir),
            source_rootfs=Path(args.source_rootfs).resolve() if args.source_rootfs else None,
            dry_run=bool(args.dry_run),
        )
        print(f"rpm built: {out_rpm}")
        return 0

    if args.cmd == "verify-pkg":
        return verify_pkg(
            pkg_path=Path(args.pkg).expanduser().resolve(),
            project_yaml_path=Path(args.project),
            manifest_path=Path(args.manifest).resolve() if args.manifest else None,
        )

    return 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
