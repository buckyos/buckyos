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


def _run_command(command: str, args: list[str]) -> int | None:
    executable = _find_command(command)
    if executable is None:
        return None

    result = subprocess.run([executable] + args, env=os.environ.copy())
    return result.returncode


def _run_with_uv(args: list[str]) -> int | None:
    uv = shutil.which("uv")
    script = Path(__file__).with_name("buckyos-build.py")
    if uv is None or not script.exists():
        return None

    result = subprocess.run([uv, "run", str(script)] + args, env=os.environ.copy())
    return result.returncode


def main() -> int:
    build_executable = _find_command("buckyos-build")
    if build_executable is None:
        result = _run_with_uv(sys.argv[1:])
        if result is not None:
            return result

        print("buckyos-build not found in the current environment")
        print("Install buckyos-devkit first, or use the repo uv runtime:")
        print("  uv run src/buckyos-build.py [args]")
        print(f'  python3 -m pip install -U "{DEVKIT_SPEC}"')
        return 1

    print("!!! buckyos depend on cyfs-gateway, MAKE SURE YOU HAVE BUILD IT FIRST!", flush=True)
    result = subprocess.run([build_executable] + sys.argv[1:], env=os.environ.copy()).returncode

    if result != 0:
        print(f"buckyos-build failed with return code {result}")
        return result

    result = _run_command("buckyos-update", [])
    if result is None:
        print("buckyos-update not found in the current environment")
        print(f'Please ensure "{DEVKIT_SPEC}" is installed correctly.')
        return 1

    if result != 0:
        print(f"buckyos-update failed with return code {result}")
        return result

    print("buckyos-build and buckyos-update completed successfully")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

