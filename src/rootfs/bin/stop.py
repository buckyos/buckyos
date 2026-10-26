import os
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

def _product_runs_containers():
    """etc/product.json of this installation (the full edition when absent):
    BuckyOS Desktop Lite never calls docker, not even to clean up."""
    import json

    path = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "etc", "product.json")
    try:
        with open(path, encoding="utf-8") as file:
            return bool(json.load(file).get("capabilities", {}).get("container", False))
    except FileNotFoundError:
        return True
    except (OSError, ValueError):
        return False


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

def kill_buckyos_containers():
    if system == "Windows" or not _product_runs_containers() or not shutil.which("docker"):
        return

    result_list = subprocess.run(
        ["docker", "ps", "-aq", "--filter", "label=buckyos.app_instance_id"],
        capture_output=True,
        text=True,
    )
    if result_list.returncode != 0:
        stderr = result_list.stderr.strip()
        print(f"Failed to list buckyos docker containers: {stderr or 'unknown error'}")
        return

    container_ids = [
        container_id.strip()
        for container_id in result_list.stdout.splitlines()
        if container_id.strip()
    ]
    if not container_ids:
        print("No buckyos docker containers found")
        return

    result_rm = subprocess.run(
        ["docker", "rm", "-f", *container_ids],
        capture_output=True,
        text=True,
    )
    if result_rm.returncode != 0:
        stderr = result_rm.stderr.strip()
        print(f"Failed to remove buckyos docker containers: {stderr or 'unknown error'}")
        return

    removed = result_rm.stdout.strip()
    if removed:
        for container_id in removed.splitlines():
            print(f"{container_id} container removed")
    else:
        print("BuckyOS docker containers removed")

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

def kill_native_apps():
    """App processes node_daemon runs natively, from its records under
    local/app_native (a Desktop Lite script app on the node's Deno, a native
    Agent Service)."""
    import json
    import signal
    import time

    records = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "local", "app_native")
    if not os.path.isdir(records):
        return
    for name in sorted(os.listdir(records)):
        if not name.endswith(".json") or name.endswith(".failure.json"):
            continue
        try:
            with open(os.path.join(records, name), encoding="utf-8") as file:
                pid = int(json.load(file)["pid"])
        except (OSError, ValueError, KeyError, TypeError):
            continue
        if system == "Windows":
            command = subprocess.run(["tasklist", "/FI", f"PID eq {pid}", "/FO", "CSV", "/NH"],
                                     capture_output=True, text=True, errors="replace",
                                     **_windows_subprocess_kwargs()).stdout.lower()
        else:
            command = subprocess.run(["ps", "-o", "comm=", "-p", str(pid)], capture_output=True, text=True).stdout.lower()
        if "deno" not in command and "opendan" not in command:
            continue
        if system == "Windows":
            subprocess.run(["taskkill", "/F", "/T", "/PID", str(pid)], capture_output=True, **_windows_subprocess_kwargs())
        else:
            try:
                os.killpg(pid, signal.SIGTERM)
                time.sleep(2)
                os.killpg(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            except PermissionError as error:
                print(f"native app {name[:-5]} (pid {pid}) not stopped: {error}")
                continue
        print(f"native app {name[:-5]} (pid {pid}) stopped")


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
    kill_process("workflow")
    kill_native_apps()
    kill_buckyos_containers()
    kill_devtest_containers()


def main():
    kill_all()


if __name__ == "__main__":
    main()
