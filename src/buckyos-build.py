#!/usr/bin/env -S uv run

import os
import shutil
import subprocess
import sys
from pathlib import Path


DEVKIT_SPEC = "buckyos-devkit @ git+https://github.com/buckyos/buckyos-devkit.git"


def _command_names(command: str) -> list[str]:
    if os.name == "nt":
        return [f"{command}.exe", f"{command}.cmd", f"{command}.bat", command]
    return [command]


def _find_command(command: str) -> str | None:
    for name in _command_names(command):
        path = shutil.which(name)
        if path is not None:
            return path

    bin_dir = Path(sys.executable).parent
    for name in _command_names(command):
        candidate = bin_dir / name
        if candidate.exists():
            return str(candidate)

    return None


def _run_command(command: str, args: list[str]) -> int:
    executable = _find_command(command)
    if executable is None:
        print(f"{command} not found in the current uv runtime.")
        print(f"Please re-run this script with `uv run src/buckyos-build.py ...` or install `{DEVKIT_SPEC}`.")
        return 127

    result = subprocess.run([executable] + args, env=os.environ.copy())
    return result.returncode


def main() -> int:
    print("!!! buckyos depend on cyfs-gateway, MAKE SURE YOU HAVE BUILD IT FIRST!", flush=True)

    result = _run_command("buckyos-build", sys.argv[1:])
    if result != 0:
        print(f"buckyos-build failed with return code {result}")
        return result

    result = _run_command("buckyos-update", [])
    if result != 0:
        print(f"buckyos-update failed with return code {result}")
        return result

    print("buckyos-build and buckyos-update completed successfully")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
