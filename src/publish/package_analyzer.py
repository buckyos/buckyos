#!/usr/bin/env python3
"""Analyze BuckyOS package artifacts into deterministic JSON reports."""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import tomllib
import xml.etree.ElementTree as ET
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterable


PACKAGE_SUFFIXES = {".deb", ".rpm", ".pkg", ".exe"}
TEXT_SCRIPT_SUFFIXES = {
    "",
    ".sh",
    ".ps1",
    ".bat",
    ".cmd",
    ".vbs",
    ".nsi",
    ".xml",
    ".plist",
    ".json",
    ".yaml",
    ".yml",
    ".toml",
    ".service",
}
INSTALLER_SCRIPT_NAMES = {
    "preinst",
    "postinst",
    "prerm",
    "postrm",
    "config",
    "triggers",
    "preinstall",
    "postinstall",
    "preuninstall",
    "postuninstall",
}
HASH_CHUNK_SIZE = 1024 * 1024
DEFAULT_CI_CONFIG = Path("/root/work/pve-pack-system/config/pve-build.toml")
WINDOWS_NSIS_RELATIVE_PATH = "win-installer/distbuild/installer.nsi"
PROCESS_DETAIL_LIMIT = 8192
LINUX_REQUIRED_TOOLS = ("7z", "bzip2", "cpio", "dpkg-deb", "gzip", "rpm", "rpm2cpio", "xar", "xz", "zstd")
OPTIONAL_TOOL_PACKAGES = {
    "ssh": "openssh-client",
}
APT_PACKAGE_BY_TOOL = {
    "7z": "7zip",
    "bzip2": "bzip2",
    "cpio": "cpio",
    "dpkg-deb": "dpkg",
    "gzip": "gzip",
    "rpm": "rpm",
    "rpm2cpio": "rpm2cpio",
    "xar": "xar",
    "xz": "xz-utils",
    "zstd": "zstd",
    **OPTIONAL_TOOL_PACKAGES,
}
APT_TOOL_NOTES = {
    "7z": "On older Ubuntu/Debian releases, install p7zip-full if the 7zip package is unavailable.",
    "rpm2cpio": "If rpm2cpio is not packaged separately on this system, install rpm.",
}


@dataclass(frozen=True)
class ToolStatus:
    name: str
    path: str | None

    @property
    def available(self) -> bool:
        return self.path is not None

    def to_json(self) -> dict[str, Any]:
        return {"name": self.name, "available": self.available, "path": self.path}


@dataclass(frozen=True)
class ExternalScript:
    name: str
    kind: str
    source: str
    data: bytes


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as f:
        while True:
            chunk = f.read(HASH_CHUNK_SIZE)
            if not chunk:
                break
            digest.update(chunk)
    return digest.hexdigest()


def text_digest_from_bytes(data: bytes, *, limit_bytes: int) -> dict[str, Any]:
    digest = hashlib.sha256(data).hexdigest()
    out: dict[str, Any] = {
        "sha256": digest,
        "size_bytes": len(data),
        "truncated": len(data) > limit_bytes,
    }
    sample = data[:limit_bytes]
    try:
        out["text"] = sample.decode("utf-8")
    except UnicodeDecodeError:
        out["text"] = sample.decode("utf-8", errors="replace")
        out["decode_errors"] = True
    return out


def read_text_digest(path: Path, *, limit_bytes: int) -> dict[str, Any]:
    data = path.read_bytes()
    return text_digest_from_bytes(data, limit_bytes=limit_bytes)


def which(name: str) -> ToolStatus:
    return ToolStatus(name=name, path=shutil.which(name))


def tool_map(names: Iterable[str]) -> dict[str, ToolStatus]:
    return {name: which(name) for name in names}


def missing_tool_names(tools: dict[str, ToolStatus], names: Iterable[str]) -> list[str]:
    return [name for name in names if not tools[name].available]


def apt_packages_for_tools(tool_names: Iterable[str]) -> list[str]:
    packages = [APT_PACKAGE_BY_TOOL.get(name, name) for name in tool_names]
    return sorted(dict.fromkeys(packages))


def tool_install_hint(tool_names: Iterable[str]) -> dict[str, Any]:
    missing = list(tool_names)
    packages = apt_packages_for_tools(missing)
    hint: dict[str, Any] = {
        "apt_packages": packages,
        "commands": [
            "sudo apt update",
            "sudo apt install -y " + " ".join(packages),
        ] if packages else [],
    }
    notes = {name: APT_TOOL_NOTES[name] for name in missing if name in APT_TOOL_NOTES}
    if notes:
        hint["notes"] = notes
    return hint


def format_missing_tools_error(tool_names: Iterable[str]) -> str:
    missing = list(tool_names)
    hint = tool_install_hint(missing)
    lines = ["missing required analyzer tools:"]
    for name in missing:
        package = APT_PACKAGE_BY_TOOL.get(name, name)
        lines.append(f"  - {name} (apt package: {package})")
    if hint["commands"]:
        lines.append("")
        lines.append("install them with:")
        lines.extend(f"  {cmd}" for cmd in hint["commands"])
    for name, note in hint.get("notes", {}).items():
        lines.append(f"note for {name}: {note}")
    return "\n".join(lines)


def required_tool_names(*, include_windows_ssh: bool = False) -> list[str]:
    names = list(LINUX_REQUIRED_TOOLS)
    if include_windows_ssh:
        names.append("ssh")
    return sorted(dict.fromkeys(names))


def add_error(record: dict[str, Any], message: str) -> None:
    record.setdefault("errors", []).append(message)


def add_warning(record: dict[str, Any], message: str) -> None:
    record.setdefault("warnings", []).append(message)


def run_capture(cmd: list[str], *, check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(cmd, check=check, capture_output=True, text=True, encoding="utf-8", errors="replace")


def run_bytes(cmd: list[str], *, check: bool = True) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(cmd, check=check, capture_output=True)


def limit_process_detail(detail: str) -> str:
    detail = detail.strip()
    if len(detail) <= PROCESS_DETAIL_LIMIT:
        return detail
    omitted = len(detail) - PROCESS_DETAIL_LIMIT
    return f"{detail[:PROCESS_DETAIL_LIMIT]}\n... truncated {omitted} chars"


def ps_single_quote(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


def split_ssh_script_spec(spec: str) -> tuple[str, str]:
    if ":" not in spec:
        raise ValueError("SSH script spec must be USER@HOST:PATH")
    remote, remote_path = spec.split(":", 1)
    if not remote.strip() or not remote_path.strip():
        raise ValueError("SSH script spec must be USER@HOST:PATH")
    return remote.strip(), remote_path.strip()


def fetch_remote_text(remote: str, remote_path: str, *, ssh_args: list[str]) -> bytes:
    ps_script = (
        "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; "
        f"Get-Content -Raw -Encoding UTF8 -LiteralPath {ps_single_quote(remote_path)}"
    )
    remote_cmd = (
        "powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "
        + shlex_quote(ps_script)
    )
    result = run_bytes(["ssh", *ssh_args, remote, remote_cmd], check=False)
    if result.returncode != 0:
        detail = result.stderr.decode("utf-8", errors="replace").strip() or result.stdout.decode(
            "utf-8", errors="replace"
        ).strip()
        raise RuntimeError(detail or f"ssh failed with exit code {result.returncode}")
    return result.stdout


def shlex_quote(value: str) -> str:
    # Kept local to avoid importing shlex in hot analysis paths where it is unused.
    import shlex

    return shlex.quote(value)


def load_windows_nsis_from_ci(config_path: Path) -> ExternalScript:
    data = tomllib.loads(config_path.read_text(encoding="utf-8"))
    windows = (data.get("guests", {}) or {}).get("windows", {}) or {}
    ssh_user = str(windows.get("ssh_user", "")).strip()
    ssh_host = str(windows.get("ssh_host", "")).strip()
    build_root = str(windows.get("build_root", "")).strip()
    if not ssh_user or not ssh_host or not build_root:
        raise RuntimeError(f"missing guests.windows ssh_user/ssh_host/build_root in {config_path}")

    remote = f"{ssh_user}@{ssh_host}"
    remote_path = build_root.rstrip("/\\") + "/" + WINDOWS_NSIS_RELATIVE_PATH
    ssh_args = ["-o", "BatchMode=yes"]
    configured_args = data.get("ssh_common_args", []) or []
    if not isinstance(configured_args, list):
        raise RuntimeError(f"ssh_common_args must be a list in {config_path}")
    ssh_args.extend(str(arg) for arg in configured_args)
    payload = fetch_remote_text(remote, remote_path, ssh_args=ssh_args)
    return ExternalScript(
        name="installer.nsi",
        kind="windows-nsis-script",
        source=f"ci:{remote}:{remote_path}",
        data=payload,
    )


def load_windows_nsis_from_ssh(spec: str) -> ExternalScript:
    remote, remote_path = split_ssh_script_spec(spec)
    payload = fetch_remote_text(remote, remote_path, ssh_args=["-o", "BatchMode=yes"])
    return ExternalScript(
        name=Path(remote_path).name or "installer.nsi",
        kind="windows-nsis-script",
        source=f"ssh:{remote}:{remote_path}",
        data=payload,
    )


def load_windows_nsis_from_local(path: Path) -> ExternalScript:
    local = path.expanduser().resolve()
    return ExternalScript(
        name=local.name,
        kind="windows-nsis-script",
        source=f"file:{local}",
        data=local.read_bytes(),
    )


def normalize_relpath(path: Path) -> str:
    return path.as_posix().lstrip("./").lstrip("/")


def file_entry(root: Path, path: Path, *, include_hashes: bool) -> dict[str, Any]:
    rel = normalize_relpath(path.relative_to(root))
    try:
        st = path.lstat()
    except FileNotFoundError:
        return {"path": rel, "type": "missing"}

    mode = stat.S_IMODE(st.st_mode)
    entry: dict[str, Any] = {
        "path": rel,
        "mode": oct(mode),
    }
    if path.is_symlink():
        entry["type"] = "symlink"
        entry["link_target"] = os.readlink(path)
    elif path.is_dir():
        entry["type"] = "dir"
    elif path.is_file():
        entry["type"] = "file"
        entry["size_bytes"] = st.st_size
        if include_hashes:
            entry["sha256"] = sha256_file(path)
    else:
        entry["type"] = "other"
    return entry


def tree_entries(root: Path, *, include_hashes: bool) -> list[dict[str, Any]]:
    if not root.exists():
        return []
    return [
        file_entry(root, path, include_hashes=include_hashes)
        for path in sorted(root.rglob("*"), key=lambda p: p.as_posix())
    ]


def collect_scripts(root: Path, *, limit_bytes: int) -> dict[str, Any]:
    scripts: dict[str, Any] = {}
    if not root.exists():
        return scripts
    for path in sorted(root.rglob("*"), key=lambda p: p.as_posix()):
        if not path.is_file():
            continue
        suffix = path.suffix.lower()
        if suffix not in TEXT_SCRIPT_SUFFIXES:
            continue
        rel = normalize_relpath(path.relative_to(root))
        name = path.name.lower()
        in_script_dir = any(part.lower() in {"scripts", "debian", "debian_binary"} for part in path.parts)
        has_script_suffix = suffix in {
            ".sh",
            ".ps1",
            ".bat",
            ".cmd",
            ".vbs",
            ".nsi",
            ".plist",
            ".service",
        }
        if name in INSTALLER_SCRIPT_NAMES or in_script_dir or has_script_suffix:
            scripts[rel] = read_text_digest(path, limit_bytes=limit_bytes)
    return scripts


def phase_from_script_name(name: str) -> str | None:
    normalized = name.lower().replace("-", "_")
    mapping = {
        "preinst": "preinstall",
        "preinstall": "preinstall",
        "postinst": "postinstall",
        "postinstall": "postinstall",
        "prerm": "preuninstall",
        "preuninstall": "preuninstall",
        "postrm": "postuninstall",
        "postuninstall": "postuninstall",
    }
    if normalized in mapping:
        return mapping[normalized]
    for key, value in mapping.items():
        if normalized.endswith(f"_{key}") or normalized.endswith(f"/{key}"):
            return value
    return None


def installer_script_entry(
    *,
    name: str,
    kind: str,
    source: str,
    digest: dict[str, Any],
    path: str | None = None,
    phase: str | None = None,
) -> dict[str, Any]:
    entry = {
        "name": name,
        "kind": kind,
        "source": source,
        **digest,
    }
    if path is not None:
        entry["path"] = path
    if phase is not None:
        entry["phase"] = phase
    return entry


def append_installer_script(record: dict[str, Any], entry: dict[str, Any]) -> None:
    record["analysis"].setdefault("installer_scripts", []).append(entry)


def append_installer_scripts_from_mapping(
    record: dict[str, Any],
    scripts: dict[str, Any],
    *,
    kind: str,
    source: str,
) -> None:
    for rel, digest in sorted(scripts.items()):
        append_installer_script(
            record,
            installer_script_entry(
                name=Path(rel).name,
                path=rel,
                kind=kind,
                source=source,
                digest=digest,
                phase=phase_from_script_name(rel),
            ),
        )


def append_external_scripts(
    record: dict[str, Any],
    scripts: Iterable[ExternalScript],
    *,
    script_text_limit: int,
) -> None:
    for script in scripts:
        append_installer_script(
            record,
            installer_script_entry(
                name=script.name,
                kind=script.kind,
                source=script.source,
                digest=text_digest_from_bytes(script.data, limit_bytes=script_text_limit),
            ),
        )


def infer_from_filename(path: Path) -> dict[str, Any]:
    name = path.name
    suffix = path.suffix.lower().lstrip(".")
    patterns = [
        (
            re.compile(r"^(?P<project>buckyos)-linux-(?P<arch>amd64|arm64|aarch64)-(?P<version>.+)\.(?P<format>deb|rpm)$"),
            "linux",
        ),
        (
            re.compile(r"^(?P<project>buckyos)-(?:macos|apple)-(?P<arch>amd64|arm64|aarch64)-(?P<version>.+)\.pkg$"),
            "macos",
        ),
        (
            re.compile(r"^(?P<project>buckyos)-(?:windows|win)-(?P<arch>amd64|arm64|aarch64)-(?P<version>.+)\.exe$"),
            "windows",
        ),
    ]
    for regex, platform_name in patterns:
        match = regex.match(name)
        if match:
            out = match.groupdict()
            out["platform"] = platform_name
            out["format"] = out.get("format") or suffix
            return out
    return {"format": suffix or "unknown"}


def base_package_record(path: Path, *, include_hashes: bool) -> dict[str, Any]:
    st = path.stat()
    suffix = path.suffix.lower()
    record: dict[str, Any] = {
        "name": path.name,
        "format": suffix.lstrip(".") or "unknown",
        "size_bytes": st.st_size,
        "inferred": infer_from_filename(path),
        "analysis": {},
    }
    if include_hashes:
        record["sha256"] = sha256_file(path)
    return record


def analyze_deb(
    path: Path,
    record: dict[str, Any],
    *,
    tools: dict[str, ToolStatus],
    include_hashes: bool,
    script_text_limit: int,
) -> None:
    required_tools = ["dpkg-deb"]
    missing = missing_tool_names(tools, required_tools)
    if missing:
        add_error(record, "deb payload inspection skipped because dpkg-deb is missing")
        return

    fields: dict[str, str] = {}
    for field in ("Package", "Version", "Architecture", "Depends", "Maintainer", "Description"):
        result = run_capture(["dpkg-deb", "-f", str(path), field], check=False)
        if result.returncode == 0:
            fields[field.lower()] = result.stdout.strip()
    record["analysis"]["metadata"] = fields

    with tempfile.TemporaryDirectory(prefix="buckyos-analyze-deb-") as td:
        work = Path(td)
        payload_root = work / "payload"
        control_root = work / "control"
        payload_root.mkdir()
        control_root.mkdir()
        payload_result = run_capture(["dpkg-deb", "-x", str(path), str(payload_root)], check=False)
        if payload_result.returncode != 0:
            add_error(record, payload_result.stderr.strip() or "dpkg-deb -x failed")
        else:
            record["analysis"]["payload"] = {
                "files": tree_entries(payload_root, include_hashes=include_hashes),
            }
        control_result = run_capture(["dpkg-deb", "-e", str(path), str(control_root)], check=False)
        if control_result.returncode != 0:
            add_warning(record, control_result.stderr.strip() or "dpkg-deb -e failed")
        else:
            record["analysis"]["control_files"] = tree_entries(control_root, include_hashes=include_hashes)
            scripts = collect_scripts(control_root, limit_bytes=script_text_limit)
            record["analysis"]["scripts"] = scripts
            append_installer_scripts_from_mapping(record, scripts, kind="deb-control-script", source="deb-control")


def extract_rpm_payload(path: Path, out_dir: Path) -> tuple[int, str]:
    rpm2cpio = subprocess.Popen(["rpm2cpio", str(path)], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    assert rpm2cpio.stdout is not None
    cpio = subprocess.run(
        ["cpio", "-idm", "--quiet", "--no-preserve-owner"],
        cwd=out_dir,
        stdin=rpm2cpio.stdout,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    rpm2cpio.stdout.close()
    _, rpm2cpio_err = rpm2cpio.communicate()
    if rpm2cpio.returncode != 0:
        return rpm2cpio.returncode or 1, limit_process_detail(rpm2cpio_err.decode("utf-8", errors="replace"))
    if cpio.returncode != 0:
        return cpio.returncode, limit_process_detail(cpio.stderr)
    return 0, ""


def analyze_rpm(
    path: Path,
    record: dict[str, Any],
    *,
    tools: dict[str, ToolStatus],
    include_hashes: bool,
    script_text_limit: int,
) -> None:
    required_tools = ["rpm", "rpm2cpio", "cpio"]
    missing = missing_tool_names(tools, required_tools)
    if missing:
        add_error(record, "rpm payload inspection skipped because required rpm tools are missing")
        return

    qf = "%{NAME}\n%{VERSION}\n%{RELEASE}\n%{ARCH}\n%{SUMMARY}\n%{LICENSE}\n"
    result = run_capture(["rpm", "-qp", "--qf", qf, str(path)], check=False)
    if result.returncode == 0:
        lines = result.stdout.splitlines()
        record["analysis"]["metadata"] = {
            "name": lines[0] if len(lines) > 0 else "",
            "version": lines[1] if len(lines) > 1 else "",
            "release": lines[2] if len(lines) > 2 else "",
            "arch": lines[3] if len(lines) > 3 else "",
            "summary": lines[4] if len(lines) > 4 else "",
            "license": lines[5] if len(lines) > 5 else "",
        }
    else:
        add_error(record, result.stderr.strip() or "rpm metadata query failed")

    requires = run_capture(["rpm", "-qp", "--requires", str(path)], check=False)
    if requires.returncode == 0:
        record["analysis"]["requires"] = sorted(line for line in requires.stdout.splitlines() if line.strip())

    scripts = run_capture(["rpm", "-qp", "--scripts", str(path)], check=False)
    if scripts.returncode == 0:
        data = scripts.stdout.encode("utf-8")
        digest = text_digest_from_bytes(data, limit_bytes=script_text_limit)
        record["analysis"]["scripts"] = {
            "rpm_scriptlets": digest
        }
        append_installer_script(
            record,
            installer_script_entry(
                name="rpm_scriptlets",
                kind="rpm-scriptlets",
                source="rpm-query",
                digest=digest,
            ),
        )

    with tempfile.TemporaryDirectory(prefix="buckyos-analyze-rpm-") as td:
        payload_root = Path(td) / "payload"
        payload_root.mkdir()
        rc, detail = extract_rpm_payload(path, payload_root)
        if rc != 0:
            add_error(record, detail or "rpm payload extraction failed")
        else:
            record["analysis"]["payload"] = {
                "files": tree_entries(payload_root, include_hashes=include_hashes),
            }


def detect_payload_compression(payload_path: Path) -> str:
    with payload_path.open("rb") as f:
        magic = f.read(8)
    if magic.startswith(b"\x1f\x8b"):
        return "gzip"
    if magic.startswith(b"\xfd7zXZ\x00"):
        return "xz"
    if magic.startswith(b"BZh"):
        return "bzip2"
    if magic.startswith(b"\x28\xb5\x2f\xfd"):
        return "zstd"
    return "raw"


def payload_decompress_command(compression: str, payload_path: Path) -> list[str] | None:
    commands = {
        "gzip": ["gzip", "-dc", str(payload_path)],
        "xz": ["xz", "-dc", str(payload_path)],
        "bzip2": ["bzip2", "-dc", str(payload_path)],
        "zstd": ["zstd", "-dcq", str(payload_path)],
    }
    return commands.get(compression)


def run_cpio_extract(
    out_dir: Path,
    *,
    stdin: Any,
) -> tuple[int, str]:
    cpio = subprocess.run(
        ["cpio", "-idm", "--quiet", "--no-preserve-owner"],
        cwd=out_dir,
        stdin=stdin,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    if cpio.returncode != 0:
        return cpio.returncode, limit_process_detail(cpio.stderr)
    return 0, ""


def extract_payload_cpio(payload_path: Path, out_dir: Path, *, tools: dict[str, ToolStatus]) -> tuple[int, str]:
    if not tools["cpio"].available:
        return 1, "missing cpio"

    compression = detect_payload_compression(payload_path)
    decompress_cmd = payload_decompress_command(compression, payload_path)
    if decompress_cmd is None:
        with payload_path.open("rb") as raw:
            return run_cpio_extract(out_dir, stdin=raw)

    tool_name = decompress_cmd[0]
    tool_status = tools.get(tool_name) or which(tool_name)
    if not tool_status.available:
        return 1, f"missing {tool_name}"

    decompressor = subprocess.Popen(decompress_cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    assert decompressor.stdout is not None
    rc, detail = run_cpio_extract(out_dir, stdin=decompressor.stdout)
    decompressor.stdout.close()
    _, decompressor_err = decompressor.communicate()
    decompressor_detail = limit_process_detail(decompressor_err.decode("utf-8", errors="replace"))
    if rc != 0:
        if decompressor.returncode != 0 and decompressor_detail:
            detail = f"{detail}\n{tool_name}: {decompressor_detail}" if detail else decompressor_detail
        return rc, detail
    if decompressor.returncode != 0:
        detail = f"{tool_name} failed: {decompressor_detail}" if decompressor_detail else f"{tool_name} failed"
        return decompressor.returncode or 1, detail
    return 0, ""


def parse_package_info(path: Path) -> dict[str, Any]:
    try:
        root = ET.fromstring(path.read_text(encoding="utf-8", errors="ignore"))
    except Exception as exc:
        return {"parse_error": str(exc)}
    return {
        "identifier": root.attrib.get("identifier", ""),
        "version": root.attrib.get("version", ""),
        "install_location": root.attrib.get("install-location", ""),
        "auth": root.attrib.get("auth", ""),
    }


def pkg_component_workdirs(expanded: Path, work_root: Path, record: dict[str, Any]) -> list[tuple[str, Path]]:
    components: list[tuple[str, Path]] = []
    candidates = sorted([p for p in expanded.iterdir() if p.suffix == ".pkg"], key=lambda p: p.name)
    for candidate in candidates:
        if candidate.is_dir():
            components.append((candidate.name, candidate))
            continue
        if not candidate.is_file():
            continue

        component_root = work_root / candidate.name
        component_root.mkdir(parents=True, exist_ok=True)
        result = run_capture(["xar", "-xf", str(candidate), "-C", str(component_root)], check=False)
        if result.returncode != 0:
            detail = result.stderr.strip() or result.stdout.strip() or "xar extraction failed"
            add_error(record, f"{candidate.name}: {detail}")
            continue
        components.append((candidate.name, component_root))
    return components


def analyze_pkg_with_xar(
    path: Path,
    record: dict[str, Any],
    *,
    tools: dict[str, ToolStatus],
    include_hashes: bool,
    script_text_limit: int,
) -> None:
    with tempfile.TemporaryDirectory(prefix="buckyos-analyze-pkg-") as td:
        expanded = Path(td) / "expanded"
        expanded.mkdir()
        result = run_capture(["xar", "-xf", str(path), "-C", str(expanded)], check=False)
        if result.returncode != 0:
            add_error(record, result.stderr.strip() or "xar extraction failed")
            return

        record["analysis"]["expanded_tree"] = tree_entries(expanded, include_hashes=include_hashes)

        dist = expanded / "Distribution"
        if dist.exists():
            distribution_digest = read_text_digest(dist, limit_bytes=script_text_limit)
            record["analysis"]["distribution"] = distribution_digest
            append_installer_script(
                record,
                installer_script_entry(
                    name="Distribution",
                    path="Distribution",
                    kind="macos-distribution",
                    source="pkg-xar",
                    digest=distribution_digest,
                ),
            )

        components: list[dict[str, Any]] = []
        component_root = Path(td) / "component-pkgs"
        for comp_name, comp_dir in pkg_component_workdirs(expanded, component_root, record):
            comp: dict[str, Any] = {
                "name": comp_name,
                "package_files": tree_entries(comp_dir, include_hashes=include_hashes),
            }
            package_info = comp_dir / "PackageInfo"
            if package_info.exists():
                comp["package_info"] = parse_package_info(package_info)
            scripts_dir = comp_dir / "Scripts"
            if scripts_dir.exists():
                if scripts_dir.is_dir():
                    comp_scripts = collect_scripts(scripts_dir, limit_bytes=script_text_limit)
                    comp["scripts"] = comp_scripts
                    append_installer_scripts_from_mapping(
                        record,
                        comp_scripts,
                        kind="macos-component-script",
                        source=f"pkg-xar:{comp_name}/Scripts",
                    )
                elif scripts_dir.is_file():
                    comp["scripts_compression"] = detect_payload_compression(scripts_dir)
                    scripts_root = Path(td) / "scripts" / comp_name
                    scripts_root.mkdir(parents=True)
                    rc, detail = extract_payload_cpio(scripts_dir, scripts_root, tools=tools)
                    if rc != 0:
                        comp["scripts_error"] = detail or "Scripts extraction failed"
                    else:
                        comp["script_files"] = tree_entries(scripts_root, include_hashes=include_hashes)
                        comp_scripts = collect_scripts(scripts_root, limit_bytes=script_text_limit)
                        comp["scripts"] = comp_scripts
                        append_installer_scripts_from_mapping(
                            record,
                            comp_scripts,
                            kind="macos-component-script",
                            source=f"pkg-xar:{comp_name}/Scripts",
                        )
            payload = comp_dir / "Payload"
            if payload.exists():
                comp["payload_compression"] = detect_payload_compression(payload)
                payload_root = Path(td) / "payloads" / comp_name
                payload_root.mkdir(parents=True)
                rc, detail = extract_payload_cpio(payload, payload_root, tools=tools)
                if rc != 0:
                    comp["payload_error"] = detail or "Payload extraction failed"
                else:
                    comp["payload"] = {"files": tree_entries(payload_root, include_hashes=include_hashes)}
            components.append(comp)
        record["analysis"]["components"] = components


def analyze_pkg(
    path: Path,
    record: dict[str, Any],
    *,
    tools: dict[str, ToolStatus],
    include_hashes: bool,
    script_text_limit: int,
) -> None:
    required_tools = ["xar", "cpio"]
    missing = missing_tool_names(tools, required_tools)
    if missing:
        add_error(record, "pkg payload inspection skipped because xar or cpio is missing")
        return

    analyze_pkg_with_xar(
        path,
        record,
        tools=tools,
        include_hashes=include_hashes,
        script_text_limit=script_text_limit,
    )


def analyze_exe(
    path: Path,
    record: dict[str, Any],
    *,
    tools: dict[str, ToolStatus],
    include_hashes: bool,
    script_text_limit: int,
    external_windows_scripts: list[ExternalScript] | None = None,
) -> None:
    if external_windows_scripts:
        append_external_scripts(record, external_windows_scripts, script_text_limit=script_text_limit)

    required_tools = ["7z"]
    missing = missing_tool_names(tools, required_tools)
    if missing:
        add_error(record, "exe payload inspection skipped because 7z is missing")
        return

    with tempfile.TemporaryDirectory(prefix="buckyos-analyze-exe-") as td:
        payload_root = Path(td) / "payload"
        result = run_capture(["7z", "x", f"-o{payload_root}", str(path), "-y"], check=False)
        if result.returncode != 0:
            add_error(record, result.stderr.strip() or result.stdout.strip() or "7z extraction failed")
            return
        record["analysis"]["payload"] = {
            "files": tree_entries(payload_root, include_hashes=include_hashes),
        }
        scripts = collect_scripts(payload_root, limit_bytes=script_text_limit)
        record["analysis"]["scripts"] = scripts
        append_installer_scripts_from_mapping(record, scripts, kind="windows-payload-script", source="exe-payload")


def analyze_package(
    path: Path,
    *,
    tools: dict[str, ToolStatus],
    include_hashes: bool,
    script_text_limit: int,
    external_windows_scripts: list[ExternalScript] | None = None,
) -> dict[str, Any]:
    record = base_package_record(path, include_hashes=include_hashes)
    suffix = path.suffix.lower()
    try:
        if suffix == ".deb":
            analyze_deb(path, record, tools=tools, include_hashes=include_hashes, script_text_limit=script_text_limit)
        elif suffix == ".rpm":
            analyze_rpm(path, record, tools=tools, include_hashes=include_hashes, script_text_limit=script_text_limit)
        elif suffix == ".pkg":
            analyze_pkg(path, record, tools=tools, include_hashes=include_hashes, script_text_limit=script_text_limit)
        elif suffix == ".exe":
            analyze_exe(
                path,
                record,
                tools=tools,
                include_hashes=include_hashes,
                script_text_limit=script_text_limit,
                external_windows_scripts=external_windows_scripts,
            )
        else:
            add_error(record, f"unsupported package suffix: {suffix}")
    except Exception as exc:
        add_error(record, f"{type(exc).__name__}: {exc}")
    return record


def find_packages(input_path: Path, *, recursive: bool) -> list[Path]:
    if input_path.is_file():
        return [input_path]
    candidates = input_path.rglob("*") if recursive else input_path.iterdir()
    return sorted(
        [
            path
            for path in candidates
            if path.is_file() and path.suffix.lower() in PACKAGE_SUFFIXES
        ],
        key=lambda p: p.as_posix(),
    )


def build_report(
    input_path: Path,
    *,
    include_hashes: bool,
    script_text_limit: int,
    recursive: bool,
    external_windows_scripts: list[ExternalScript] | None = None,
    tools: dict[str, ToolStatus] | None = None,
) -> dict[str, Any]:
    if tools is None:
        tools = tool_map(required_tool_names())
    packages = find_packages(input_path, recursive=recursive)
    report = {
        "schema_version": 1,
        "package_count": len(packages),
        "packages": [
            analyze_package(
                package,
                tools=tools,
                include_hashes=include_hashes,
                script_text_limit=script_text_limit,
                external_windows_scripts=external_windows_scripts,
            )
            for package in packages
        ],
    }
    return report


def write_report(report: dict[str, Any], out: Path | None, *, pretty: bool) -> None:
    text = json.dumps(report, ensure_ascii=False, indent=2 if pretty else None, sort_keys=True) + "\n"
    if out is None:
        sys.stdout.write(text)
        return
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(text, encoding="utf-8")
    print(out)


def write_separate_reports(report: dict[str, Any], out_dir: Path, *, pretty: bool) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    index: dict[str, Any] = {
        "schema_version": report["schema_version"],
        "package_count": report["package_count"],
        "packages": [],
    }
    for package in report["packages"]:
        package_report = {
            "schema_version": report["schema_version"],
            "package_count": 1,
            "packages": [package],
        }
        out_name = f"{package['name']}.analysis.json"
        out_path = out_dir / out_name
        write_report(package_report, out_path, pretty=pretty)
        index["packages"].append(
            {
                "name": package["name"],
                "format": package["format"],
                "size_bytes": package["size_bytes"],
                "analysis_json": out_name,
                "sha256": package.get("sha256"),
                "inferred": package.get("inferred", {}),
            }
        )
        if package.get("errors"):
            index["packages"][-1]["errors"] = package["errors"]
        if package.get("warnings"):
            index["packages"][-1]["warnings"] = package["warnings"]
    write_report(index, out_dir / "package_analysis_index.json", pretty=pretty)


def load_external_windows_scripts_from_args(args: argparse.Namespace) -> list[ExternalScript]:
    scripts: list[ExternalScript] = []
    for raw_path in args.windows_nsis_script or []:
        scripts.append(load_windows_nsis_from_local(Path(raw_path)))
    for spec in args.windows_nsis_ssh or []:
        scripts.append(load_windows_nsis_from_ssh(spec))
    if args.windows_nsis_from_ci:
        scripts.append(load_windows_nsis_from_ci(Path(args.ci_config).expanduser().resolve()))
    return scripts


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="Analyze BuckyOS package artifacts into JSON.")
    parser.add_argument("input", nargs="?", help="Package file or directory containing packages")
    parser.add_argument("--out", default=None, help="Output JSON file, or output directory with --separate")
    parser.add_argument("--separate", action="store_true", help="Write one JSON per package plus an index")
    parser.add_argument("--recursive", action="store_true", help="Recursively search directories for package files")
    parser.add_argument("--no-hash", action="store_true", help="Do not hash package/payload files")
    parser.add_argument("--compact", action="store_true", help="Emit compact JSON instead of pretty JSON")
    parser.add_argument(
        "--script-text-limit",
        type=int,
        default=64 * 1024,
        help="Maximum bytes of each script/control text to embed",
    )
    parser.add_argument(
        "--windows-nsis-script",
        action="append",
        default=[],
        help="Local generated installer.nsi to attach to Windows .exe analysis",
    )
    parser.add_argument(
        "--windows-nsis-ssh",
        action="append",
        default=[],
        metavar="USER@HOST:PATH",
        help="Fetch generated installer.nsi over SSH and attach it to Windows .exe analysis",
    )
    parser.add_argument(
        "--windows-nsis-from-ci",
        action="store_true",
        help="Fetch the current Windows installer.nsi from the CI Windows builder config",
    )
    parser.add_argument(
        "--ci-config",
        default=str(DEFAULT_CI_CONFIG),
        help="CI config path used with --windows-nsis-from-ci",
    )
    parser.add_argument("--check-tools", action="store_true", help="Only print analyzer tool availability JSON")
    args = parser.parse_args(argv[1:])
    required_tools = required_tool_names(
        include_windows_ssh=bool(args.windows_nsis_ssh or args.windows_nsis_from_ci)
    )
    tools = tool_map(required_tools)
    missing = missing_tool_names(tools, required_tools)

    if args.check_tools:
        payload = {
            "generated_at": utc_now(),
            "tools": {name: status.to_json() for name, status in tools.items()},
            "missing_tools": missing,
        }
        if missing:
            payload["install_hint"] = tool_install_hint(missing)
        write_report(payload, Path(args.out) if args.out else None, pretty=not args.compact)
        return 1 if missing else 0

    if missing:
        print(format_missing_tools_error(missing), file=sys.stderr)
        return 2

    if not args.input:
        parser.error("input is required unless --check-tools is used")

    input_path = Path(args.input).expanduser().resolve()
    if not input_path.exists():
        print(f"input not found: {input_path}", file=sys.stderr)
        return 2

    try:
        external_windows_scripts = load_external_windows_scripts_from_args(args)
    except Exception as exc:
        print(f"failed to load external Windows installer script: {exc}", file=sys.stderr)
        return 2

    report = build_report(
        input_path,
        include_hashes=not args.no_hash,
        script_text_limit=max(0, int(args.script_text_limit)),
        recursive=bool(args.recursive),
        external_windows_scripts=external_windows_scripts,
        tools=tools,
    )
    if args.separate:
        if not args.out:
            print("--separate requires --out <dir>", file=sys.stderr)
            return 2
        write_separate_reports(report, Path(args.out).expanduser().resolve(), pretty=not args.compact)
    else:
        write_report(report, Path(args.out).expanduser().resolve() if args.out else None, pretty=not args.compact)
    return 1 if any(package.get("errors") for package in report["packages"]) else 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
