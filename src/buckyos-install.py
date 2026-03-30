#!/usr/bin/env -S uv run

import os
import shutil
import subprocess
import sys
from pathlib import Path


DEVKIT_SPEC = "buckyos-devkit @ git+https://github.com/buckyos/buckyos-devkit.git"
SCRIPT_DIR = Path(__file__).resolve().parent
PROJECT_CONFIG = SCRIPT_DIR / "bucky_project.yaml"


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


def _print_help() -> int:
    script_path = SCRIPT_DIR / "buckyos-install.py"
    print(
        "\n".join(
            [
                "BuckyOS install wrapper",
                "",
                "Run the devkit `buckyos-install` command from the repository `src/`",
                "directory so `bucky_project.yaml` is resolved correctly.",
                "",
                "Usage:",
                f"  uv run {script_path.relative_to(SCRIPT_DIR.parent)} [args]",
                f"  cd {SCRIPT_DIR} && uv run ./buckyos-install.py [args]",
                "",
                "Common examples:",
                f"  uv run {script_path.relative_to(SCRIPT_DIR.parent)} --all",
                f"  uv run {script_path.relative_to(SCRIPT_DIR.parent)} --app=buckyos --all",
                f"  uv run {script_path.relative_to(SCRIPT_DIR.parent)} --app=buckyos --target-rootfs=/tmp/buckyos",
                "",
                "README first install flow:",
                f"  cd {SCRIPT_DIR}",
                "  uv run ./buckyos-build.py",
                "  uv run ./buckyos-install.py --all",
            ]
        )
    )
    return 0


def main() -> int:
    args = sys.argv[1:]
    if any(arg in {"-h", "--help"} for arg in args):
        return _print_help()

    if not PROJECT_CONFIG.exists():
        print(f"Missing project config: {PROJECT_CONFIG}")
        return 1

    executable = _find_command("buckyos-install")
    if executable is None:
        print("buckyos-install not found in the current uv runtime.")
        print(f"Please re-run this script with `uv run buckyos-install.py ...`")
        print(f"or install `{DEVKIT_SPEC}`.")
        return 127

    print(f"Running buckyos-install in {SCRIPT_DIR}")
    result = subprocess.run([executable] + args, cwd=SCRIPT_DIR, env=os.environ.copy())
    if result.returncode != 0:
        print(f"buckyos-install failed with return code {result.returncode}")

    return result.returncode


if __name__ == "__main__":
    raise SystemExit(main())
