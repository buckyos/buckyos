#!/usr/bin/env -S uv run

import json
import os
from pathlib import Path
import platform
import shutil
import subprocess

system = platform.system()
ext = ""
if system == "Windows":
    ext = ".exe"


def _windows_subprocess_kwargs() -> dict[str, object]:
    if system != "Windows":
        return {}

    startupinfo = subprocess.STARTUPINFO()
    startupinfo.dwFlags |= subprocess.STARTF_USESHOWWINDOW
    startupinfo.wShowWindow = subprocess.SW_HIDE
    return {
        "startupinfo": startupinfo,
        "creationflags": subprocess.CREATE_NO_WINDOW,
    }

def _buckyos_root():
    if os.environ.get("BUCKYOS_ROOT"):
        return Path(os.environ["BUCKYOS_ROOT"])
    if system == "Windows":
        return Path(r"C:\buckyos")
    return Path("/opt/buckyos")


def _product_runs_containers():
    """etc/product.json of the installation (the full edition when absent):
    BuckyOS Desktop Lite never calls docker, not even to clean up."""
    try:
        profile = json.loads((_buckyos_root() / "etc" / "product.json").read_text(encoding="utf-8"))
        return bool(profile.get("capabilities", {}).get("container", False))
    except FileNotFoundError:
        return True
    except (OSError, ValueError):
        return False


def _process_command(pid):
    if system == "Windows":
        result = subprocess.run(
            ["tasklist", "/FI", f"PID eq {pid}", "/FO", "CSV", "/NH"],
            capture_output=True, text=True, errors="replace", **_windows_subprocess_kwargs(),
        )
        return result.stdout.split(",")[0].strip('"').lower() if result.returncode == 0 else ""
    result = subprocess.run(["ps", "-o", "comm=", "-p", str(pid)], capture_output=True, text=True)
    return os.path.basename(result.stdout.strip()).lower() if result.returncode == 0 else ""


def kill_native_apps():
    """App processes node_daemon runs natively, from its records under
    local/app_native (a Desktop Lite script app on the node's Deno, a native
    Agent Service): a fresh install drops the records, so they go now."""
    import signal
    import time

    for record in sorted((_buckyos_root() / "local" / "app_native").glob("*.json")):
        if record.name.endswith(".failure.json"):
            continue
        try:
            pid = int(json.loads(record.read_text(encoding="utf-8"))["pid"])
        except (OSError, ValueError, KeyError, TypeError):
            continue
        command = _process_command(pid)
        if not any(name in command for name in ("deno", "opendan")):
            continue
        if system == "Windows":
            subprocess.run(["taskkill", "/F", "/T", "/PID", str(pid)], capture_output=True,
                           **_windows_subprocess_kwargs())
        else:
            try:
                os.killpg(pid, signal.SIGTERM)
                time.sleep(2)
                os.killpg(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            except PermissionError as error:
                print(f"native app {record.stem} (pid {pid}) not stopped: {error}")
                continue
        print(f"native app {record.stem} (pid {pid}) stopped")


def kill_devtest_containers():
    if system == "Windows" or not _product_runs_containers() or not shutil.which("docker"):
        return

    result_list = subprocess.run(
        ["docker", "ps", "-a", "--format", "{{.Names}}"],
        capture_output=True,
        text=True,
    )
    if result_list.returncode != 0:
        stderr = result_list.stderr.strip()
        print(f"Failed to list docker containers: {stderr or 'unknown error'}")
        return

    container_names = [
        name.strip()
        for name in result_list.stdout.splitlines()
        if name.strip().startswith("devtest-")
    ]
    if not container_names:
        print("No devtest-* docker containers found")
        return

    for container_name in container_names:
        result_kill = subprocess.run(
            ["docker", "kill", container_name],
            capture_output=True,
            text=True,
        )
        if result_kill.returncode != 0:
            stderr = result_kill.stderr.strip()
            print(f"Failed to kill {container_name}: {stderr or 'unknown error'}")
        else:
            print(f"{container_name} container killed")

def _container_program():
    runtime = os.environ.get("BUCKYOS_CONTAINER_RUNTIME")
    if runtime not in (None, "docker", "wslc"):
        raise ValueError(f"unknown BUCKYOS_CONTAINER_RUNTIME: {runtime}")
    if runtime == "docker" or (system != "Windows" and runtime is None):
        return "docker", False
    wslc = shutil.which("wslc.exe")
    if not wslc:
        candidate = Path(os.environ.get("ProgramFiles", r"C:\Program Files")) / "WSL" / "wslc.exe"
        if candidate.is_file():
            wslc = str(candidate)
    if wslc or runtime == "wslc":
        return wslc or "wslc.exe", True
    return "docker", False


def kill_buckyos_containers():
    if not _product_runs_containers():
        print("This product edition runs no containers")
        return
    program, is_wslc = _container_program()
    try:
        result_list = subprocess.run(
            [program, "container", "ls", "-a", "--format", "json" if is_wslc else "{{.Names}}"],
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            **_windows_subprocess_kwargs(),
        )
    except FileNotFoundError:
        print(f"Container runtime {program} is not installed")
        return
    if result_list.returncode != 0:
        stderr = result_list.stderr.strip()
        print(f"Failed to list buckyos containers: {stderr or 'unknown error'}")
        return

    if is_wslc:
        names = []
        decoder = json.JSONDecoder()
        remaining = result_list.stdout.strip()
        while remaining:
            value, offset = decoder.raw_decode(remaining)
            rows = value if isinstance(value, list) else [value]
            names.extend(row["Names"] for row in rows)
            remaining = remaining[offset:].lstrip()
    else:
        names = result_list.stdout.splitlines()
    container_names = [name.strip() for name in names if name.strip().startswith("buckyos-app-")]
    if not container_names:
        print("No buckyos containers found")
        return

    result_rm = subprocess.run(
        [program, "rm", "-f", *container_names],
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        **_windows_subprocess_kwargs(),
    )
    if result_rm.returncode != 0:
        stderr = result_rm.stderr.strip()
        print(f"Failed to remove buckyos containers: {stderr or 'unknown error'}")
        return
    for name in container_names:
        print(f"{name} container removed")

def kill_process(name):
    if system == "Windows":
        # /T also kills children such as leftover `docker pull` so the next
        # start is not blocked on a daemon-side image lock.
        result = subprocess.run(
            ["taskkill", "/F", "/T", "/IM", f"{name}{ext}"],
            capture_output=True,
            **_windows_subprocess_kwargs(),
        )
    else:
        result = subprocess.run(
            ["killall", f"{name}{ext}"],
            capture_output=True,
            text=True,
        )

    if result.returncode != 0:
        print(f"{name} not running")
    else:
        print(f"{name} killed")

MACOS_RUNTIME_LABEL = "com.buckyos.runtime"


def stop_macos_runtime_agent():
    """Unload the bound user's LaunchAgent first, or KeepAlive restarts
    node_daemon as soon as it is killed. The login item stays installed."""
    if system != "Darwin":
        return
    root = _buckyos_root()
    uid = os.getuid()
    try:
        binding = json.loads((root / "etc" / "desktop_runtime.json").read_text(encoding="utf-8"))
        uid = int(binding["uid"])
    except (OSError, ValueError, KeyError, TypeError):
        pass
    target = f"gui/{uid}/{MACOS_RUNTIME_LABEL}"
    result = subprocess.run(["launchctl", "bootout", target], capture_output=True, text=True)
    if result.returncode == 0:
        print(f"{target} booted out")
    else:
        print(f"{target} not loaded")


def kill_all():
    stop_macos_runtime_agent()
    kill_process("node-daemon")
    kill_process("node_daemon")
    kill_process("scheduler")
    kill_process("verify-hub")
    kill_process("verify_hub")
    kill_process("klog-daemon")
    kill_process("klog_daemon")
    kill_process("slog_daemon")
    kill_process("slog_server")
    kill_process("system-config")
    kill_process("system_config")
    kill_process("cyfs-gateway")
    kill_process("cyfs_gateway")
    kill_process("filebrowser")
    kill_process("smb-service")
    kill_process("smb_service")
    kill_process("nfs-server")
    kill_process("nfs_server")
    kill_process("aiworkspace")
    kill_process("homestation")
    kill_process("repo-service")
    kill_process("repo_service")
    kill_process("control-panel")
    kill_process("control_panel")
    kill_process("aicc")
    kill_process("task_manager")
    kill_process("kmsg")
    kill_process("msg_center")
    kill_process("opendan")
    kill_process("xagent")
    kill_process("agent_tool")
    kill_process("workflow")
    kill_native_apps()
    kill_buckyos_containers()
    kill_devtest_containers()


def main():
    kill_all()


if __name__ == "__main__":
    main()
