#!/usr/bin/env -S uv run

"""Windows scheduled-task startup helper.

Simulates the production Windows installer launch path from
`publish/win_pkg/scripts/buckyos_postinstall.ps1`:

1) stop existing BuckyOS processes and update files (same as start.py)
2) install `node_daemon_loader.vbs` into `$BUCKYOS_ROOT/scripts`
3) create keepalive task `BuckyOSNodeDaemonKeepAlive` (every 1 minute)
4) write HKCU Run key `BuckyOSDaemon`
5) immediately `schtasks /Run` so node_daemon starts without waiting
"""

from __future__ import annotations

import csv
import io
import os
import platform
import shutil
import subprocess
import sys
import time
from pathlib import Path

import start

SCRIPT_DIR = Path(__file__).resolve().parent
LOADER_SOURCE = SCRIPT_DIR / "publish" / "win_pkg" / "scripts" / "node_daemon_loader.vbs"
TASK_NAME = "BuckyOSNodeDaemonKeepAlive"
RUN_VALUE_NAME = "BuckyOSDaemon"
RUN_KEY = r"Software\Microsoft\Windows\CurrentVersion\Run"


def _print_help() -> int:
    script_path = SCRIPT_DIR / "start_win.py"
    print(
        "\n".join(
            [
                "BuckyOS Windows scheduled-task startup helper",
                "",
                "Same install/update flow as start.py, but starts node_daemon through",
                "the production Windows keepalive task instead of spawning it directly.",
                "",
                "Usage:",
                f"  uv run {script_path.relative_to(SCRIPT_DIR.parent)} [options]",
                f"  cd {SCRIPT_DIR} && uv run ./start_win.py [options]",
                "",
                "Options:",
                "  --all                 Fresh dev install before startup.",
                "  --reinstall [GROUP]   Fresh install before startup, optionally running make_config.ts GROUP.",
                "  --skip-update         Skip file update and only restart BuckyOS.",
                "  -h, --help            Show this help message.",
                "",
                "Notes:",
                f"  Keepalive task {TASK_NAME} relaunches node_daemon every minute.",
                "  Use stop_win.py (not stop.py) to delete the task before killing processes.",
            ]
        )
    )
    return 0


def _run_captured(args: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        args,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="ignore",
        **start._windows_subprocess_kwargs(),
    )


def _command_output(result: subprocess.CompletedProcess[str]) -> str:
    return (result.stdout or "").strip() or (result.stderr or "").strip()


def _ensure_windows() -> None:
    if platform.system() != "Windows":
        print("start_win.py only supports Windows.")
        sys.exit(1)


def _ensure_loader(buckyos_root: Path) -> Path:
    if not LOADER_SOURCE.is_file():
        raise FileNotFoundError(f"Missing loader source: {LOADER_SOURCE}")

    dest_dir = buckyos_root / "scripts"
    dest_dir.mkdir(parents=True, exist_ok=True)
    dest = dest_dir / "node_daemon_loader.vbs"
    shutil.copy2(LOADER_SOURCE, dest)
    return dest


def _build_run_command(loader: Path, node_daemon: Path) -> str:
    return f'wscript.exe //B //NoLogo "{loader}" "{node_daemon}"'


def _create_keepalive_task(run_command: str) -> None:
    result = _run_captured(
        [
            "schtasks",
            "/Create",
            "/TN",
            TASK_NAME,
            "/SC",
            "MINUTE",
            "/MO",
            "1",
            "/F",
            "/TR",
            run_command,
        ]
    )
    output = _command_output(result)
    if result.returncode != 0:
        raise RuntimeError(output or f"schtasks /Create failed with exit code {result.returncode}")
    if output:
        print(output)


def _run_keepalive_task() -> None:
    result = _run_captured(["schtasks", "/Run", "/TN", TASK_NAME])
    output = _command_output(result)
    if result.returncode != 0:
        raise RuntimeError(output or f"schtasks /Run failed with exit code {result.returncode}")
    if output:
        print(output)


def _write_run_key(run_command: str) -> None:
    import winreg

    with winreg.CreateKeyEx(winreg.HKEY_CURRENT_USER, RUN_KEY, 0, winreg.KEY_SET_VALUE) as key:
        winreg.SetValueEx(key, RUN_VALUE_NAME, 0, winreg.REG_SZ, run_command)


def _query_node_daemon_pid() -> int | None:
    result = _run_captured(
        ["tasklist", "/FI", "IMAGENAME eq node_daemon.exe", "/FO", "CSV", "/NH"]
    )
    if result.returncode != 0:
        return None

    text = (result.stdout or "").strip()
    if not text or "No tasks are running" in text or "INFO:" in text:
        return None

    try:
        row = next(csv.reader(io.StringIO(text)))
    except StopIteration:
        return None
    if len(row) < 2:
        return None
    try:
        return int(row[1])
    except ValueError:
        return None


def _wait_for_node_daemon(timeout_secs: float = 5.0) -> int | None:
    deadline = time.time() + timeout_secs
    while time.time() < deadline:
        pid = _query_node_daemon_pid()
        if pid is not None:
            return pid
        time.sleep(0.3)
    return None


def start_system() -> None:
    print("Starting BuckyOS via Windows scheduled task...")
    buckyos_root = start.resolve_buckyos_root()
    node_daemon = buckyos_root / "bin" / "node-daemon" / "node_daemon.exe"
    if not node_daemon.is_file():
        print(f"Error: Cannot find node_daemon executable: {node_daemon}")
        print(f"Please check if the installation directory is correct: {buckyos_root}")
        sys.exit(1)

    try:
        loader = _ensure_loader(buckyos_root)
        run_command = _build_run_command(loader, node_daemon)
        print(f"Loader: {loader}")
        print(f"Task command: {run_command}")

        _create_keepalive_task(run_command)
        print(f"Scheduled task ready: {TASK_NAME} (every 1 minute)")

        _write_run_key(run_command)
        print(f"Startup Run key written: HKCU\\{RUN_KEY}\\{RUN_VALUE_NAME}")

        _run_keepalive_task()
        print(f"Triggered scheduled task: {TASK_NAME}")

        pid = _wait_for_node_daemon()
        if pid is None:
            print("Warning: node_daemon.exe was not observed within 5 seconds.")
            print("The keepalive task will retry every minute.")
        else:
            print(f"node_daemon pid: {pid}")
        print("System is running via Windows scheduled-task keepalive...")
    except Exception as e:
        print(f"Failed to start system via scheduled task: {e}")
        sys.exit(1)


def main() -> int:
    args = sys.argv[1:]
    if any(arg in {"-h", "--help"} for arg in args):
        return _print_help()

    _ensure_windows()
    print("=== BuckyOS Windows Scheduled-Task Startup ===")

    config_group_name = None
    install_all = "--all" in args or "--reinstall" in args
    need_update = "--skip-update" not in args
    if install_all:
        config_group_name = "dev"
    if "--reinstall" in args:
        config_group_name = None
        group_name_index = args.index("--reinstall") + 1
        if group_name_index < len(args):
            config_group_name = args[group_name_index]

    start.kill_all_processes()

    if install_all or need_update:
        start.update_files(install_all, config_group_name)

    start_system()
    print("=== BuckyOS Windows Startup Complete ===")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
