import subprocess
from collections.abc import Callable
from pathlib import Path


def collect_gateway(
    repo: Path,
    collect_cargo: Callable,
    offline: bool = False,
) -> tuple[list[dict], dict, list[str]]:
    gateway = repo.resolve().parent / "cyfs-gateway"
    scope = {"id": "gateway", "label": "CYFS Gateway", "status": "unavailable", "isSnapshot": True}
    notice = "CYFS Gateway is built separately. Its entries are a snapshot of the sibling checkout's locked dependencies, not a verification of the gateway installed with this BuckyOS build."
    if not (gateway / "src/Cargo.toml").is_file():
        return [], scope, [notice, "CYFS Gateway source checkout is unavailable; its dependencies are not included."]
    if not (gateway / "src/Cargo.lock").is_file():
        return [], scope, [notice, "CYFS Gateway Cargo.lock is unavailable; exact dependency versions could not be verified."]
    try:
        packages, cargo_scope, notes = collect_cargo(gateway, None, offline)
    except (OSError, subprocess.SubprocessError, ValueError, KeyError, TypeError):
        return [], scope, [notice, "CYFS Gateway locked Cargo metadata could not be collected; its dependencies are not included."]
    scope["status"] = cargo_scope["status"]
    packages = [{**package, "scopes": ["gateway"]} for package in packages]
    return packages, scope, [notice, *notes]
