#!/usr/bin/env -S uv run

import json
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


BUILD_HELP = """usage: buckyos-build.py [-h] [--agent-daemon] [--skip-web]
                        [--timings] [--timings-dir DIR]
                        [--app APP [APP ...]] [-s [MODULE ...]]
                        [--target=TARGET] [amd64|aarch64]

Prepare the SDK/CLI and agent tools, build BuckyOS modules via devkit,
then assemble the macOS runtime bundle.

options:
  -h, --help             Show this help and exit without building anything
  --agent-daemon         Build only buckyos_agent_daemon; skip SDK/CLI, agent
                         tools and runtime assembly. Cannot combine with -s,
                         --select or --app
  --skip-web             Skip web modules (still prepares SDK/CLI)
  --timings              Generate Cargo timings reports
  --timings-dir DIR      Generate Cargo timings reports and copy them to DIR
  --app APP [APP ...]    Build modules used by the specified apps;
                         app names may also be comma-separated
  -s, --select [MODULE ...]
                         Select modules from bucky_project.yaml; omit names
                         for interactive selection in a terminal
  --target=TARGET        Override the Rust target triple
  amd64                  Build for x86_64-unknown-linux-musl
  aarch64                Build for aarch64-unknown-linux-musl

SDK/CLI environment:
  BUCKYOS_SDK_TOOL_SOURCE
      Local buckyos-websdk checkout (defaults to the sibling repository).
      If absent, use published buckyos@latest; local build failures stop.
  BUCKYOS_SDK_TOOL_DENO
      Deno executable path (otherwise discovered on PATH for local builds).
  BUCKYOS_SDK_TOOL_ARTIFACT, BUCKYOS_SDK_TOOL_RELEASE_MANIFEST,
  BUCKYOS_SDK_TOOL_SBOM
      Prebuilt distribution inputs. Setting any of these requires all three
      plus BUCKYOS_SDK_TOOL_DENO. Setting Deno alone keeps source/published mode.

Examples:
  uv run buckyos-build.py -s scheduler
  uv run buckyos-build.py --skip-web
  uv run buckyos-build.py --agent-daemon --target=x86_64-unknown-linux-musl

Other arguments are forwarded unchanged to devkit's buckyos-build.
"""


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


def _build_local_sdk_tool_distribution(env: dict[str, str], source: Path) -> int:
    import yaml

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


def _build_published_sdk_tool_distribution(env: dict[str, str]) -> int:
    print("Preparing SDK/Tool from published buckyos@latest", flush=True)
    with tempfile.TemporaryDirectory(prefix="buckyos-sdk-tool-release-") as temporary:
        command = [
            sys.executable,
            str(Path(__file__).parent / "tools" / "prepare_sdk_tool_distribution.py"),
            "--work-dir",
            temporary,
        ]
        result = subprocess.run(command, env=env).returncode
    if result != 0:
        print(f"Published SDK/Tool preparation failed with return code {result}")
    return result


def _prepare_sdk_tool_distribution(env: dict[str, str]) -> int:
    artifact_inputs = set(SDK_TOOL_INPUTS) - {"BUCKYOS_SDK_TOOL_DENO"}
    if any(env.get(name) for name in artifact_inputs):
        return _build_sdk_tool_distribution(env)
    source = Path(env.get(
        "BUCKYOS_SDK_TOOL_SOURCE",
        str(Path(__file__).resolve().parents[2] / "buckyos-websdk"),
    )).expanduser().resolve()
    if source.exists():
        return _build_local_sdk_tool_distribution(env, source)
    print(f"SDK/Tool source not found: {source}; using the published package.")
    return _build_published_sdk_tool_distribution(env)


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


# Shared TypeScript modules the buckyos-agent commands import from the source tree (`../../frame/…` in its
# deno.json): copied beside them as `vendor/frame/…`, so the readable sources travel with the commands.
AGENT_TOOL_SHARED_MODULES = ("frame/aiworkspace/import-table", "frame/aiworkspace/import-drawio")


def _prepare_agent_tools(source: Path | None = None, target: Path | None = None, src_root: Path | None = None) -> int:
    """Host copy of the buckyos-agent commands (libexec/buckyos-agent): the
    standard AICC commands a host session-runtime runs on the Tool's Deno,
    importing the Tool's SDK beside it."""
    here = Path(__file__).resolve().parent
    source = source or here / "tools" / "buckyos-agent"
    target = target or here / "rootfs" / "libexec" / "buckyos-agent"
    src_root = src_root or here
    if not (source / "aicc-tool.ts").is_file():
        print(f"buckyos-agent sources not found: {source}")
        return 2
    if target.exists():
        shutil.rmtree(target)
    shutil.copytree(source, target)
    for module in AGENT_TOOL_SHARED_MODULES:
        if (src_root / module).is_dir():
            shutil.copytree(src_root / module, target / "vendor" / module, ignore=shutil.ignore_patterns("*_test.ts", "README.md"))
    config_path = target / "deno.json"
    config = json.loads(config_path.read_text(encoding="utf-8"))
    imports = config.setdefault("imports", {})
    imports["buckyos"] = "../buckyos-tool/dist/node.mjs"
    for key, value in list(imports.items()):
        if isinstance(value, str) and value.startswith("../../frame/"):
            imports[key] = "./vendor/" + value[len("../../"):]
    config_path.write_text(json.dumps(config, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return 0


def main(argv: list[str] | None = None) -> int:
    args = sys.argv[1:] if argv is None else list(argv)
    if any(arg in ("-h", "--help") for arg in args):
        print(BUILD_HELP)
        return 0

    standalone_daemon = "--agent-daemon" in args
    if standalone_daemon:
        args.remove("--agent-daemon")
        if any(arg in ("--select", "-s", "--app") for arg in args):
            print("--agent-daemon cannot be combined with module or app selection.")
            return 2
        args.extend(["--select", "buckyos_agent_daemon"])
    executable = _find_command("buckyos-build")
    if executable is None:
        print("buckyos-build not found in the current uv runtime.")
        print(f"Install `{DEVKIT_SPEC}` and try again.")
        return 127

    env = os.environ.copy()
    if not standalone_daemon:
        result = _prepare_sdk_tool_distribution(env)
        if result != 0:
            return result
        result = _prepare_agent_tools()
        if result != 0:
            return result

    env["BUCKYOS_PIKG_COMMAND"] = json.dumps([
        _find_command("node") or "node",
        str(Path(__file__).resolve().parent / "rootfs" / "libexec" / "buckyos-tool" / "cli" / "launcher.mjs"),
    ])
    result = subprocess.run(
        [executable, *args],
        env=env,
    ).returncode
    if result != 0 or standalone_daemon:
        return result
    return _assemble_macos_runtime(env)


def _assemble_macos_runtime(env: dict[str, str]) -> int:
    """Move a freshly built macOS node_daemon into the BuckyOS Runtime bundle."""
    return subprocess.run(
        [sys.executable, str(Path(__file__).resolve().parent / "tools" / "macos_runtime_bundle.py"), "assemble"],
        env=env,
    ).returncode


if __name__ == "__main__":
    raise SystemExit(main())
