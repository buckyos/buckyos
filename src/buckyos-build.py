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


def main(argv: list[str] | None = None) -> int:
    args = sys.argv[1:] if argv is None else list(argv)
    executable = _find_command("buckyos-build")
    if executable is None:
        print("buckyos-build not found in the current uv runtime.")
        print(f"Install `{DEVKIT_SPEC}` and try again.")
        return 127

    return subprocess.run(
        [executable, *args],
        env=os.environ.copy(),
    ).returncode


if __name__ == "__main__":
    raise SystemExit(main())
