#!/usr/bin/env python3

# BuckyOS runtime check script.
# This script is read-only: it only reports status and diagnosis suggestions.
#
# Check logic:
# 1. Resolve BUCKYOS_ROOT, then use BUCKYOS_ROOT/etc, BUCKYOS_ROOT/bin, BUCKYOS_ROOT/logs.
# 2. Detect activation state:
#    - If etc/node_identity.json does not exist, treat the system as not activated yet.
#    - In not-activated mode:
#      a. Check whether node_daemon is running.
#      b. Check whether port 3182 is listening.
#      c. Probe HTTP on 3182. If it responds, the system is in activation-ready state.
# 3. If etc/node_identity.json exists, treat the system as activated.
#    - Check core processes:
#      node_daemon, cyfs_gateway, system_config, scheduler, verify_hub, control_panel.
#    - Check key ports:
#      80, 3180, 3200, 3300, 4020.
#    - If gateway-related processes or ports are missing, verify whether the cyfs_gateway binary exists.
# 4. Scan logs under BUCKYOS_ROOT/logs and reuse existing runtime conventions for diagnosis:
#    - scheduler/node_daemon churn: too many recent log files with different PIDs
#    - permission errors in system_config/control_panel logs
#    - AICC/provider failures in aicc logs
#    - Message Center / Telegram failures in msg_center logs
#    - generic "service login to system failed" failures in service logs
# 5. Print one-shot summary, itemized checks, common error analysis, and return non-zero on hard failures.

from __future__ import annotations

import glob
import http.client
import os
import platform
import re
import shutil
import socket
import subprocess
import sys
from dataclasses import dataclass, field
from datetime import datetime
from pathlib import Path
from typing import Iterable


DEFAULT_BUCKYOS_ROOT = Path("/opt/buckyos")
TCP_TIMEOUT_SECS = 1.5
HTTP_TIMEOUT_SECS = 2.0
LOG_SCAN_LINE_LIMIT = 200


PROCESS_ALIASES = {
    "node_daemon": ("node-daemon", "node_daemon"),
    "cyfs_gateway": ("cyfs-gateway", "cyfs_gateway"),
    "system_config": ("system-config", "system_config"),
    "scheduler": ("scheduler",),
    "verify_hub": ("verify-hub", "verify_hub"),
    "control_panel": ("control-panel", "control_panel"),
    "msg_center": ("msg-center", "msg_center"),
    "aicc": ("aicc",),
    "repo_service": ("repo-service", "repo_service"),
    "task_manager": ("task-manager", "task_manager"),
    "opendan": ("opendan",),
}


ACTIVE_PORTS = {
    "zone_gateway_http": 80,
    "node_gateway_http": 3180,
    "system_config": 3200,
    "verify_hub": 3300,
    "control_panel": 4020,
}


LOG_DIR_CANDIDATES = {
    "node_daemon": ("node_daemon", "node-daemon"),
    "scheduler": ("scheduler",),
    "cyfs_gateway": ("cyfs_gateway", "cyfs-gateway"),
    "system_config": ("system_config_service", "system-config", "system_config"),
    "verify_hub": ("verify_hub", "verify-hub"),
    "control_panel": ("control-panel", "control_panel"),
    "msg_center": ("msg_center", "msg-center"),
    "aicc": ("aicc",),
    "repo_service": ("repo_service", "repo-service"),
    "task_manager": ("task_manager", "task-manager"),
    "opendan": ("opendan",),
}


CYFS_GATEWAY_BIN_CANDIDATES = (
    ("cyfs-gateway", "cyfs_gateway"),
    ("cyfs_gateway", "cyfs_gateway"),
    ("cyfs-gateway", "cyfs-gateway"),
    ("cyfs_gateway", "cyfs-gateway"),
)


@dataclass
class ProcessInfo:
    pid: int
    command: str
    args: str


@dataclass
class CheckItem:
    name: str
    status: str
    summary: str
    details: list[str] = field(default_factory=list)


@dataclass
class Diagnostic:
    severity: str
    title: str
    detail: str


def normalize_name(value: str) -> str:
    return value.strip().lower().replace("_", "-")


def resolve_buckyos_root() -> Path:
    raw = os.environ.get("BUCKYOS_ROOT", "").strip()
    return Path(raw).expanduser() if raw else DEFAULT_BUCKYOS_ROOT


def run_command(args: list[str]) -> subprocess.CompletedProcess[str] | None:
    try:
        return subprocess.run(
            args,
            capture_output=True,
            text=True,
            check=False,
        )
    except Exception:
        return None


def collect_processes() -> list[ProcessInfo]:
    result = run_command(["ps", "-axo", "pid=,comm=,args="])
    if result is None or result.returncode != 0:
        return []

    processes: list[ProcessInfo] = []
    for raw_line in result.stdout.splitlines():
        line = raw_line.strip()
        if not line:
            continue
        parts = line.split(None, 2)
        if len(parts) < 2:
            continue
        try:
            pid = int(parts[0])
        except ValueError:
            continue
        command = parts[1]
        args = parts[2] if len(parts) > 2 else command
        processes.append(ProcessInfo(pid=pid, command=command, args=args))
    return processes


def process_matches(proc: ProcessInfo, aliases: Iterable[str]) -> bool:
    normalized_aliases = {normalize_name(alias) for alias in aliases}
    base_command = normalize_name(Path(proc.command).name)
    args0 = normalize_name(Path(proc.args.split()[0]).name) if proc.args.strip() else base_command

    if base_command in normalized_aliases or args0 in normalized_aliases:
        return True

    haystack = normalize_name(proc.args)
    return any(alias in haystack for alias in normalized_aliases)


def find_processes(processes: list[ProcessInfo], aliases: Iterable[str]) -> list[ProcessInfo]:
    return [proc for proc in processes if process_matches(proc, aliases)]


def probe_tcp(host: str, port: int) -> bool:
    try:
        with socket.create_connection((host, port), timeout=TCP_TIMEOUT_SECS):
            return True
    except OSError:
        return False


def probe_http(host: str, port: int, path: str = "/") -> tuple[bool, str]:
    try:
        conn = http.client.HTTPConnection(host, port, timeout=HTTP_TIMEOUT_SECS)
        conn.request("GET", path)
        response = conn.getresponse()
        body = response.read(80)
        preview = body.decode("utf-8", errors="replace").strip().replace("\n", " ")
        return True, f"HTTP {response.status}" + (f", body={preview[:60]}" if preview else "")
    except Exception as error:
        return False, str(error)
    finally:
        try:
            conn.close()  # type: ignore[name-defined]
        except Exception:
            pass


def get_port_listener(port: int) -> str | None:
    lsof = shutil.which("lsof")
    if lsof:
        result = run_command([lsof, "-nP", f"-iTCP:{port}", "-sTCP:LISTEN"])
        if result and result.returncode == 0:
            lines = [line.strip() for line in result.stdout.splitlines() if line.strip()]
            if len(lines) >= 2:
                return lines[1]

    ss = shutil.which("ss")
    if ss:
        result = run_command([ss, "-lntp"])
        if result and result.returncode == 0:
            for line in result.stdout.splitlines():
                if f":{port} " in line or line.rstrip().endswith(f":{port}"):
                    return line.strip()

    return None


def port_owned_by(listener: str | None, aliases: Iterable[str]) -> bool:
    if not listener:
        return False
    normalized_listener = normalize_name(listener)
    compact_listener = re.sub(r"[^a-z0-9]", "", normalized_listener)
    listener_head = normalized_listener.split()[0] if normalized_listener.split() else normalized_listener
    compact_head = re.sub(r"[^a-z0-9]", "", listener_head)

    for alias in aliases:
        normalized_alias = normalize_name(alias)
        compact_alias = re.sub(r"[^a-z0-9]", "", normalized_alias)
        if normalized_alias in normalized_listener or compact_alias in compact_listener:
            return True
        if compact_alias.startswith(compact_head) or compact_head.startswith(compact_alias):
            return True
    return False


def find_log_dir(log_root: Path, service_key: str) -> Path | None:
    for name in LOG_DIR_CANDIDATES.get(service_key, (service_key,)):
        candidate = log_root / name
        if candidate.is_dir():
            return candidate
    return None


def list_log_files(log_dir: Path) -> list[Path]:
    if not log_dir.is_dir():
        return []
    files = [Path(p) for p in glob.glob(str(log_dir / "*.log"))]
    files.sort(key=lambda item: item.stat().st_mtime if item.exists() else 0, reverse=True)
    return files


def tail_lines(path: Path, max_lines: int = LOG_SCAN_LINE_LIMIT) -> list[str]:
    try:
        with path.open("r", encoding="utf-8", errors="replace") as handle:
            lines = handle.readlines()
        return [line.rstrip("\n") for line in lines[-max_lines:]]
    except Exception:
        return []


def extract_pid_from_log_name(path: Path) -> str | None:
    match = re.search(r"[_-](\d+)\.log$", path.name)
    if match:
        return match.group(1)
    return None


def service_error_lines(lines: list[str]) -> list[str]:
    hits: list[str] = []
    for line in lines:
        lowered = line.lower()
        if "[error]" in lowered or " error!" in lowered or " error:" in lowered or " failed" in lowered:
            hits.append(line.strip())
    return hits


def find_first_matching_line(lines: list[str], patterns: Iterable[str]) -> str | None:
    compiled = [re.compile(pattern, re.IGNORECASE) for pattern in patterns]
    for line in reversed(lines):
        for regex in compiled:
            if regex.search(line):
                return line.strip()
    return None


def build_check(name: str, status: str, summary: str, *details: str) -> CheckItem:
    return CheckItem(name=name, status=status, summary=summary, details=[detail for detail in details if detail])


def add_churn_diagnostic(diags: list[Diagnostic], log_dir: Path, service_label: str) -> None:
    log_files = list_log_files(log_dir)
    if not log_files:
        return

    pids = {pid for pid in (extract_pid_from_log_name(path) for path in log_files[:10]) if pid}
    if len(pids) > 2:
        diags.append(
            Diagnostic(
                severity="warn",
                title=f"{service_label} may be restarting repeatedly",
                detail=(
                    f"Recent log files under {log_dir} map to {len(pids)} different PIDs. "
                    "According to the check.py flow, the PID should be stable after boot, so check config and startup errors first."
                ),
            )
        )


def analyze_logs(log_root: Path) -> list[Diagnostic]:
    diagnostics: list[Diagnostic] = []

    for service_key, service_label in (
        ("scheduler", "scheduler"),
        ("node_daemon", "node_daemon"),
    ):
        log_dir = find_log_dir(log_root, service_key)
        if log_dir:
            add_churn_diagnostic(diagnostics, log_dir, service_label)

    for service_key, label in (
        ("system_config", "system_config"),
        ("control_panel", "control_panel"),
    ):
        log_dir = find_log_dir(log_root, service_key)
        if not log_dir:
            continue
        files = list_log_files(log_dir)
        if not files:
            continue
        lines = tail_lines(files[0])
        hit = find_first_matching_line(lines, [r"no permission", r"permission denied"])
        if hit:
            diagnostics.append(
                Diagnostic(
                    severity="warn",
                    title=f"{label} has permission-related errors",
                    detail=(
                        f"Recent logs contain `{hit}`. This usually means system config or file access permissions are wrong, "
                        "which can directly break config writes, Files, and related features."
                    ),
                )
            )

    aicc_log_dir = find_log_dir(log_root, "aicc")
    if aicc_log_dir:
        files = list_log_files(aicc_log_dir)
        if files:
            lines = tail_lines(files[0])
            hit = find_first_matching_line(
                lines,
                [
                    r"openai api error",
                    r"openai request failed",
                    r"provider_start_failed",
                    r"failed to parse openai response body",
                    r"token_limit_exceeded",
                ],
            )
            if hit:
                diagnostics.append(
                    Diagnostic(
                        severity="warn",
                        title="AICC/provider may be unhealthy",
                        detail=(
                            f"Recent logs contain `{hit}`. This usually means the API key is missing, the provider config is wrong, "
                            "or requests to OpenAI / the cloud provider are failing."
                        ),
                    )
                )

    msg_center_log_dir = find_log_dir(log_root, "msg_center")
    if msg_center_log_dir:
        files = list_log_files(msg_center_log_dir)
        if files:
            lines = tail_lines(files[0])
            hit = find_first_matching_line(
                lines,
                [
                    r"msg-center service login to system failed",
                    r"telegram.*failed",
                    r"getme",
                    r"bot.*failed",
                ],
            )
            if hit:
                diagnostics.append(
                    Diagnostic(
                        severity="warn",
                        title="Message Center / Telegram may be unhealthy",
                        detail=(
                            f"Recent logs contain `{hit}`. This usually means the Bot Token or AccountId is wrong, "
                            "or Telegram is unreachable, which often shows up as messages not being received."
                        ),
                    )
                )

    for service_key, label in (
        ("verify_hub", "verify_hub"),
        ("control_panel", "control_panel"),
        ("msg_center", "msg_center"),
        ("aicc", "aicc"),
        ("repo_service", "repo_service"),
        ("task_manager", "task_manager"),
        ("opendan", "opendan"),
    ):
        log_dir = find_log_dir(log_root, service_key)
        if not log_dir:
            continue
        files = list_log_files(log_dir)
        if not files:
            continue
        lines = tail_lines(files[0])
        hit = find_first_matching_line(lines, [r"service login to system failed"])
        if hit:
            diagnostics.append(
                Diagnostic(
                    severity="warn",
                    title=f"{label} cannot finish system login",
                    detail=(
                        f"Recent logs contain `{hit}`. This type of problem is usually related to system_config, verify_hub, "
                        "trust keys, or the zone config chain."
                    ),
                )
            )

        errors = service_error_lines(lines)
        if len(errors) >= 20:
            diagnostics.append(
                Diagnostic(
                    severity="warn",
                    title=f"{label} has many error logs",
                    detail=(
                        f"Detected {len(errors)} error/failed-related log lines in the latest {LOG_SCAN_LINE_LIMIT} lines. "
                        "Review this service log first."
                    ),
                )
            )

    return diagnostics


def cyfs_gateway_binary_exists(bin_root: Path) -> tuple[bool, list[Path]]:
    candidates: list[Path] = []
    for parent, binary in CYFS_GATEWAY_BIN_CANDIDATES:
        path = bin_root / parent / binary
        candidates.append(path)
        if path.exists():
            return True, candidates
    return False, candidates


def summarize_status(checks: list[CheckItem], activated: bool, activation_ready: bool, runtime_ready: bool, core_ready: bool) -> tuple[str, str]:
    if not activated:
        if activation_ready:
            return "Activation Ready", "node_active is serving on this machine and the system is waiting for activation"
        return "Not Running", "the system is not activated and node_active is not serving normally"

    has_fail = any(item.status == "fail" for item in checks)
    has_warn = any(item.status == "warn" for item in checks)
    if runtime_ready and not has_fail:
        if has_warn:
            return "Running With Warnings", "core services are reachable but there are warnings that need attention"
        return "Running", "core services and key ports are ready"
    if core_ready:
        return "Booting", "the system is activated and core processes exist, but runtime is not stable yet"
    return "Abnormal", "the system is activated, but core processes or key ports are missing"


def print_section(title: str) -> None:
    print()
    print(title)


def print_checks(checks: list[CheckItem]) -> None:
    status_prefix = {
        "ok": "[OK]",
        "warn": "[WARN]",
        "fail": "[FAIL]",
        "info": "[INFO]",
    }
    for item in checks:
        print(f"{status_prefix.get(item.status, '[INFO]')} {item.name}: {item.summary}")
        for detail in item.details:
            print(f"  - {detail}")


def main() -> int:
    root = resolve_buckyos_root()
    etc_dir = root / "etc"
    bin_dir = root / "bin"
    log_root = root / "logs"
    node_identity = etc_dir / "node_identity.json"

    now = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
    processes = collect_processes()

    checks: list[CheckItem] = []
    diagnostics: list[Diagnostic] = []

    activated = node_identity.exists()
    if activated:
        checks.append(
            build_check(
                "Activation State",
                "ok",
                f"Activated: found {node_identity}",
            )
        )
    else:
        checks.append(
            build_check(
                "Activation State",
                "warn",
                f"Not found: {node_identity}",
                "According to the runtime flow, this means the system is still in the not-activated / activation-pending stage.",
            )
        )

    node_daemon_procs = find_processes(processes, PROCESS_ALIASES["node_daemon"])
    if node_daemon_procs:
        pids = ", ".join(str(proc.pid) for proc in node_daemon_procs[:5])
        checks.append(build_check("node_daemon Process", "ok", f"Found {len(node_daemon_procs)} process(es)", f"PID: {pids}"))
    else:
        checks.append(build_check("node_daemon Process", "fail", "No node_daemon/node-daemon process found"))

    if not activated:
        port_3182_open = probe_tcp("127.0.0.1", 3182)
        listener_3182 = get_port_listener(3182)
        http_ok, http_detail = probe_http("127.0.0.1", 3182)

        if port_3182_open:
            status = "ok" if port_owned_by(listener_3182, PROCESS_ALIASES["node_daemon"]) else "warn"
            details = []
            if listener_3182:
                details.append(f"Listener: {listener_3182}")
            details.append(f"HTTP Probe: {http_detail}")
            checks.append(build_check("3182 Activation Port", status, "3182 is reachable", *details))
        else:
            checks.append(build_check("3182 Activation Port", "fail", "3182 is not reachable"))

        activation_ready = bool(node_daemon_procs) and port_3182_open and http_ok
        summary_title, summary_detail = summarize_status(checks, activated, activation_ready, False, False)
    else:
        core_proc_specs = (
            ("cyfs_gateway Process", "cyfs_gateway"),
            ("system_config Process", "system_config"),
            ("scheduler Process", "scheduler"),
            ("verify_hub Process", "verify_hub"),
            ("control_panel Process", "control_panel"),
        )
        process_presence: dict[str, bool] = {"node_daemon": bool(node_daemon_procs)}
        for label, key in core_proc_specs:
            procs = find_processes(processes, PROCESS_ALIASES[key])
            process_presence[key] = bool(procs)
            if procs:
                pids = ", ".join(str(proc.pid) for proc in procs[:5])
                checks.append(build_check(label, "ok", f"Found {len(procs)} process(es)", f"PID: {pids}"))
            else:
                severity = "warn" if key == "control_panel" else "fail"
                checks.append(build_check(label, severity, f"No {key} process found"))

        port_results: dict[int, bool] = {}
        for port_name, port in ACTIVE_PORTS.items():
            is_open = probe_tcp("127.0.0.1", port)
            port_results[port] = is_open
            listener = get_port_listener(port)
            aliases = PROCESS_ALIASES.get(port_name, PROCESS_ALIASES.get("cyfs_gateway", ()))
            expected_ok = port_owned_by(listener, aliases) if listener else False
            details = [f"Listener: {listener}"] if listener else []
            if is_open:
                status = "ok" if not listener or expected_ok else "warn"
                checks.append(build_check(f"Port {port}", status, f"{port_name} is reachable", *details))
            else:
                severity = "warn" if port_name == "control_panel" else "fail"
                checks.append(build_check(f"Port {port}", severity, f"{port_name} is not reachable", *details))

        cyfs_bin_exists, cyfs_bin_candidates = cyfs_gateway_binary_exists(bin_dir)
        if not process_presence["cyfs_gateway"] or not port_results[80] or not port_results[3180]:
            if cyfs_bin_exists:
                checks.append(
                    build_check(
                        "cyfs_gateway Binary",
                        "ok",
                        "cyfs_gateway executable exists",
                    )
                )
            else:
                candidates = ", ".join(str(path) for path in cyfs_bin_candidates)
                checks.append(
                    build_check(
                        "cyfs_gateway Binary",
                        "fail",
                        "cyfs_gateway executable was not found",
                        f"Checked paths: {candidates}",
                        "If 80/3180 are not listening, this usually means cyfs-gateway has not been built or installed successfully yet.",
                    )
                )

        diagnostics = analyze_logs(log_root)
        core_ready = (
            process_presence["node_daemon"]
            and process_presence["cyfs_gateway"]
            and process_presence["system_config"]
        )
        runtime_ready = (
            core_ready
            and process_presence["scheduler"]
            and process_presence["verify_hub"]
            and port_results[80]
            and port_results[3180]
            and port_results[3200]
            and port_results[3300]
        )
        summary_title, summary_detail = summarize_status(checks, activated, False, runtime_ready, core_ready)

    print("BuckyOS Local Runtime Check")
    print(f"- Time: {now}")
    print(f"- Platform: {platform.system()} {platform.release()}")
    print(f"- BUCKYOS_ROOT: {root}")
    print(f"- Overall Status: {summary_title}")
    print(f"- Status Detail: {summary_detail}")

    print_section("Checks")
    print_checks(checks)

    if diagnostics:
        print_section("Common Error Analysis")
        for item in diagnostics:
            prefix = "[WARN]" if item.severity == "warn" else "[INFO]"
            print(f"{prefix} {item.title}: {item.detail}")

    if log_root.exists():
        print_section("Extra Info")
        print(f"- Log Root: {log_root}")
        print(f"- Log Root Exists: Yes")
    else:
        print_section("Extra Info")
        print(f"- Log Root: {log_root}")
        print(f"- Log Root Exists: No")

    has_fail = any(item.status == "fail" for item in checks)
    return 1 if has_fail else 0


if __name__ == "__main__":
    sys.exit(main())
