#!/usr/bin/env -S uv run

"""Windows scheduled-task stop helper.

Mirrors the production Windows installer teardown from
`publish/win_pkg/scripts/buckyos_preuninstall.ps1`:

1) delete keepalive task `BuckyOSNodeDaemonKeepAlive`
2) remove HKCU Run key `BuckyOSDaemon`
3) kill BuckyOS processes (same as stop.py)

The task must be removed before killing processes, otherwise keepalive
would relaunch node_daemon within a minute.
"""

from __future__ import annotations

import platform
import sys
from pathlib import Path

import start_win
import stop

SCRIPT_DIR = Path(__file__).resolve().parent


def _print_help() -> int:
    script_path = SCRIPT_DIR / "stop_win.py"
    print(
        "\n".join(
            [
                "BuckyOS Windows scheduled-task stop helper",
                "",
                "Removes the production Windows keepalive task and Run key,",
                "then stops BuckyOS processes the same way as stop.py.",
                "",
                "Usage:",
                f"  uv run {script_path.relative_to(SCRIPT_DIR.parent)}",
                f"  cd {SCRIPT_DIR} && uv run ./stop_win.py",
                "",
                "Options:",
                "  -h, --help            Show this help message.",
            ]
        )
    )
    return 0


def _ensure_windows() -> None:
    if platform.system() != "Windows":
        print("stop_win.py only supports Windows.")
        sys.exit(1)


def _delete_keepalive_task() -> None:
    query = start_win._run_captured(["schtasks", "/Query", "/TN", start_win.TASK_NAME])
    if query.returncode != 0:
        print(f"Scheduled task {start_win.TASK_NAME} not found")
        return

    result = start_win._run_captured(
        ["schtasks", "/Delete", "/TN", start_win.TASK_NAME, "/F"]
    )
    output = start_win._command_output(result)
    if result.returncode != 0:
        print(
            f"Failed to delete scheduled task {start_win.TASK_NAME}: "
            f"{output or f'exit code {result.returncode}'}"
        )
        sys.exit(1)
    print(output or f"Scheduled task deleted: {start_win.TASK_NAME}")


def _delete_run_key() -> None:
    import winreg

    try:
        with winreg.OpenKey(
            winreg.HKEY_CURRENT_USER,
            start_win.RUN_KEY,
            0,
            winreg.KEY_SET_VALUE,
        ) as key:
            winreg.DeleteValue(key, start_win.RUN_VALUE_NAME)
        print(f"Startup Run key removed: HKCU\\{start_win.RUN_KEY}\\{start_win.RUN_VALUE_NAME}")
    except FileNotFoundError:
        print(f"Startup Run key not found: HKCU\\{start_win.RUN_KEY}\\{start_win.RUN_VALUE_NAME}")


def main() -> int:
    args = sys.argv[1:]
    if any(arg in {"-h", "--help"} for arg in args):
        return _print_help()

    _ensure_windows()
    print("=== BuckyOS Windows Scheduled-Task Stop ===")
    _delete_keepalive_task()
    _delete_run_key()
    stop.kill_all()
    print("=== BuckyOS Windows Stop Complete ===")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
