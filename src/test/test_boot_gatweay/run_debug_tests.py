#!/usr/bin/env -S uv run
import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path


SCRIPT_DIR = Path(__file__).resolve().parent
PROJECT_ROOT = SCRIPT_DIR.parents[2]
DEFAULT_CONFIG = PROJECT_ROOT / "src" / "rootfs" / "etc" / "boot_gateway.yaml"
DEFAULT_BUCKYOS_ROOT = Path("/opt/buckyos")
REMOTE_APP_CASES = {
    "req_app_remote_ok",
}


def resolve_buckyos_root() -> Path:
    raw = os.environ.get("BUCKYOS_ROOT", "").strip()
    if raw:
        return Path(raw).expanduser()
    return DEFAULT_BUCKYOS_ROOT


def resolve_default_binary() -> Path | None:
    buckyos_root = resolve_buckyos_root()
    candidates = [
        buckyos_root / "bin" / "cyfs-gateway" / "cyfs_gateway",
        PROJECT_ROOT / "src" / "rootfs" / "bin" / "cyfs-gateway" / "cyfs_gateway",
        PROJECT_ROOT.parent / "cyfs-gateway" / "src" / "rootfs" / "bin" / "cyfs-gateway" / "cyfs_gateway",
        Path.home() / "cyfs-gateway" / "src" / "rootfs" / "bin" / "cyfs-gateway" / "cyfs_gateway",
    ]
    for candidate in candidates:
        if candidate.exists():
            return candidate
    return None


def build_selector(*targets: tuple[str, int, int]) -> dict:
    return {
        node_id: {
            "port": port,
            "weight": weight,
        }
        for node_id, port, weight in targets
    }


def build_typical_node_gateway_info(*, remote_app: bool) -> dict:
    filebrowser_node_id = "ood2" if remote_app else "ood1"
    control_panel_selector = build_selector(
        ("ood1", 10262, 10),
        ("ood2", 10263, 10),
    )
    return {
        "node_info": {
            "this_node_id": "ood1",
            "this_zone_host": "test.buckyos.io",
        },
        "app_info": {
            "publicview": {
                "app_id": "publicview",
                "sdk_version": 10,
                "access_mode": "private",
                "node_id": "ood1",
                "port": 10161,
            },
            "filebrowser": {
                "app_id": "filebrowser",
                "sdk_version": 10,
                "access_mode": "private",
                "node_id": filebrowser_node_id,
                "port": 10160,
                "block_services": ["kevent"],
            },
            "www": {
                "service_id": "control-panel",
                "selector": control_panel_selector,
            },
            "_": {
                "service_id": "control-panel",
                "selector": control_panel_selector,
            },
        },
        "service_info": {
            "control-panel": {
                "selector": control_panel_selector,
            },
            "system_config": {
                "selector": build_selector(("ood1", 3200, 10)),
            },
            "kmsg": {
                "selector": build_selector(
                    ("ood2", 10163, 10),
                    ("ood3", 10164, 10),
                ),
            },
            "kevent": {
                "selector": build_selector(("ood1", 10165, 10)),
            },
        },
        "node_route_map": {
            "ood2": "rtcp://ood2.test.buckyos.io/",
            "ood3": "rtcp://ood3.test.buckyos.io/",
        },
        "routes": {},
        "trust_key": {
            "issuer-main": "Wo0udCICmiQtnLwzpfulTbFEDvtT5UHNP-MZvnQ3dns",
            "issuer-backup": "s9j6X2zwk1DPjFt60z65LeBJN1DCTsqgeh15iF6Zmd4",
        },
    }


def stage_runtime(config: Path, node_gateway_info: dict) -> tuple[tempfile.TemporaryDirectory, Path]:
    temp_root = tempfile.TemporaryDirectory(prefix="boot-gateway-debug-")
    runtime_root = Path(temp_root.name)
    etc_dir = runtime_root / "etc"
    etc_dir.mkdir(parents=True, exist_ok=True)
    (runtime_root / "data" / "srv" / "publish").mkdir(parents=True, exist_ok=True)
    shutil.copy2(config, etc_dir / "boot_gateway.yaml")
    (etc_dir / "node_gateway_info.json").write_text(
        json.dumps(node_gateway_info, indent=2),
        encoding="utf-8",
    )
    return temp_root, etc_dir / "boot_gateway.yaml"


def parse_debug_output(output: str) -> dict:
    out = output.strip()
    if not out:
        raise RuntimeError("Empty cyfs_gateway debug output")

    try:
        return json.loads(out)
    except json.JSONDecodeError:
        pass

    decoder = json.JSONDecoder()
    candidate_indexes = [index for index, ch in enumerate(out) if ch == "{"]
    for index in reversed(candidate_indexes):
        try:
            parsed, end = decoder.raw_decode(out[index:])
        except json.JSONDecodeError:
            continue
        if out[index + end :].strip():
            continue
        if isinstance(parsed, dict):
            return parsed

    raise RuntimeError(f"No trailing JSON object in cyfs_gateway debug output: {output}")


def run_debug(binary: Path, config: Path, node_gateway_info: dict, req_file: Path) -> dict:
    temp_root, staged_config = stage_runtime(config, node_gateway_info)
    try:
        cmd = [
            str(binary),
            "debug",
            "--config_file",
            str(staged_config),
            "--req_file",
            str(req_file),
        ]
        result = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            cwd=PROJECT_ROOT,
        )
    finally:
        temp_root.cleanup()

    if result.returncode != 0:
        raise RuntimeError(
            f"cyfs_gateway debug failed (exit {result.returncode})\n"
            f"stderr: {result.stderr}\nstdout: {result.stdout}"
        )

    try:
        return parse_debug_output(result.stdout)
    except RuntimeError as error:
        raise RuntimeError(f"{error}\nOutput: {result.stdout}") from error


def build_node_gateway_info_for_case(case_name: str) -> dict:
    if case_name == "req_app_remote_via_routes_ok":
        info = build_typical_node_gateway_info(remote_app=True)
        info["routes"] = {
            "ood2": {
                "primary": {
                    "url": "tcp://ood2-edge.test.buckyos.io",
                    "backup": False,
                },
            },
        }
        return info
    if case_name == "req_service_kmsg_via_routes_ok":
        info = build_typical_node_gateway_info(remote_app=False)
        info["service_info"]["kmsg"] = {
            "selector": build_selector(("ood2", 10163, 10)),
        }
        info["routes"] = {
            "ood2": {
                "primary": {
                    "url": "tcp://ood2-edge.test.buckyos.io",
                    "backup": False,
                },
            },
        }
        return info
    return build_typical_node_gateway_info(remote_app=case_name in REMOTE_APP_CASES)


def control_matches(action_set, expected_substring=None, exact_value=None):
    def _check(result):
        ctrl = result.get("control_result", {})
        if ctrl.get("type") != "control":
            return False, f"expected control result, got {ctrl}"
        action = ctrl.get("action", "")
        value = str(ctrl.get("value", ""))
        if action not in action_set:
            return False, f"expected action in {sorted(action_set)}, got {ctrl}"
        if expected_substring is not None and expected_substring not in value:
            return False, f"expected value containing '{expected_substring}', got {ctrl}"
        if exact_value is not None and value != exact_value:
            return False, f"expected value '{exact_value}', got {ctrl}"
        return True, ""

    return _check


def assertions_for_case(case_name: str):
    if case_name == "req_app_local_ok":
        return [control_matches({"return", "exit"}, expected_substring="tcp:///127.0.0.1:10160")]
    if case_name == "req_app_remote_ok":
        return [control_matches({"return", "exit"}, expected_substring="rtcp://ood2.test.buckyos.io/:10160")]
    if case_name == "req_app_remote_via_routes_ok":
        return [control_matches({"return", "exit"}, expected_substring="tcp://ood2-edge.test.buckyos.io:10160")]
    if case_name == "req_service_kmsg_via_routes_ok":
        return [control_matches({"return", "exit"}, expected_substring="tcp://ood2-edge.test.buckyos.io:10163")]
    if case_name == "req_invalid_host_prefix_reject":
        return [control_matches({"exit"}, exact_value="reject")]
    if case_name == "req_invalid_host_dash_reject":
        return [control_matches({"exit"}, exact_value="reject")]
    if case_name == "req_service_by_host_prefix_ok":
        return [control_matches({"return", "exit"}, expected_substring="tcp:///127.0.0.1:10262")]
    if case_name == "req_service_by_root_host_ok":
        return [control_matches({"return", "exit"}, expected_substring="tcp:///127.0.0.1:10262")]
    if case_name == "req_service_by_kapi_ok":
        return [control_matches({"return", "exit"}, expected_substring="tcp:///127.0.0.1:10262")]
    if case_name == "req_sso_callback_ok":
        return [control_matches({"return", "exit"}, expected_substring="tcp:///127.0.0.1:10262")]
    if case_name == "req_ndm_ok":
        return [control_matches({"return", "exit"}, expected_substring="tcp:///127.0.0.1:10262")]
    if case_name == "req_service_system_config_identifiers_ok":
        return [control_matches({"return", "exit"}, expected_substring="tcp:///127.0.0.1:3200")]
    if case_name == "req_service_system_config_well_known_ok":
        return [control_matches({"return", "exit"}, expected_substring="tcp:///127.0.0.1:3200")]
    if case_name == "req_kevent_direct_ok":
        return [control_matches({"return", "exit"}, expected_substring="tcp:///127.0.0.1:3181")]
    return []


def test_case(binary: Path, config: Path, req_file: Path, case_name: str) -> bool:
    node_gateway_info = build_node_gateway_info_for_case(case_name)
    try:
        result = run_debug(binary, config, node_gateway_info, req_file)
    except Exception as error:
        print(f"  FAIL {case_name}: {error}")
        return False

    for assertion in assertions_for_case(case_name):
        passed, msg = assertion(result)
        if not passed:
            print(f"  FAIL {case_name}: {msg}")
            return False

    print(f"  PASS {case_name}")
    return True


def main():
    parser = argparse.ArgumentParser(description="Run boot_gateway debug tests")
    parser.add_argument(
        "--binary",
        type=Path,
        default=resolve_default_binary(),
        help="Path to cyfs_gateway binary",
    )
    parser.add_argument(
        "--config",
        type=Path,
        default=DEFAULT_CONFIG,
        help="Path to boot_gateway.yaml",
    )
    parser.add_argument(
        "--req-dir",
        type=Path,
        default=SCRIPT_DIR,
        help="Directory containing req_*.json files",
    )
    args = parser.parse_args()

    if args.binary is None:
        print("Error: cyfs_gateway binary not found")
        sys.exit(1)
    if not args.binary.exists():
        print(f"Error: binary not found: {args.binary}")
        sys.exit(1)
    if not args.config.exists():
        print(f"Error: config not found: {args.config}")
        sys.exit(1)

    req_files = sorted(args.req_dir.glob("req_*.json"))
    if not req_files:
        print(f"No req_*.json files in {args.req_dir}")
        sys.exit(1)

    print(f"Binary: {args.binary}")
    print(f"Config: {args.config}")
    print(f"Test cases: {len(req_files)}")
    print()

    passed = 0
    failed = 0

    for req_file in req_files:
        if test_case(args.binary, args.config, req_file, req_file.stem):
            passed += 1
        else:
            failed += 1

    print()
    print(f"Result: {passed} passed, {failed} failed")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
