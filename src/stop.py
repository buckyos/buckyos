#!/usr/bin/env -S uv run

import os
import platform
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

def kill_process(name):
    if system == "Windows":
        result = subprocess.run(
            ["taskkill", "/F", "/IM", f"{name}{ext}"],
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
    kill_process("system-config")
    kill_process("system_config")
    kill_process("cyfs-gateway")
    kill_process("cyfs_gateway")
    kill_process("filebrowser")
    kill_process("smb-service")
    kill_process("smb_service")
    kill_process("repo-service")
    kill_process("repo_service")
    kill_process("control-panel")
    kill_process("control_panel")
    kill_process("aicc")
    kill_process("task_manager")
    kill_process("kmsg")
    kill_process("msg_center")
    kill_process("opendan")
    kill_process("workflow")
    kill_devtest_containers()


def main():
    kill_all()


if __name__ == "__main__":
    main()
