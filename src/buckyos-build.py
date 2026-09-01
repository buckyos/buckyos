#!/usr/bin/env -S uv run

import os
import shutil
import subprocess
import sys
from pathlib import Path


DEVKIT_SPEC = "buckyos-devkit @ git+https://github.com/buckyos/buckyos-devkit.git"
SDK_TOOL_INPUTS = {
    "BUCKYOS_SDK_TOOL_ARTIFACT": "--artifact",
    "BUCKYOS_SDK_TOOL_RELEASE_MANIFEST": "--release-manifest",
    "BUCKYOS_SDK_TOOL_DENO": "--deno",
    "BUCKYOS_SDK_TOOL_SBOM": "--sbom",
}


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


def _prepare_sdk_tool_distribution(env: dict[str, str]) -> int:
    missing = [name for name in SDK_TOOL_INPUTS if not env.get(name)]
    if missing:
        print(
            "Missing immutable SDK/Tool build inputs: " + ", ".join(missing)
        )
        print(
            "Set all four BUCKYOS_SDK_TOOL_* paths before running "
            "buckyos-build.py."
        )
        return 2

    command = [
        sys.executable,
        str(Path(__file__).parent / "tools" / "build_sdk_tool_distribution.py"),
    ]
    for name, option in SDK_TOOL_INPUTS.items():
        command.extend([option, env[name]])
    if os.name == "nt":
        command.append("--windows")

    result = subprocess.run(command, env=env).returncode
    if result != 0:
        print(f"SDK/Tool distribution build failed with return code {result}")
    return result


def main(argv: list[str] | None = None) -> int:
    args = sys.argv[1:] if argv is None else list(argv)
    executable = _find_command("buckyos-build")
    if executable is None:
        print("buckyos-build not found in the current uv runtime.")
        print(f"Install `{DEVKIT_SPEC}` and try again.")
        return 127

    env = os.environ.copy()
    result = _prepare_sdk_tool_distribution(env)
    if result != 0:
        return result

    return subprocess.run(
        [executable, *args],
        env=env,
    ).returncode


if __name__ == "__main__":
    raise SystemExit(main())
