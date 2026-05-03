#!/usr/bin/env -S uv run

from __future__ import annotations

import argparse
import ctypes
import os
import platform
import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable

from buckyos_devkit.buckyos_kit import get_buckyos_root
from buckyos_devkit.project import AppInfo, BuckyProject


SCRIPT_DIR = Path(__file__).resolve().parent
PROJECT_CONFIG = SCRIPT_DIR / "bucky_project.yaml"

WINDOWS_TASK_NAME = "BuckyOSNodeDaemonKeepAlive"
WINDOWS_RUN_KEY = r"Software\Microsoft\Windows\CurrentVersion\Run"
WINDOWS_APP_KEY = r"Software\BuckyOS"
WINDOWS_UNINSTALL_KEY = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\BuckyOS"
WINDOWS_LEGACY_SERVICE = "buckyos"

MACOS_DAEMON_PLIST = Path("/Library/LaunchDaemons/buckyos.service.plist")
MACOS_AGENT_PLIST = Path("/Library/LaunchAgents/buckyos.service.plist")
MACOS_SERVICE_LABEL = "buckyos.service"

LINUX_SYSTEMD_UNIT = "buckyos.service"
LINUX_SYSTEMD_PATHS = (
    Path("/etc/systemd/system/buckyos.service"),
    Path("/lib/systemd/system/buckyos.service"),
    Path("/usr/lib/systemd/system/buckyos.service"),
)

HELPER_PATHS = (
    ".buckyos_installer_defaults",
    "scripts",
    "uninstall.exe",
)


@dataclass
class ActionResult:
    kind: str
    target: str
    status: str
    detail: str = ""


def _print_help() -> int:
    script_path = SCRIPT_DIR / "uninstall.py"
    print(
        "\n".join(
            [
                "BuckyOS uninstall helper",
                "",
                "Default behavior removes installed modules, runtime clean paths, and",
                "platform service/startup registrations while keeping user data.",
                "",
                "Usage:",
                f"  uv run {script_path.relative_to(SCRIPT_DIR.parent)}",
                f"  uv run {script_path.relative_to(SCRIPT_DIR.parent)} --all",
                "",
                "Options:",
                "  --all        Remove the entire BUCKYOS_ROOT directory after cleanup.",
                "  --root PATH  Override the detected BUCKYOS_ROOT.",
                "  --app NAME   App name in bucky_project.yaml, default: buckyos.",
            ]
        )
    )
    return 0


def _normalize_case(path: str) -> str:
    return os.path.normcase(os.path.normpath(path))


def _append_result(results: list[ActionResult], kind: str, target: str, status: str, detail: str = "") -> None:
    results.append(ActionResult(kind=kind, target=target, status=status, detail=detail))


def _run_command(args: list[str]) -> subprocess.CompletedProcess[str]:
    try:
        return subprocess.run(args, capture_output=True, text=True, encoding="utf-8", errors="ignore")
    except FileNotFoundError:
        return subprocess.CompletedProcess(args=args, returncode=127, stdout="", stderr=f"command not found: {args[0]}")


def _safe_command_output(result: subprocess.CompletedProcess[str]) -> str:
    return (result.stdout or "").strip() or (result.stderr or "").strip()


def _safe_join(root: Path, rel_path: str | Path) -> Path:
    root_resolved = root.resolve(strict=False)
    rel_text = os.fspath(rel_path).strip()
    rel_text = rel_text.lstrip("/\\")
    candidate = (root_resolved / rel_text).resolve(strict=False)
    try:
        candidate.relative_to(root_resolved)
    except ValueError as exc:
        raise ValueError(f"Refusing to operate outside BUCKYOS_ROOT: {rel_path}") from exc
    return candidate


def _remove_fs_path(path: Path, results: list[ActionResult], *, kind: str = "path") -> None:
    if not path.exists() and not path.is_symlink():
        _append_result(results, kind, str(path), "skipped", "not found")
        return

    try:
        if path.is_symlink() or path.is_file():
            path.unlink()
        else:
            shutil.rmtree(path)
        _append_result(results, kind, str(path), "ok", "removed")
    except Exception as exc:
        _append_result(results, kind, str(path), "failed", str(exc))


def _load_project_app(app_name: str) -> AppInfo:
    if not PROJECT_CONFIG.exists():
        raise FileNotFoundError(f"Missing project config: {PROJECT_CONFIG}")

    project = BuckyProject.from_file(PROJECT_CONFIG)
    app_info = project.apps.get(app_name)
    if app_info is None:
        raise ValueError(f"App {app_name!r} not found in {PROJECT_CONFIG}")
    return app_info


def _import_winreg():
    if os.name != "nt":
        return None
    import winreg  # type: ignore

    return winreg


def _read_windows_reg_value(root_key: str, subkey: str, value_name: str) -> str | None:
    winreg = _import_winreg()
    if winreg is None:
        return None

    hive = getattr(winreg, root_key, None)
    if hive is None:
        return None

    access = winreg.KEY_READ
    if hasattr(winreg, "KEY_WOW64_64KEY"):
        access |= winreg.KEY_WOW64_64KEY

    try:
        with winreg.OpenKey(hive, subkey, 0, access) as key:
            value, _ = winreg.QueryValueEx(key, value_name)
            if isinstance(value, str) and value.strip():
                return os.path.expandvars(value.strip())
    except OSError:
        return None
    return None


def _detect_windows_root_candidates() -> list[str]:
    candidates = []
    for root_key, subkey, value_name in (
        ("HKEY_CURRENT_USER", "Environment", "BUCKYOS_ROOT"),
        ("HKEY_CURRENT_USER", WINDOWS_APP_KEY, "InstallDir"),
        ("HKEY_CURRENT_USER", WINDOWS_APP_KEY, "BuckyOSUserDir"),
        ("HKEY_CURRENT_USER", WINDOWS_APP_KEY, "InstDir_buckyos"),
        ("HKEY_LOCAL_MACHINE", r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment", "BUCKYOS_ROOT"),
        ("HKEY_LOCAL_MACHINE", WINDOWS_APP_KEY, "BuckyOSServiceDir"),
    ):
        value = _read_windows_reg_value(root_key, subkey, value_name)
        if value:
            candidates.append(value)
    return candidates


def _detect_buckyos_root(explicit_root: str | None) -> Path:
    candidates: list[str] = []
    if explicit_root:
        candidates.append(explicit_root)

    env_root = os.environ.get("BUCKYOS_ROOT")
    if env_root:
        candidates.append(env_root)

    if os.name == "nt":
        candidates.extend(_detect_windows_root_candidates())

    candidates.append(get_buckyos_root())

    seen: set[str] = set()
    unique: list[Path] = []
    for raw in candidates:
        expanded = os.path.expandvars(os.path.expanduser(raw))
        if not expanded:
            continue
        normalized = _normalize_case(expanded)
        if normalized in seen:
            continue
        seen.add(normalized)
        unique.append(Path(expanded).resolve(strict=False))

    for candidate in unique:
        if candidate.exists():
            return candidate
    return unique[0]


def _kill_process_windows(results: list[ActionResult]) -> None:
    result = _run_command(["taskkill", "/F", "/IM", "node_daemon.exe"])
    output = _safe_command_output(result)
    if result.returncode == 0:
        _append_result(results, "process", "node_daemon.exe", "ok", output or "terminated")
        return
    lowered = output.lower()
    if "not found" in lowered or "no running instance" in lowered:
        _append_result(results, "process", "node_daemon.exe", "skipped", output or "not running")
        return
    _append_result(results, "process", "node_daemon.exe", "failed", output or f"exit code {result.returncode}")


def _kill_process_posix(results: list[ActionResult], root: Path) -> None:
    patterns = [
        str(root / "bin" / "node-daemon" / "node_daemon"),
        "node_daemon --enable_active",
        "node_daemon",
    ]
    for pattern in patterns:
        result = _run_command(["pkill", "-f", pattern])
        output = _safe_command_output(result)
        if result.returncode == 0:
            _append_result(results, "process", pattern, "ok", "terminated")
            return
        if result.returncode == 1:
            continue
        _append_result(results, "process", pattern, "failed", output or f"exit code {result.returncode}")
        return
    _append_result(results, "process", "node_daemon", "skipped", "not running")


def _windows_task_exists(task_name: str) -> bool:
    result = _run_command(["schtasks", "/Query", "/TN", task_name])
    return result.returncode == 0


def _delete_windows_task(task_name: str, results: list[ActionResult]) -> None:
    if not _windows_task_exists(task_name):
        _append_result(results, "task", task_name, "skipped", "not found")
        return

    result = _run_command(["schtasks", "/Delete", "/TN", task_name, "/F"])
    output = _safe_command_output(result)
    if result.returncode == 0:
        _append_result(results, "task", task_name, "ok", output or "deleted")
    else:
        _append_result(results, "task", task_name, "failed", output or f"exit code {result.returncode}")


def _windows_service_exists(name: str) -> bool:
    result = _run_command(["sc", "query", name])
    text = _safe_command_output(result).lower()
    if "does not exist" in text or "failed 1060" in text:
        return False
    return result.returncode == 0 or bool(text)


def _stop_delete_windows_service(name: str, results: list[ActionResult]) -> None:
    if not _windows_service_exists(name):
        _append_result(results, "service", name, "skipped", "not found")
        return

    stop_result = _run_command(["sc", "stop", name])
    stop_output = _safe_command_output(stop_result)
    if stop_result.returncode == 0:
        _append_result(results, "service", f"{name} (stop)", "ok", stop_output or "stopped")
    else:
        lowered = stop_output.lower()
        if "not started" in lowered or "service has not been started" in lowered:
            _append_result(results, "service", f"{name} (stop)", "skipped", stop_output or "not running")
        else:
            _append_result(
                results,
                "service",
                f"{name} (stop)",
                "failed",
                stop_output or f"exit code {stop_result.returncode}",
            )

    delete_result = _run_command(["sc", "delete", name])
    delete_output = _safe_command_output(delete_result)
    if delete_result.returncode == 0:
        _append_result(results, "service", f"{name} (delete)", "ok", delete_output or "deleted")
    else:
        lowered = delete_output.lower()
        if "does not exist" in lowered or "failed 1060" in lowered:
            _append_result(results, "service", f"{name} (delete)", "skipped", delete_output or "not found")
        else:
            _append_result(
                results,
                "service",
                f"{name} (delete)",
                "failed",
                delete_output or f"exit code {delete_result.returncode}",
            )


def _delete_windows_reg_value(
    root_key: str,
    subkey: str,
    value_name: str,
    results: list[ActionResult],
    *,
    only_if_matches: str | None = None,
) -> None:
    winreg = _import_winreg()
    if winreg is None:
        _append_result(results, "registry", f"{root_key}\\{subkey}\\{value_name}", "failed", "winreg unavailable")
        return

    hive = getattr(winreg, root_key, None)
    if hive is None:
        _append_result(results, "registry", f"{root_key}\\{subkey}\\{value_name}", "failed", "invalid hive")
        return

    access = winreg.KEY_READ | winreg.KEY_SET_VALUE
    if hasattr(winreg, "KEY_WOW64_64KEY"):
        access |= winreg.KEY_WOW64_64KEY

    target = f"{root_key}\\{subkey}\\{value_name}"
    try:
        with winreg.OpenKey(hive, subkey, 0, access) as key:
            value, _ = winreg.QueryValueEx(key, value_name)
            value_text = value.strip() if isinstance(value, str) else str(value)
            if only_if_matches and only_if_matches not in value_text:
                _append_result(results, "registry", target, "skipped", f"value kept: {value_text}")
                return
            winreg.DeleteValue(key, value_name)
            _append_result(results, "registry", target, "ok", "deleted")
    except FileNotFoundError:
        _append_result(results, "registry", target, "skipped", "not found")
    except OSError as exc:
        _append_result(results, "registry", target, "failed", str(exc))


def _delete_windows_reg_key(root_key: str, subkey: str, results: list[ActionResult]) -> None:
    winreg = _import_winreg()
    if winreg is None:
        _append_result(results, "registry", f"{root_key}\\{subkey}", "failed", "winreg unavailable")
        return

    hive = getattr(winreg, root_key, None)
    if hive is None:
        _append_result(results, "registry", f"{root_key}\\{subkey}", "failed", "invalid hive")
        return

    access = winreg.KEY_READ | winreg.KEY_WRITE
    if hasattr(winreg, "KEY_WOW64_64KEY"):
        access |= winreg.KEY_WOW64_64KEY

    target = f"{root_key}\\{subkey}"
    try:
        delete_key = getattr(winreg, "DeleteKeyEx", None)
        if delete_key is not None:
            delete_key(hive, subkey, access, 0)
        else:
            winreg.DeleteKey(hive, subkey)
        _append_result(results, "registry", target, "ok", "deleted")
    except FileNotFoundError:
        _append_result(results, "registry", target, "skipped", "not found")
    except OSError as exc:
        _append_result(results, "registry", target, "failed", str(exc))


def _broadcast_windows_environment_change(results: list[ActionResult]) -> None:
    HWND_BROADCAST = 0xFFFF
    WM_SETTINGCHANGE = 0x001A
    SMTO_ABORTIFHUNG = 0x0002

    try:
        user32 = ctypes.windll.user32
        SendMessageTimeoutW = user32.SendMessageTimeoutW
        SendMessageTimeoutW.argtypes = [
            ctypes.c_void_p,
            ctypes.c_uint,
            ctypes.c_void_p,
            ctypes.c_wchar_p,
            ctypes.c_uint,
            ctypes.c_uint,
            ctypes.POINTER(ctypes.c_ulong),
        ]
        SendMessageTimeoutW.restype = ctypes.c_void_p
        result_value = ctypes.c_ulong(0)
        rc = SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            "Environment",
            SMTO_ABORTIFHUNG,
            5000,
            ctypes.byref(result_value),
        )
        if rc:
            _append_result(results, "system", "Environment broadcast", "ok", "broadcasted")
        else:
            _append_result(results, "system", "Environment broadcast", "failed", "SendMessageTimeoutW returned 0")
    except Exception as exc:
        _append_result(results, "system", "Environment broadcast", "failed", str(exc))


def _cleanup_windows_platform(root: Path, results: list[ActionResult]) -> None:
    _delete_windows_task(WINDOWS_TASK_NAME, results)
    _delete_windows_reg_value("HKEY_CURRENT_USER", WINDOWS_RUN_KEY, "BuckyOSDaemon", results)
    _kill_process_windows(results)
    _stop_delete_windows_service(WINDOWS_LEGACY_SERVICE, results)

    root_text = str(root)
    _delete_windows_reg_value("HKEY_CURRENT_USER", "Environment", "BUCKYOS_ROOT", results)
    _delete_windows_reg_value(
        "HKEY_LOCAL_MACHINE",
        r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
        "BUCKYOS_ROOT",
        results,
        only_if_matches=root_text,
    )
    _delete_windows_reg_key("HKEY_CURRENT_USER", WINDOWS_UNINSTALL_KEY, results)
    _delete_windows_reg_key("HKEY_CURRENT_USER", WINDOWS_APP_KEY, results)
    _delete_windows_reg_key("HKEY_LOCAL_MACHINE", WINDOWS_APP_KEY, results)
    _broadcast_windows_environment_change(results)


def _cleanup_macos_platform(root: Path, results: list[ActionResult]) -> None:
    _kill_process_posix(results, root)

    disable_result = _run_command(["launchctl", "disable", f"system/{MACOS_SERVICE_LABEL}"])
    disable_output = _safe_command_output(disable_result)
    if disable_result.returncode == 0:
        _append_result(results, "service", f"launchctl disable system/{MACOS_SERVICE_LABEL}", "ok", "disabled")
    else:
        _append_result(
            results,
            "service",
            f"launchctl disable system/{MACOS_SERVICE_LABEL}",
            "skipped",
            disable_output or "not enabled",
        )

    for plist_path, scope in ((MACOS_DAEMON_PLIST, "system"), (MACOS_AGENT_PLIST, "gui")):
        if plist_path.exists():
            if scope == "system":
                bootout = _run_command(["launchctl", "bootout", "system", str(plist_path)])
            else:
                console_user = _run_command(["stat", "-f%Su", "/dev/console"])
                user_name = (console_user.stdout or "").strip()
                if user_name and user_name not in {"root", "loginwindow"}:
                    uid_result = _run_command(["id", "-u", user_name])
                    uid = (uid_result.stdout or "").strip()
                    if uid:
                        bootout = _run_command(["launchctl", "bootout", f"gui/{uid}", str(plist_path)])
                    else:
                        bootout = None
                else:
                    bootout = None

            if bootout is None:
                _append_result(results, "service", str(plist_path), "skipped", "no console user launch agent")
            else:
                output = _safe_command_output(bootout)
                if bootout.returncode == 0:
                    _append_result(results, "service", str(plist_path), "ok", output or "booted out")
                else:
                    _append_result(results, "service", str(plist_path), "skipped", output or "not loaded")
        else:
            _append_result(results, "service", str(plist_path), "skipped", "not found")

        _remove_fs_path(plist_path, results, kind="service-file")


def _cleanup_linux_platform(root: Path, results: list[ActionResult]) -> None:
    _kill_process_posix(results, root)

    stop_result = _run_command(["systemctl", "stop", LINUX_SYSTEMD_UNIT])
    stop_output = _safe_command_output(stop_result)
    if stop_result.returncode == 0:
        _append_result(results, "service", f"systemctl stop {LINUX_SYSTEMD_UNIT}", "ok", "stopped")
    else:
        _append_result(
            results,
            "service",
            f"systemctl stop {LINUX_SYSTEMD_UNIT}",
            "skipped",
            stop_output or "not active",
        )

    disable_result = _run_command(["systemctl", "disable", LINUX_SYSTEMD_UNIT])
    disable_output = _safe_command_output(disable_result)
    if disable_result.returncode == 0:
        _append_result(results, "service", f"systemctl disable {LINUX_SYSTEMD_UNIT}", "ok", "disabled")
    else:
        _append_result(
            results,
            "service",
            f"systemctl disable {LINUX_SYSTEMD_UNIT}",
            "skipped",
            disable_output or "not enabled",
        )

    removed_unit = False
    for unit_path in LINUX_SYSTEMD_PATHS:
        if unit_path.exists():
            removed_unit = True
        _remove_fs_path(unit_path, results, kind="service-file")

    daemon_reload = _run_command(["systemctl", "daemon-reload"])
    reload_output = _safe_command_output(daemon_reload)
    if daemon_reload.returncode == 0:
        _append_result(results, "service", "systemctl daemon-reload", "ok", "reloaded")
    else:
        status = "failed" if removed_unit else "skipped"
        _append_result(
            results,
            "service",
            "systemctl daemon-reload",
            status,
            reload_output or f"exit code {daemon_reload.returncode}",
        )


def _cleanup_platform(root: Path, results: list[ActionResult]) -> None:
    system_name = platform.system().lower()
    if system_name == "windows":
        _cleanup_windows_platform(root, results)
    elif system_name == "darwin":
        _cleanup_macos_platform(root, results)
    elif system_name == "linux":
        _cleanup_linux_platform(root, results)
    else:
        _append_result(results, "platform", system_name or "unknown", "skipped", "no platform cleanup defined")


def _run_installed_stop_script(root: Path, results: list[ActionResult]) -> None:
    system_name = platform.system().lower()
    if system_name == "windows":
        script_path = root / "bin" / "stop.ps1"
        if not script_path.exists():
            _append_result(results, "stop-script", str(script_path), "skipped", "not found")
            return
        result = _run_command([
            "powershell.exe",
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            str(script_path),
        ])
    elif system_name == "darwin":
        script_path = root / "bin" / "stop_osx.sh"
        if script_path.exists():
            result = _run_command(["/bin/sh", str(script_path)])
        else:
            legacy_script_path = root / "bin" / "stop.py"
            if not legacy_script_path.exists():
                _append_result(results, "stop-script", str(script_path), "skipped", "not found")
                return
            script_path = legacy_script_path
            result = _run_command(["python3", str(script_path)])
    else:
        script_path = root / "bin" / "stop.py"
        if not script_path.exists():
            _append_result(results, "stop-script", str(script_path), "skipped", "not found")
            return
        result = _run_command(["python3", str(script_path)])

    output = _safe_command_output(result)
    if result.returncode == 0:
        _append_result(results, "stop-script", str(script_path), "ok", output or "completed")
    else:
        _append_result(results, "stop-script", str(script_path), "failed", output or f"exit code {result.returncode}")


def _iter_remove_paths(root: Path, app_info: AppInfo) -> Iterable[Path]:
    for module_path in app_info.modules.values():
        normalized = str(module_path).strip().rstrip("/\\")
        if normalized:
            yield _safe_join(root, normalized)

    for clean_path in app_info.clean_paths:
        normalized = os.fspath(clean_path).strip().rstrip("/\\")
        if normalized:
            yield _safe_join(root, normalized)

    for helper_path in HELPER_PATHS:
        yield _safe_join(root, helper_path)


def _is_fs_root(path: Path) -> bool:
    resolved = path.resolve(strict=False)
    if resolved.parent == resolved:
        return True
    if os.name == "nt":
        anchor = Path(resolved.anchor).resolve(strict=False)
        return resolved == anchor
    return str(resolved) == "/"


def _remove_install_paths(root: Path, app_info: AppInfo, results: list[ActionResult]) -> None:
    seen: set[str] = set()
    for path in _iter_remove_paths(root, app_info):
        normalized = _normalize_case(str(path))
        if normalized in seen:
            continue
        seen.add(normalized)
        _remove_fs_path(path, results)


def _remove_root_all(root: Path, results: list[ActionResult]) -> None:
    if _is_fs_root(root):
        _append_result(results, "root", str(root), "failed", "refusing to delete filesystem root")
        return
    _remove_fs_path(root, results, kind="root")


def _print_results(root: Path, remove_all: bool, results: list[ActionResult]) -> None:
    print(f"Platform: {platform.system()}")
    print(f"BUCKYOS_ROOT: {root}")
    print(f"Mode: {'full removal (--all)' if remove_all else 'keep user data'}")
    print("")
    print("Operation results:")
    for item in results:
        detail = f" ({item.detail})" if item.detail else ""
        print(f"[{item.status.upper():7}] {item.kind}: {item.target}{detail}")

    ok_count = sum(1 for item in results if item.status == "ok")
    skipped_count = sum(1 for item in results if item.status == "skipped")
    failed_count = sum(1 for item in results if item.status == "failed")

    print("")
    print(f"Summary: ok={ok_count}, skipped={skipped_count}, failed={failed_count}")


def main() -> int:
    args = sys.argv[1:]
    if any(arg in {"-h", "--help"} for arg in args):
        return _print_help()

    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("--all", action="store_true")
    parser.add_argument("--root")
    parser.add_argument("--app", default="buckyos")
    parsed = parser.parse_args(args)

    root = _detect_buckyos_root(parsed.root)
    os.environ["BUCKYOS_ROOT"] = str(root)

    try:
        app_info = _load_project_app(parsed.app)
    except Exception as exc:
        print(f"Failed to load uninstall configuration: {exc}")
        return 1

    results: list[ActionResult] = []
    _cleanup_platform(root, results)
    _run_installed_stop_script(root, results)
    _remove_install_paths(root, app_info, results)
    if parsed.all:
        _remove_root_all(root, results)

    _print_results(root, parsed.all, results)
    return 1 if any(item.status == "failed" for item in results) else 0


if __name__ == "__main__":
    raise SystemExit(main())
