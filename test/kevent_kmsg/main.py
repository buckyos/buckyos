#!/usr/bin/env python3

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
CASES = {
    "dv": "kevent_kmsg/dv",
    "task_mgr": "kevent_kmsg/task_mgr",
    "restart": "kevent_kmsg/restart",
    "peer_container": "kevent_kmsg/peer_container",
    "peer_vm": "kevent_kmsg/peer_vm",
}


def print_help() -> None:
    print("kevent/kmsg grouped test entry")
    print("")
    print("Run an individual subcase from the repository root:")
    for name, path in CASES.items():
        print(f"  uv run test/run.py -p {path}  # {name}")
    print("")
    print("To run multiple subcases through this grouped entry:")
    print("  BUCKYOS_KEVENT_KMSG_CASES=dv,restart uv run test/run.py -p kevent_kmsg")
    print("")
    print("Per-run detailed reports are kept in test/kevent_kmsg/reports/.")


def main() -> int:
    selected = os.environ.get("BUCKYOS_KEVENT_KMSG_CASES", "").strip()
    if not selected:
        print_help()
        return 0

    names = [item.strip() for item in selected.split(",") if item.strip()]
    unknown = [name for name in names if name not in CASES]
    if unknown:
        print(f"unknown kevent/kmsg test case(s): {', '.join(unknown)}", file=sys.stderr)
        return 2

    for name in names:
        result = subprocess.run(
            [sys.executable, "test/run.py", "-p", CASES[name]],
            cwd=ROOT,
            text=True,
        )
        if result.returncode != 0:
            return result.returncode

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
