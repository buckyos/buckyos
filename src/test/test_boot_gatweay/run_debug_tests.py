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
AUTH_REJECTION_TOKENS = {
    "missing_target_kind": "eyJhbGciOiJFZERTQSIsImtpZCI6InZlcmlmeS1odWIifQ.eyJpc3MiOiJ2ZXJpZnktaHViIiwiYXBwaWQiOiJmaWxlYnJvd3NlciIsInN1YiI6ImRlYnVnLXVzZXIiLCJleHAiOjIwNTg4Mzg5MzksInByaW5jaXBhbF9raW5kIjoidXNlciIsInRva2VuX3VzZSI6InNlc3Npb24iLCJhcHBfaW5zdGFuY2VfaWQiOiJmaWxlYnJvd3NlckBhbGljZSIsImFwcF9vd25lcl91c2VyX2lkIjoiYWxpY2UifQ.j5UQoweW-PEkj_rejBXsZ8qBBErooWTk97ANgJm4s4z1P3s8zG8LfRoya7rijxKkQOJeGNSC42G7v0ycLEvpBQ",
    "system_target": "eyJhbGciOiJFZERTQSIsImtpZCI6InZlcmlmeS1odWIifQ.eyJpc3MiOiJ2ZXJpZnktaHViIiwiYXBwaWQiOiJmaWxlYnJvd3NlciIsInN1YiI6ImRlYnVnLXVzZXIiLCJleHAiOjIwNTg4Mzg5MzksInByaW5jaXBhbF9raW5kIjoidXNlciIsInRva2VuX3VzZSI6InNlc3Npb24iLCJ0YXJnZXRfa2luZCI6InN5c3RlbSJ9.mTbmmVGtoXc3POhI7hlXAOosMvqCJhsIrxxG7KKs8UZNPPX6UZaAejbuZqvAVQJxSXWZmFs6T27F4j2yiTKqDw",
    "wrong_owner": "eyJhbGciOiJFZERTQSIsImtpZCI6InZlcmlmeS1odWIifQ.eyJpc3MiOiJ2ZXJpZnktaHViIiwiYXBwaWQiOiJmaWxlYnJvd3NlciIsInN1YiI6ImRlYnVnLXVzZXIiLCJleHAiOjIwNTg4Mzg5MzksInByaW5jaXBhbF9raW5kIjoidXNlciIsInRva2VuX3VzZSI6InNlc3Npb24iLCJ0YXJnZXRfa2luZCI6ImFwcCIsImFwcF9pbnN0YW5jZV9pZCI6ImZpbGVicm93c2VyQGJvYiIsImFwcF9vd25lcl91c2VyX2lkIjoiYm9iIn0.0npUXmr3E46hxl9ubHUvAdLAazDkkVh9i8S7sXaEu7dSxAF7XkWsM4c9lMzhM6JOvbGYxRH_FiTguQN0x_y1Bg",
    "refresh": "eyJhbGciOiJFZERTQSIsImtpZCI6InZlcmlmeS1odWIifQ.eyJpc3MiOiJ2ZXJpZnktaHViIiwiYXBwaWQiOiJmaWxlYnJvd3NlciIsInN1YiI6ImRlYnVnLXVzZXIiLCJleHAiOjIwNTg4Mzg5MzksInByaW5jaXBhbF9raW5kIjoidXNlciIsInRva2VuX3VzZSI6InJlZnJlc2giLCJ0YXJnZXRfa2luZCI6ImFwcCIsImFwcF9pbnN0YW5jZV9pZCI6ImZpbGVicm93c2VyQGFsaWNlIiwiYXBwX293bmVyX3VzZXJfaWQiOiJhbGljZSJ9.89Gw518ApNzWH16rkDCqi0axJAIDBLuoqIUN86ToAhntWN5E-LLzQuy3t37LUWXyJIhZ9jJnzLJD3NCoO-8pCA",
    "sudo": "eyJhbGciOiJFZERTQSIsImtpZCI6InZlcmlmeS1odWIifQ.eyJpc3MiOiJ2ZXJpZnktaHViIiwiYXBwaWQiOiJmaWxlYnJvd3NlciIsInN1YiI6ImRlYnVnLXVzZXIiLCJleHAiOjIwNTg4Mzg5MzksInByaW5jaXBhbF9raW5kIjoidXNlciIsInRva2VuX3VzZSI6InNlc3Npb24iLCJ0YXJnZXRfa2luZCI6ImFwcCIsImFwcF9pbnN0YW5jZV9pZCI6ImZpbGVicm93c2VyQGFsaWNlIiwiYXBwX293bmVyX3VzZXJfaWQiOiJhbGljZSIsInN1ZG8iOnRydWV9.jfdptnTl-7boo8oHqnZRgID6TXPXrcvyMJaSbGqa1B-ivIJqKexRS4QF36lf3_I6uztprnX0UbzwC5eIvUJDCg",
    "non_verify_hub": "eyJhbGciOiJFZERTQSIsImtpZCI6InZlcmlmeS1odWIifQ.eyJpc3MiOiJvb2QxIiwiYXBwaWQiOiJmaWxlYnJvd3NlciIsInN1YiI6ImRlYnVnLXVzZXIiLCJleHAiOjIwNTg4Mzg5MzksInByaW5jaXBhbF9raW5kIjoidXNlciIsInRva2VuX3VzZSI6InNlc3Npb24iLCJ0YXJnZXRfa2luZCI6ImFwcCIsImFwcF9pbnN0YW5jZV9pZCI6ImZpbGVicm93c2VyQGFsaWNlIiwiYXBwX293bmVyX3VzZXJfaWQiOiJhbGljZSJ9.JvCE0v6ZyWC7wCO8uMNwbrU4sw4EyzreC485kNbW-JK18DQBZY4s9sJfV59Fa0TKi6kxw5lSygR-vjdCfVC2DA",
    "missing_owner": "eyJhbGciOiJFZERTQSIsImtpZCI6InZlcmlmeS1odWIifQ.eyJpc3MiOiJ2ZXJpZnktaHViIiwiYXBwaWQiOiJmaWxlYnJvd3NlciIsInN1YiI6ImRlYnVnLXVzZXIiLCJleHAiOjIwNTg4Mzg5MzksInByaW5jaXBhbF9raW5kIjoidXNlciIsInRva2VuX3VzZSI6InNlc3Npb24iLCJ0YXJnZXRfa2luZCI6ImFwcCIsImFwcF9pbnN0YW5jZV9pZCI6ImZpbGVicm93c2VyQGFsaWNlIn0.TNLxnk9X4A9LqXek_uVhBJmi6PR92P-KH88PKwL8M-RLtPlgcvBAS55NcrNi2XpMXoCgba7Vnvha8UFM0kcPBg",
}
ROUTE_REJECTION_CASES = ("mixed_fields", "missing_owner", "mismatched_instance")
SSO_SERVICE_PATHS = ("/sso_refresh", "/sso_logout")


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
                "app_instance_id": "publicview@alice",
                "app_owner_user_id": "alice",
                "sdk_version": 10,
                "access_mode": "public",
                "node_id": "ood1",
                "port": 10161,
            },
            "filebrowser": {
                "app_id": "filebrowser",
                "app_instance_id": "filebrowser@alice",
                "app_owner_user_id": "alice",
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
            "verify-hub": "gubVIszw-u_d5PVTh-oc8CKAhM9C-ne5G_yUK5BDaXc",
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
    params_src = config.parent / "node_gateway_params.json"
    if params_src.exists():
        shutil.copy2(params_src, etc_dir / "node_gateway_params.json")
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


def run_auth_rejection_case(binary: Path, config: Path, name: str, token: str) -> bool:
    request = {
        "input": {
            "REQ": {
                "path": "/index.html",
                "host": "filebrowser.test.buckyos.io",
                "uri": "/index.html",
                "url": "https://filebrowser.test.buckyos.io/index.html",
                "Cookie": f"buckyos_session_token={token}",
            }
        },
        "id": "server:node_gateway:main",
        "output": ["REQ", "RESP"],
    }
    with tempfile.TemporaryDirectory(prefix="boot-gateway-auth-req-") as request_dir:
        req_file = Path(request_dir) / f"req_auth_{name}.json"
        req_file.write_text(json.dumps(request), encoding="utf-8")
        result = run_debug(binary, config, build_typical_node_gateway_info(remote_app=False), req_file)
    passed, message = control_matches({"return"}, expected_substring="/login?redirect_url=")(
        result
    )
    if not passed:
        print(f"  FAIL req_auth_{name}_reject: {message}")
        return False
    print(f"  PASS req_auth_{name}_reject")
    return True


def run_route_rejection_case(binary: Path, config: Path, name: str) -> bool:
    info = build_typical_node_gateway_info(remote_app=False)
    entry = info["app_info"]["filebrowser"]
    if name == "mixed_fields":
        entry["service_id"] = "control-panel"
    elif name == "missing_owner":
        del entry["app_owner_user_id"]
    elif name == "mismatched_instance":
        entry["app_instance_id"] = "filebrowser@bob"
    request = {
        "input": {"REQ": {"path": "/", "host": "filebrowser.test.buckyos.io", "uri": "/"}},
        "id": "server:node_gateway:main",
        "output": ["REQ", "RESP"],
    }
    with tempfile.TemporaryDirectory(prefix="boot-gateway-route-req-") as request_dir:
        req_file = Path(request_dir) / f"req_route_{name}.json"
        req_file.write_text(json.dumps(request), encoding="utf-8")
        result = run_debug(binary, config, info, req_file)
    passed, message = control_matches({"exit"}, exact_value="reject")(result)
    if not passed:
        print(f"  FAIL req_route_{name}_reject: {message}")
        return False
    print(f"  PASS req_route_{name}_reject")
    return True


def run_sso_service_route_case(binary: Path, config: Path, path: str) -> bool:
    request = {
        "input": {"REQ": {"path": path, "host": "filebrowser.test.buckyos.io", "uri": path}},
        "id": "server:node_gateway:main",
        "output": ["REQ", "RESP"],
    }
    with tempfile.TemporaryDirectory(prefix="boot-gateway-sso-req-") as request_dir:
        req_file = Path(request_dir) / f"req_{path.removeprefix('/')}.json"
        req_file.write_text(json.dumps(request), encoding="utf-8")
        result = run_debug(
            binary, config, build_typical_node_gateway_info(remote_app=False), req_file
        )
    passed, message = control_matches(
        {"return", "exit"}, expected_substring="tcp:///127.0.0.1:10262"
    )(result)
    if not passed:
        print(f"  FAIL req_{path.removeprefix('/')}_no_cookie: {message}")
        return False
    print(f"  PASS req_{path.removeprefix('/')}_no_cookie")
    return True


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


def request_header_equals(name, expected_value):
    def _check(result):
        request = result.get("output", {}).get("REQ", {})
        actual = request.get(name)
        if actual != expected_value:
            return False, f"expected request header {name}={expected_value!r}, got {actual!r}"
        return True, ""

    return _check


def assertions_for_case(case_name: str):
    if case_name == "req_app_public_no_cookie_ok":
        return [control_matches({"return", "exit"}, expected_substring="tcp:///127.0.0.1:10161")]
    if case_name == "req_app_root_no_cookie_redirect_ok":
        return [
            control_matches(
                {"return"},
                exact_value='redirect "https://sys.test.buckyos.io/login?redirect_url=https%3A%2F%2Ffilebrowser.test.buckyos.io%2F"',
            )
        ]
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
        return [
            control_matches({"return", "exit"}, expected_substring="tcp:///127.0.0.1:10262"),
            request_header_equals("X-Forwarded-Proto", "https"),
        ]
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
    extra_cases = len(AUTH_REJECTION_TOKENS) + len(ROUTE_REJECTION_CASES) + len(SSO_SERVICE_PATHS)
    print(f"Test cases: {len(req_files) + extra_cases}")
    print()

    passed = 0
    failed = 0

    for req_file in req_files:
        if test_case(args.binary, args.config, req_file, req_file.stem):
            passed += 1
        else:
            failed += 1

    for name, token in AUTH_REJECTION_TOKENS.items():
        if run_auth_rejection_case(args.binary, args.config, name, token):
            passed += 1
        else:
            failed += 1

    for name in ROUTE_REJECTION_CASES:
        if run_route_rejection_case(args.binary, args.config, name):
            passed += 1
        else:
            failed += 1

    for path in SSO_SERVICE_PATHS:
        if run_sso_service_route_case(args.binary, args.config, path):
            passed += 1
        else:
            failed += 1

    print()
    print(f"Result: {passed} passed, {failed} failed")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
