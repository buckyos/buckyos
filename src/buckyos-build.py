#!/usr/bin/env -S uv run

import os
import shutil
import subprocess
import sys
import tempfile
import uuid
from pathlib import Path


DEVKIT_SPEC = "buckyos-devkit @ git+https://github.com/buckyos/buckyos-devkit.git@main"
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


def _build_local_sdk_tool_distribution(env: dict[str, str]) -> int:
    import yaml

    source = Path(env.get(
        "BUCKYOS_SDK_TOOL_SOURCE",
        str(Path(__file__).resolve().parents[2] / "buckyos-websdk"),
    )).expanduser().resolve()
    if not (source / "package.json").is_file():
        print(f"SDK/Tool source not found: {source}")
        print("Set BUCKYOS_SDK_TOOL_SOURCE to the local buckyos-websdk checkout.")
        return 2

    commands = {name: _find_command(name) for name in ("pnpm", "npm", "node")}
    deno = env.get("BUCKYOS_SDK_TOOL_DENO") or _find_command("deno")
    missing = [name for name, command in commands.items() if command is None]
    if deno is None:
        missing.append("deno")
    if missing:
        print("Missing SDK/Tool build commands: " + ", ".join(missing))
        return 127
    deno = str(Path(deno).expanduser().resolve())
    project = yaml.safe_load(
        (Path(__file__).parent / "bucky_project.yaml").read_text(encoding="utf-8")
    )
    print(f"Building SDK/Tool from local source: {source}", flush=True)
    with tempfile.TemporaryDirectory(prefix="buckyos-sdk-tool-build-") as temporary:
        output = Path(temporary)
        sbom = output / "sbom.json"
        manifest = output / "release.json"
        steps = [
            [commands["pnpm"], "install", "--frozen-lockfile"],
            [commands["pnpm"], "run", "build"],
            [commands["npm"], "pack", "--ignore-scripts", "--pack-destination", temporary],
        ]
        for command in steps:
            result = subprocess.run(command, cwd=source, env=env).returncode
            if result != 0:
                return result
        artifacts = list(output.glob("*.tgz"))
        if len(artifacts) != 1:
            print("SDK/Tool pack must produce exactly one npm tarball.")
            return 2
        steps = [
            [commands["node"], "scripts/create-sbom.mjs",
             "--tarball", str(artifacts[0]), "--deno", deno, "--output", str(sbom)],
            [commands["node"], "scripts/create-release-manifest.mjs",
             "--tarball", str(artifacts[0]), "--deno", deno, "--sbom", str(sbom),
             "--output", str(manifest), "--buckyos-version", str(project["version"]),
             "--build-id", f"dev-{uuid.uuid4()}"],
        ]
        for command in steps:
            result = subprocess.run(command, cwd=source, env=env).returncode
            if result != 0:
                return result
        return _build_sdk_tool_distribution({
            **env,
            "BUCKYOS_SDK_TOOL_ARTIFACT": str(artifacts[0]),
            "BUCKYOS_SDK_TOOL_RELEASE_MANIFEST": str(manifest),
            "BUCKYOS_SDK_TOOL_DENO": deno,
            "BUCKYOS_SDK_TOOL_SBOM": str(sbom),
        })


def _prepare_sdk_tool_distribution(env: dict[str, str]) -> int:
    artifact_inputs = set(SDK_TOOL_INPUTS) - {"BUCKYOS_SDK_TOOL_DENO"}
    if any(env.get(name) for name in artifact_inputs):
        return _build_sdk_tool_distribution(env)
    return _build_local_sdk_tool_distribution(env)


def _build_sdk_tool_distribution(env: dict[str, str]) -> int:
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
