#!/usr/bin/env -S uv run

import argparse
import os
import platform
import shutil
import subprocess
import sys
from pathlib import Path


SCRIPT_DIR = Path(__file__).resolve().parent
PROJECT_ROOT = SCRIPT_DIR.parent
DEFAULT_VM_ROOTFS = Path("/opt/buckyosvm")
DEFAULT_WEB3_GATEWAY_ROOT = Path("/opt/web3-gateway")
DEVKIT_SPEC = "buckyos-devkit @ git+https://github.com/buckyos/buckyos-devkit.git"


def _command_names(command: str) -> list[str]:
    if os.name == "nt":
        return [f"{command}.exe", f"{command}.cmd", f"{command}.bat", command]
    return [command]


def _find_command(command: str) -> str:
    for name in _command_names(command):
        path = shutil.which(name)
        if path is not None:
            return path

    bin_dir = Path(sys.executable).parent
    for name in _command_names(command):
        candidate = bin_dir / name
        if candidate.exists():
            return str(candidate)

    raise FileNotFoundError(
        f"{command} not found in the current uv runtime. "
        f"Run this script with `uv run src/build_for_vm_test.py` or install `{DEVKIT_SPEC}`."
    )


def _linux_target_for_host() -> str:
    machine = platform.machine().lower()
    if machine in {"arm64", "aarch64"}:
        return "aarch64"
    if machine in {"x86_64", "amd64"}:
        return "amd64"
    raise RuntimeError(f"Unsupported host CPU architecture for VM test: {machine}")


def _run(cmd: list[str], cwd: Path, env: dict[str, str] | None = None) -> None:
    print(f"* ({cwd}) {' '.join(cmd)}", flush=True)
    result = subprocess.run(cmd, cwd=cwd, env=env or os.environ.copy(), check=False)
    if result.returncode != 0:
        raise RuntimeError(f"command failed with return code {result.returncode}: {' '.join(cmd)}")


def _find_cyfs_gateway_src(explicit_path: Path | None) -> Path:
    if explicit_path is not None:
        path = explicit_path.expanduser().resolve()
    else:
        path = (PROJECT_ROOT.parent / "cyfs-gateway" / "src").resolve()

    if not (path / "bucky_project.yaml").exists():
        raise FileNotFoundError(f"cyfs-gateway src directory not found: {path}")

    return path


def _build_project(project_src: Path, target: str, rootfs: Path) -> None:
    buckyos_build = _find_command("buckyos-build")
    env = os.environ.copy()
    env["BUCKYOS_ROOT"] = str(rootfs)
    _run([buckyos_build, target], cwd=project_src, env=env)


def _install_app(project_src: Path, app_name: str, rootfs: Path) -> None:
    buckyos_install = _find_command("buckyos-install")
    env = os.environ.copy()
    env["BUCKYOS_ROOT"] = str(rootfs)
    _run(
        [
            buckyos_install,
            "--all",
            f"--app={app_name}",
            f"--target-rootfs={rootfs}",
        ],
        cwd=project_src,
        env=env,
    )


def _make_config(node_group: str, rootfs: Path) -> None:
    env = os.environ.copy()
    env["BUCKYOS_ROOT"] = str(rootfs)
    _run(
        [
            "deno",
            "task",
            "make_config",
            node_group,
            "--rootfs",
            str(rootfs),
        ],
        cwd=SCRIPT_DIR,
        env=env,
    )


def _prepare_node_rootfs(base_rootfs: Path, current_rootfs: Path, node_group: str) -> None:
    if not base_rootfs.exists():
        raise FileNotFoundError(
            f"base VM test rootfs is missing: {base_rootfs}. "
            "Run `uv run ./build_for_vm_test.py` once before devtest install/update."
        )

    tmp_rootfs = current_rootfs.with_name(f".{current_rootfs.name}.tmp")
    shutil.rmtree(tmp_rootfs, ignore_errors=True)
    shutil.copytree(base_rootfs, tmp_rootfs, symlinks=True)
    _make_config(node_group, tmp_rootfs)

    shutil.rmtree(current_rootfs, ignore_errors=True)
    tmp_rootfs.rename(current_rootfs)
    print(f"* Prepared node rootfs for {node_group}: {current_rootfs}", flush=True)


def _parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Build and stage Linux BuckyOS rootfs for VM tests.",
        formatter_class=argparse.ArgumentDefaultsHelpFormatter,
    )
    parser.add_argument(
        "--rootfs",
        type=Path,
        default=Path(os.environ.get("BUCKYOS_VMTEST_ROOT", DEFAULT_VM_ROOTFS)),
        help="VM test BUCKYOS_ROOT staging directory",
    )
    parser.add_argument(
        "--target",
        default=None,
        choices=["amd64", "aarch64"],
        help="Linux target CPU architecture. Defaults to the current host CPU.",
    )
    parser.add_argument(
        "--cyfs-gateway-src",
        type=Path,
        default=None,
        help="Path to sibling cyfs-gateway/src",
    )
    parser.add_argument(
        "--web3-gateway-root",
        type=Path,
        default=Path(os.environ.get("BUCKYOS_WEB3_GATEWAY_ROOT", DEFAULT_WEB3_GATEWAY_ROOT)),
        help="VM test web3-gateway staging directory used by devtest as source",
    )
    parser.add_argument(
        "--node-group",
        default=None,
        help="Optional make_config.ts group, e.g. alice.ood1",
    )
    parser.add_argument(
        "--skip-build",
        action="store_true",
        help="Skip buckyos-build for both cyfs-gateway and BuckyOS.",
    )
    parser.add_argument(
        "--skip-install",
        action="store_true",
        help="Skip installing BuckyOS, cyfs-gateway, and web3-gateway into staging roots.",
    )
    return parser.parse_args()


def main() -> int:
    args = _parse_args()
    target = args.target or _linux_target_for_host()
    rootfs = args.rootfs.expanduser().resolve()
    base_rootfs = rootfs / "base"
    current_rootfs = rootfs / "current"
    cyfs_gateway_src = _find_cyfs_gateway_src(args.cyfs_gateway_src)
    web3_gateway_root = args.web3_gateway_root.expanduser().resolve()

    print(f"* VM test target: linux/{target}", flush=True)
    print(f"* VM test rootfs: {rootfs}", flush=True)
    print(f"* VM test base rootfs: {base_rootfs}", flush=True)
    print(f"* VM test current rootfs: {current_rootfs}", flush=True)
    print(f"* cyfs-gateway src: {cyfs_gateway_src}", flush=True)
    print(f"* web3-gateway root: {web3_gateway_root}", flush=True)

    try:
        if not args.skip_build or not args.skip_install:
            base_rootfs.mkdir(parents=True, exist_ok=True)

        if not args.skip_build:
            _build_project(cyfs_gateway_src, target, base_rootfs)
            _build_project(SCRIPT_DIR, target, base_rootfs)

        if not args.skip_install:
            _install_app(SCRIPT_DIR, "buckyos", base_rootfs)
            _install_app(cyfs_gateway_src, "cyfs-gateway", base_rootfs)
            web3_gateway_root.mkdir(parents=True, exist_ok=True)
            _install_app(cyfs_gateway_src, "web3-gateway", web3_gateway_root)

        if args.node_group:
            _prepare_node_rootfs(base_rootfs, current_rootfs, args.node_group)
    except Exception as exc:
        print(f"build_for_vm_test failed: {exc}", file=sys.stderr)
        return 1

    print("build_for_vm_test completed successfully", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
