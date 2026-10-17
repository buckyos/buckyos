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

def kill_devtest_containers():
    if system == "Windows":
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
            text=True,
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

def kill_all():
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
    kill_buckyos_containers()
    kill_devtest_containers()


def main():
    kill_all()


if __name__ == "__main__":
    main()
