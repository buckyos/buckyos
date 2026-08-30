#!/usr/bin/env -S uv run

import os
import shutil
import subprocess
import sys
import platform
from pathlib import Path


DEVKIT_SPEC = "buckyos-devkit @ git+https://github.com/buckyos/buckyos-devkit.git"
SCRIPT_DIR = Path(__file__).resolve().parent
SDK_TOOL_DIR = SCRIPT_DIR / "rootfs" / "libexec" / "buckyos-tool"

build_dir = os.path.dirname(os.path.abspath(__file__))

# after run build.py ,use this script to restart the dev buckyos system
# 1) killall process
# 2) update files to /opt/buckyos (--all to update all files to /opt/buckyos)
# 3) start the system (run /opt/buckyos/bin/node_daemon/node_daemon)


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
                "and starts node_daemon from BUCKYOS_ROOT.",
                "",
                "Usage:",
                f"  uv run {script_path.relative_to(SCRIPT_DIR.parent)} [options]",
                f"  cd {SCRIPT_DIR} && uv run ./start.py [options]",
                "",
                "Options:",
                "  --all                 Fresh dev install before startup.",
                "  --reinstall [GROUP]   Fresh install before startup, optionally running make_config.ts GROUP.",
                "  --skip-update         Skip file update and only restart BuckyOS.",
                "  -h, --help            Show this help message.",
                "",
                "Common examples:",
                f"  uv run {script_path.relative_to(SCRIPT_DIR.parent)}",
                f"  uv run {script_path.relative_to(SCRIPT_DIR.parent)} --all",
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
        "Generate it first with `uv run buckyos-build.py` and the four "
        "BUCKYOS_SDK_TOOL_* input paths."
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


def _run_make_config(config_group_name: str, target_root: Path) -> None:
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
        ],
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

def update_files(install_all=False,config_group_name=None):
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
           _run_make_config(config_group_name, target_root)
        print("Files updated successfully")
    except Exception as e:
        print(f"Failed to update files: {e}")
        sys.exit(1)


    

def start_system():
    """Start BuckyOS system"""
    print("Starting BuckyOS system...")
    buckyos_root = str(resolve_buckyos_root())
    
    # Start node_daemon
    node_daemon_path = os.path.join(buckyos_root, "bin", "node-daemon", "node_daemon")
    
    if platform.system() == "Windows":
        node_daemon_path += ".exe"
    
    if not os.path.exists(node_daemon_path):
        print(f"Error: Cannot find node_daemon executable: {node_daemon_path}")
        print(f"Please check if the installation directory is correct: {buckyos_root}")
        sys.exit(1)
    
    try:
        # Start node_daemon in background with BUCKYOS_ROOT environment
        env = os.environ.copy()
        env['BUCKYOS_ROOT'] = buckyos_root

        pid = _spawn_background([node_daemon_path, "--enable_active"], env)
        print(f"BuckyOS system started: {node_daemon_path}")
        print(f"node_daemon pid: {pid}")
        print("System is running in background...")
        
    except Exception as e:
        print(f"Failed to start system: {e}")
        sys.exit(1)

def main(argv: list[str] | None = None) -> int:
    """Main function"""
    args = sys.argv[1:] if argv is None else list(argv)
    if any(arg in {"-h", "--help"} for arg in args):
        return _print_help()

    print("=== BuckyOS Development Environment Startup Script ===")
    
    # Parse command line arguments
    config_group_name = None
    install_all = "--all" in args or "--reinstall" in args
    need_update = "--skip-update" not in args
    if install_all:
        config_group_name = "dev"
    if "--reinstall" in args:
        config_group_name = None
        group_name_index = args.index("--reinstall") + 1
        if group_name_index < len(args):
            config_group_name = args[group_name_index]

    if need_update and not _sdk_tool_distribution_ready():
        return 2
    
    # Step 1: Kill all processes
    kill_all_processes()
    
    if install_all or need_update:
        # Step 2: Update files
        update_files(install_all,config_group_name)
    
    # Step 3: Start system
    start_system()
    
    print("=== BuckyOS Startup Complete ===")
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
