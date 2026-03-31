#!/usr/bin/env -S uv run

from __future__ import annotations

import argparse
import getpass
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path


ROOT_DIR = Path(__file__).resolve().parent
SRC_DIR = ROOT_DIR / "src"
CHECK_SCRIPT = ROOT_DIR / "src" / "check.py"
DEVENV_SCRIPT = ROOT_DIR / "devenv.py"
BOOT_CONFIG = Path.home() / ".buckycli" / "buckyos_boot.toml"
BOOT_CONFIG_SAMPLE = SRC_DIR / "rootfs" / "etc" / "scheduler" / "buckyos_boot.toml.sample"
BOOT_TEMPLATE = SRC_DIR / "rootfs" / "etc" / "scheduler" / "boot.template.toml"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Build, install and start the devtest.buckyos.io local development flow.",
    )
    parser.add_argument(
        "--yes",
        action="store_true",
        help="Use default answers for confirmation prompts.",
    )
    parser.add_argument(
        "--non-interactive",
        action="store_true",
        help="Fail instead of prompting when user input is required.",
    )
    parser.add_argument(
        "--jarvis-agent",
        action="store_true",
        help="Force creation of ~/.buckycli/buckyos_boot.toml when it is missing.",
    )
    parser.add_argument(
        "--refresh-images",
        action="store_true",
        help="Pull docker images even if they already exist locally.",
    )
    parser.add_argument(
        "--skip-docker",
        action="store_true",
        help="Skip docker checks and image pulls.",
    )
    parser.add_argument(
        "--skip-clone",
        action="store_true",
        help="Skip cloning cyfs-gateway and buckyosapp.",
    )
    parser.add_argument(
        "--skip-root-build",
        action="store_true",
        help="Skip building/installing the current buckyos repo.",
    )
    parser.add_argument(
        "--skip-gateway-build",
        action="store_true",
        help="Skip building/installing ../cyfs-gateway.",
    )
    parser.add_argument(
        "--skip-start",
        action="store_true",
        help="Skip `uv run start.py --all`.",
    )
    parser.add_argument(
        "--skip-check",
        action="store_true",
        help="Skip the final runtime health check.",
    )
    parser.add_argument(
        "--wait-seconds",
        type=int,
        default=20,
        help="Seconds to wait before running check.py. Default: 20.",
    )
    return parser.parse_args()


def normalized_arch() -> str:
    raw = platform.machine().lower()
    if raw in {"x86_64", "amd64"}:
        return "amd64"
    if raw in {"arm64", "aarch64"}:
        return "aarch64"
    return "amd64"


def resolve_filebrowser_image(arch: str) -> str:
    fallback = f"buckyos/nightly-buckyos_filebrowser:0.5.1-{arch}"
    if not BOOT_TEMPLATE.exists():
        return fallback

    template = BOOT_TEMPLATE.read_text(encoding="utf-8")
    pattern = re.compile(
        rf'"{re.escape(arch)}_docker_image"\s*:\s*\{{.*?"docker_image_name"\s*:\s*"([^"]+)"',
        re.DOTALL,
    )
    match = pattern.search(template)
    if match is None:
        return fallback
    return match.group(1)


def default_docker_images() -> tuple[str, str]:
    arch = normalized_arch()
    return (
        "paios/aios:latest",
        resolve_filebrowser_image(arch),
    )


def json_string_fragment(value: str) -> str:
    return json.dumps(value)[1:-1]


def is_interactive(args: argparse.Namespace) -> bool:
    return not args.non_interactive and sys.stdin.isatty()


def print_step(title: str) -> None:
    print(f"\n==> {title}", flush=True)


def format_command(args: list[str]) -> str:
    return " ".join(subprocess.list2cmdline([part]) for part in args)


def run_command(
    args: list[str],
    *,
    cwd: Path | None = None,
    check: bool = True,
    capture_output: bool = False,
) -> subprocess.CompletedProcess[str]:
    print(f"$ {format_command(args)}", flush=True)
    result = subprocess.run(
        args,
        cwd=cwd,
        check=False,
        env=os.environ.copy(),
        text=True,
        capture_output=capture_output,
    )
    if check and result.returncode != 0:
        raise RuntimeError(
            f"command failed with exit code {result.returncode}: {format_command(args)}"
        )
    return result


def confirm(message: str, *, default: bool, args: argparse.Namespace) -> bool:
    if args.yes:
        return default

    if not is_interactive(args):
        raise RuntimeError(f"need interactive confirmation: {message}")

    suffix = " [Y/n] " if default else " [y/N] "
    raw = input(message + suffix).strip().lower()
    if not raw:
        return default
    if raw in {"y", "yes"}:
        return True
    if raw in {"n", "no"}:
        return False
    print("Please answer yes or no.", flush=True)
    return confirm(message, default=default, args=args)


def prompt_text(
    label: str,
    *,
    args: argparse.Namespace,
    secret: bool = False,
    default: str | None = None,
) -> str:
    if not is_interactive(args):
        raise RuntimeError(f"need interactive input for: {label}")

    while True:
        prompt = f"{label}"
        if default:
            prompt += f" [{default}]"
        prompt += ": "
        value = getpass.getpass(prompt) if secret else input(prompt)
        value = value.strip()
        if value:
            return value
        if default is not None:
            return default
        print(f"{label} cannot be empty.", flush=True)


def ensure_dependency_notice(args: argparse.Namespace) -> None:
    print_step("Dependency Reminder")
    print("Please run `uv run devenv.py` first to install the development dependencies.", flush=True)
    if DEVENV_SCRIPT.exists():
        print(f"Detected helper script: {DEVENV_SCRIPT}", flush=True)
    if is_interactive(args):
        if not confirm("Continue with the current environment?", default=True, args=args):
            raise RuntimeError("user aborted before build flow")


def ensure_boot_config(args: argparse.Namespace) -> None:
    print_step("Boot Config")
    if BOOT_CONFIG.exists():
        print(f"Using existing boot config: {BOOT_CONFIG}", flush=True)
        return

    print(f"Missing boot config: {BOOT_CONFIG}", flush=True)
    if not BOOT_CONFIG_SAMPLE.exists():
        raise RuntimeError(f"missing sample config: {BOOT_CONFIG_SAMPLE}")

    should_create = args.jarvis_agent or confirm(
        "Do you want to create a boot config for testing Jarvis Agent now?",
        default=False,
        args=args,
    )
    if not should_create:
        print("Skip boot config creation. Jarvis Agent testing will stay unavailable.", flush=True)
        return

    openai_api_key = prompt_text("OpenAI API Key", args=args, secret=True)
    bot_token = prompt_text("TG Bot Token", args=args, secret=True)
    bot_account_id = prompt_text("TG Bot AccountId", args=args)
    user_account_id = prompt_text("Your TG AccountId", args=args)
    user_name = prompt_text("Your TG UserName", args=args)
    show_name = prompt_text("Show name", args=args, default=user_name)

    template = BOOT_CONFIG_SAMPLE.read_text(encoding="utf-8")
    replacements = {
        "$YOUR_BOT_TOKEN$": json_string_fragment(bot_token),
        "$ YOUR_BOT_ACCOUNT_ID": json_string_fragment(bot_account_id),
        "$OPEN_AI_API_TOKEN$": json_string_fragment(openai_api_key),
        "$YOUR NAME": json_string_fragment(show_name),
        "$YOUR_TG_AccountId$": json_string_fragment(user_account_id),
        "$YOUR_TG_AccountName$": json_string_fragment(user_name),
    }
    for placeholder, value in replacements.items():
        template = template.replace(placeholder, value)

    BOOT_CONFIG.parent.mkdir(parents=True, exist_ok=True)
    BOOT_CONFIG.write_text(template, encoding="utf-8")
    print(f"Created boot config: {BOOT_CONFIG}", flush=True)


def docker_available() -> bool:
    return shutil.which("docker") is not None


def ensure_docker_images(args: argparse.Namespace) -> None:
    print_step("Docker")
    if args.skip_docker:
        print("Skipped docker checks by option.", flush=True)
        return

    if not docker_available():
        raise RuntimeError(
            "docker command not found. Install/start docker first. "
            "If you plan to debug Jarvis Agent, keep docker stopped."
        )

    docker_info = run_command(["docker", "info"], check=False, capture_output=True)
    if docker_info.returncode != 0:
        message = (
            "docker daemon is not running or not reachable.\n"
            "If you plan to debug Jarvis Agent, do not start docker.\n"
            "Otherwise start docker first."
        )
        if is_interactive(args) and confirm("Docker is unavailable. Continue anyway?", default=False, args=args):
            print(message, flush=True)
            return
        raise RuntimeError(message)

    for image in default_docker_images():
        inspect_result = run_command(["docker", "image", "inspect", image], check=False, capture_output=True)
        if inspect_result.returncode == 0 and not args.refresh_images:
            print(f"Image already present: {image}", flush=True)
            continue

        print(f"Pulling docker image: {image}", flush=True)
        pull_result = run_command(["docker", "pull", image], check=False)
        if pull_result.returncode == 0:
            continue

        if is_interactive(args) and confirm(
            f"Failed to pull {image}. Continue without it?",
            default=False,
            args=args,
        ):
            continue
        raise RuntimeError(f"failed to pull docker image: {image}")


def repo_specs() -> list[tuple[str, Path, str]]:
    return [
        (
            "cyfs-gateway",
            ROOT_DIR.parent / "cyfs-gateway",
            os.environ.get("BUCKYOS_CYFS_GATEWAY_REPO", "https://github.com/buckyos/cyfs-gateway.git"),
        ),
        (
            "buckyosapp",
            ROOT_DIR.parent / "buckyosapp",
            os.environ.get("BUCKYOS_APP_REPO", "https://github.com/buckyos/BuckyOSApp.git"),
        ),
    ]


def ensure_repos(args: argparse.Namespace) -> None:
    print_step("Required Repositories")
    if args.skip_clone:
        print("Skipped clone checks by option.", flush=True)
        return

    if shutil.which("git") is None:
        raise RuntimeError("git command not found")

    for name, path, url in repo_specs():
        if path.exists():
            print(f"Using existing repo: {path}", flush=True)
            continue

        print(f"Cloning {name} into {path}", flush=True)
        path.parent.mkdir(parents=True, exist_ok=True)
        run_command(["git", "clone", url, str(path)])


def build_current_repo() -> None:
    print_step("Build Current Repo")
    run_command(["uv", "run", "./buckyos-build.py"], cwd=SRC_DIR)
    run_command(["uv", "run", "./buckyos-install.py"], cwd=SRC_DIR)


def build_cyfs_gateway() -> None:
    print_step("Build cyfs-gateway")
    gateway_root = ROOT_DIR.parent / "cyfs-gateway"
    gateway_src = gateway_root / "src"
    if not gateway_src.exists():
        raise RuntimeError(f"missing cyfs-gateway source directory: {gateway_src}")

    run_command(["uv", "run", "buckyos-build.py"], cwd=gateway_src)
    run_command(["uv", "run", "buckyos-install.py"], cwd=gateway_src)


def start_buckyos() -> None:
    print_step("Start BuckyOS")
    run_command(["uv", "run", "start.py", "--all"], cwd=SRC_DIR)


def run_health_check(wait_seconds: int) -> int:
    print_step("Health Check")
    if wait_seconds > 0:
        print(f"Waiting {wait_seconds}s before check...", flush=True)
        time.sleep(wait_seconds)

    result = run_command(["uv", "run", str(CHECK_SCRIPT)], cwd=ROOT_DIR, check=False)
    return result.returncode


def print_success_tips() -> None:
    print_step("Next Steps")
    print("Open http://test.buckyos.io in a browser.", flush=True)
    print("Username: devtest", flush=True)
    print("Password: bucky2025", flush=True)
    print("If the browser runs on another host, forward port 80 from the dev machine.", flush=True)
    print("Say hello to Jarvis Agent in Telegram.", flush=True)
    print("Logs are under $BUCKYOS_ROOT/logs.", flush=True)


def main() -> int:
    args = parse_args()

    try:
        ensure_dependency_notice(args)
        ensure_boot_config(args)
        ensure_docker_images(args)
        ensure_repos(args)

        if not args.skip_root_build:
            build_current_repo()
        else:
            print_step("Build Current Repo")
            print("Skipped current repo build by option.", flush=True)

        if not args.skip_gateway_build:
            build_cyfs_gateway()
        else:
            print_step("Build cyfs-gateway")
            print("Skipped cyfs-gateway build by option.", flush=True)

        if not args.skip_start:
            start_buckyos()
        else:
            print_step("Start BuckyOS")
            print("Skipped start by option.", flush=True)

        if args.skip_check:
            print_step("Health Check")
            print("Skipped final runtime check by option.", flush=True)
            return 0

        check_code = run_health_check(args.wait_seconds)
        if check_code != 0:
            print("Runtime check failed. Please inspect the output above.", flush=True)
            return check_code

        print_success_tips()
        return 0
    except KeyboardInterrupt:
        print("\nInterrupted by user.", flush=True)
        return 130
    except Exception as error:
        print(f"\nBuild flow failed: {error}", flush=True)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
