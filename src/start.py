#!/usr/bin/env -S uv run

import argparse
import json
from contextlib import contextmanager
import os
import shutil
import subprocess
import sys
import platform
import tempfile
from pathlib import Path


DEVKIT_SPEC = "buckyos-devkit @ git+https://github.com/buckyos/buckyos-devkit.git@main"
SCRIPT_DIR = Path(__file__).resolve().parent
SDK_TOOL_DIR = SCRIPT_DIR / "rootfs" / "libexec" / "buckyos-tool"

build_dir = os.path.dirname(os.path.abspath(__file__))

# after run build.py ,use this script to restart the dev buckyos system
# 1) killall process
# 2) update files to /opt/buckyos (--all to update all files to /opt/buckyos)
# 3) start the system (node_daemon; on macOS the BuckyOS Runtime LaunchAgent)


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
    script_path = SCRIPT_DIR / "start.py"
    print(
        "\n".join(
            [
                "BuckyOS development startup helper",
                "",
                "Stops existing BuckyOS processes, updates installed files by default,",
                "and starts node_daemon from BUCKYOS_ROOT (on macOS in a GUI session:",
                "binds the root to you and starts the BuckyOS Runtime LaunchAgent).",
                "",
                "Usage:",
                f"  uv run {script_path.relative_to(SCRIPT_DIR.parent)} [options]",
                f"  cd {SCRIPT_DIR} && uv run ./start.py [options]",
                "",
                "Options:",
                "  --all                 Fresh dev install before startup; preserve etc/devenv.json.",
                "  --reinstall [GROUP]   Fresh install before startup, optionally running make_config.ts GROUP.",
                "  --skip-update         Skip file update and only restart BuckyOS.",
                "  --edition full|lite   With --all/--reinstall GROUP: the product edition to install",
                "                        (lite = BuckyOS Desktop Lite, no containers; default full).",
                "  -h, --help            Show this help message.",
                "",
                "Common examples:",
                f"  uv run {script_path.relative_to(SCRIPT_DIR.parent)}",
                f"  uv run {script_path.relative_to(SCRIPT_DIR.parent)} --all",
                f"  uv run {script_path.relative_to(SCRIPT_DIR.parent)} --all --edition lite",
                f"  uv run {script_path.relative_to(SCRIPT_DIR.parent)} --reinstall release",
                f"  uv run {script_path.relative_to(SCRIPT_DIR.parent)} --skip-update",
            ]
        )
    )
    return 0


def _sdk_tool_distribution_ready() -> bool:
    runtime_name = "deno.exe" if os.name == "nt" else "deno"
    required = (
        SDK_TOOL_DIR / "distribution.json",
        SDK_TOOL_DIR / "cli" / "system_bootstrap.ts",
        SDK_TOOL_DIR / "runtime" / runtime_name,
    )
    missing = [path for path in required if not path.is_file()]
    if not missing:
        return True

    print(f"Missing generated SDK/Tool distribution: {SDK_TOOL_DIR}")
    print("Missing required files:")
    for path in missing:
        print(f"  - {path.relative_to(SCRIPT_DIR)}")
    print(
        "Generate it first from the local websdk source with `uv run buckyos-build.py`."
    )
    return False


def _run_command(command: str, args: list[str]) -> int:
    executable = _find_command(command)
    if executable is None:
        print(f"{command} not found in the current uv runtime.")
        print("Please re-run this script with `uv run src/start.py ...`")
        print(f"or install `{DEVKIT_SPEC}`.")
        return 127

    return subprocess.run(
        [executable] + args,
        env=os.environ.copy(),
        **_windows_subprocess_kwargs(),
    ).returncode


def _run_make_config(config_group_name: str, target_root: Path, edition: str | None = None) -> None:
    executable = _find_command("deno")
    if executable is None:
        raise RuntimeError("deno not found. make_config.ts requires Deno >= 2.2")

    env = os.environ.copy()
    env["BUCKYOS_ROOT"] = str(target_root)
    result = subprocess.run(
        [
            executable,
            "run",
            "-A",
            "make_config.ts",
            config_group_name,
            "--rootfs",
            str(target_root),
        ]
        + (["--edition", edition] if edition else []),
        cwd=build_dir,
        env=env,
        **_windows_subprocess_kwargs(),
    ).returncode
    if result != 0:
        raise RuntimeError(f"make_config.ts failed with return code {result}")


def _windows_subprocess_kwargs(detached: bool = False) -> dict[str, object]:
    if platform.system() != "Windows":
        return {}

    startupinfo = subprocess.STARTUPINFO()
    startupinfo.dwFlags |= subprocess.STARTF_USESHOWWINDOW
    startupinfo.wShowWindow = subprocess.SW_HIDE

    creationflags = subprocess.CREATE_NO_WINDOW
    if detached:
        creationflags |= subprocess.DETACHED_PROCESS | subprocess.CREATE_NEW_PROCESS_GROUP

    return {
        "startupinfo": startupinfo,
        "creationflags": creationflags,
    }


def _spawn_background(args: list[str], env: dict[str, str]) -> int:
    if platform.system() == "Windows":
        proc = subprocess.Popen(
            args,
            env=env,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            close_fds=True,
            **_windows_subprocess_kwargs(detached=True),
        )
        return proc.pid

    proc = subprocess.Popen(
        args,
        env=env,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        preexec_fn=os.setsid,
        close_fds=True,
    )
    return proc.pid


def resolve_buckyos_root() -> Path:
    """Resolve BUCKYOS_ROOT using env first, then platform defaults."""
    buckyos_root = os.environ.get("BUCKYOS_ROOT")
    if buckyos_root:
        print(f"Using BUCKYOS_ROOT: {buckyos_root}")
        return Path(buckyos_root).expanduser()

    if platform.system() == "Windows":
        default_root = os.path.join(os.path.expandvars("%AppData%"), "buckyos")
    else:
        default_root = "/opt/buckyos"

    print(f"BUCKYOS_ROOT not set, using default: {default_root}")
    return Path(default_root).expanduser()


@contextmanager
def _preserve_devenv_config(enabled: bool):
    if not enabled:
        yield None
        return

    devenv_path = resolve_buckyos_root() / "etc" / "devenv.json"
    if not devenv_path.is_file():
        yield None
        return

    with tempfile.TemporaryDirectory(prefix="buckyos-devenv-") as temporary:
        backup_path = Path(temporary) / "devenv.json"
        shutil.copy2(devenv_path, backup_path)
        try:
            yield devenv_path
        finally:
            devenv_path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(backup_path, devenv_path)


def kill_all_processes():
    """Kill all related BuckyOS processes"""
    print("Stopping all BuckyOS processes...")
    
    # Import and execute killall.py functions directly
    try:
        import stop
        # Execute the main logic of killall.py
        stop.kill_all()
        print("All processes stopped")
    except ImportError as e:
        print(f"Failed to import killall module: {e}")
        sys.exit(1)
    except Exception as e:
        print(f"Warning: Some processes may not have been stopped: {e}")
        # Continue execution even if some processes fail to stop

def update_files(install_all=False,config_group_name=None,edition=None):
    """Update files to installation directory"""
    print("Updating files...")

    try:
        install_args = ["--app=buckyos"]
        if install_all:
            install_args.append("--all")

        result = _run_command("buckyos-update", install_args)
        if result != 0:
            raise RuntimeError(f"buckyos-update failed with return code {result}")

        if config_group_name:
           target_root = resolve_buckyos_root()
           _run_make_config(config_group_name, target_root, edition)
        print("Files updated successfully")
    except Exception as e:
        print(f"Failed to update files: {e}")
        sys.exit(1)


    

def _missing_paios_images(buckyos_root: Path) -> list[str]:
    """paios images node_daemon needs but cannot find, in local image mode.

    Release installs pin the images in bin/paios-images/ and load them from the
    bundled tarballs; dev installs use <repo>:latest-<arch> from the local store
    (repo overridable in etc/devenv.json). node_daemon never pulls either.
    """
    if (buckyos_root / "bin" / "paios-images" / "paios_images.json").is_file():
        return []
    docker = _find_command("docker")
    if docker is None:
        return []
    arch = "aarch64" if platform.machine().lower() in ("arm64", "aarch64") else "amd64"
    repos = {"aios": "paios/aios", "exttool": "paios/exttool"}
    try:
        overrides = json.loads((buckyos_root / "etc" / "devenv.json").read_text(encoding="utf-8"))
    except (OSError, ValueError):
        overrides = {}
    if isinstance(overrides, dict):
        for name in repos:
            value = overrides.get(name)
            if isinstance(value, str) and value.strip():
                repos[name] = value.strip()
    missing = []
    for repo in repos.values():
        reference = f"{repo}:latest-{arch}"
        inspected = subprocess.run(
            [docker, "image", "inspect", reference],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            **_windows_subprocess_kwargs(),
        )
        if inspected.returncode != 0:
            missing.append(reference)
    return missing


def _warn_missing_paios_images(buckyos_root: Path) -> None:
    missing = _missing_paios_images(buckyos_root)
    if not missing:
        return
    print("")
    print("!" * 72)
    print("WARNING: paios images missing; Agents and script apps will fail to deploy.")
    print("node_daemon does not pull them. Get them with one of:")
    for reference in missing:
        print(f"  docker pull {reference}")
    print("  uv run build.py            (from the repository root; pre-pulls them)")
    print("  ./build_aios --local-test  (build the worker image from local source)")
    print("!" * 72)


MACOS_RUNTIME_EXECUTABLE = Path("bin", "node-daemon", "BuckyOS Runtime.app", "Contents", "MacOS", "node_daemon")


def node_daemon_path(buckyos_root: Path) -> Path:
    if platform.system() == "Darwin":
        return buckyos_root / MACOS_RUNTIME_EXECUTABLE
    suffix = ".exe" if platform.system() == "Windows" else ""
    return buckyos_root / "bin" / "node-daemon" / f"node_daemon{suffix}"


def _macos_gui_session() -> bool:
    """A non-root user with an Aqua launchd domain (logged in at the console
    or by fast user switching); sudo, SSH-only and CI root runs are not."""
    if os.geteuid() == 0:
        return False
    return subprocess.run(
        ["launchctl", "print", f"gui/{os.getuid()}"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    ).returncode == 0


def _start_macos_runtime(executable: Path, buckyos_root: Path) -> None:
    """Bind the root to this user and start it as the user's LaunchAgent, the
    same Runtime path a package install uses."""
    user = os.environ.get("USER") or subprocess.run(
        ["id", "-un"], capture_output=True, text=True, check=True
    ).stdout.strip()
    for command in (
        ["service", "register", "--user", user, "--root", str(buckyos_root)],
        ["service", "start", "--root", str(buckyos_root)],
    ):
        result = subprocess.run([str(executable), *command])
        if result.returncode != 0:
            raise RuntimeError(f"{executable.name} {' '.join(command[:2])} failed with return code {result.returncode}")
    print(f"BuckyOS Runtime started as {user}'s LaunchAgent (com.buckyos.runtime)")


def start_system():
    """Start BuckyOS system"""
    print("Starting BuckyOS system...")
    buckyos_root = resolve_buckyos_root()
    executable = node_daemon_path(buckyos_root)

    if not executable.exists():
        print(f"Error: Cannot find node_daemon executable: {executable}")
        print(f"Please check if the installation directory is correct: {buckyos_root}")
        sys.exit(1)

    try:
        if platform.system() == "Darwin" and _macos_gui_session():
            _start_macos_runtime(executable, buckyos_root)
        else:
            env = os.environ.copy()
            env['BUCKYOS_ROOT'] = str(buckyos_root)
            args = [str(executable), "--enable_active"]
            if platform.system() == "Darwin":
                print("Warning: not a macOS GUI session (root, sudo or SSH); starting node_daemon directly.")
                print("Desktop control and the login item are unavailable for this run.")
            pid = _spawn_background(args, env)
            print(f"BuckyOS system started: {executable}")
            print(f"node_daemon pid: {pid}")
        print("System is running in background...")
        _warn_missing_paios_images(buckyos_root)

    except Exception as e:
        print(f"Failed to start system: {e}")
        sys.exit(1)

def main(argv: list[str] | None = None) -> int:
    """Main function"""
    args = sys.argv[1:] if argv is None else list(argv)
    if any(arg in {"-h", "--help"} for arg in args):
        return _print_help()

    parser = argparse.ArgumentParser(allow_abbrev=False, add_help=False)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--all", action="store_true")
    mode.add_argument("--reinstall", nargs="?", const="", metavar="GROUP")
    mode.add_argument("--skip-update", action="store_true")
    parser.add_argument("--edition", choices=("full", "lite"))
    try:
        options = parser.parse_args(args)
    except SystemExit as error:
        return error.code

    print("=== BuckyOS Development Environment Startup Script ===")
    
    config_group_name = None
    install_all = options.all or options.reinstall is not None
    need_update = not options.skip_update
    if options.all:
        config_group_name = "dev"
    if options.reinstall is not None:
        config_group_name = options.reinstall or None
    if options.edition and not config_group_name:
        print("--edition needs --all or --reinstall GROUP: the edition is part of a fresh install")
        return 2

    if need_update and not _sdk_tool_distribution_ready():
        return 2
    
    with _preserve_devenv_config(options.all) as preserved_devenv:
        # Step 1: Kill all processes
        kill_all_processes()

        if install_all or need_update:
            # Step 2: Update files
            update_files(install_all,config_group_name,options.edition)
    
    # Step 3: Start system
    start_system()
    
    print("=== BuckyOS Startup Complete ===")
    if preserved_devenv is not None:
        print(f"Note: Preserved local development config: {preserved_devenv}")
        print(
            "Local image overrides in this file still apply. "
            "Remove the aios/exttool overrides and restart to use the default images."
        )
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
