#!/usr/bin/env python3

from __future__ import annotations

import json
import os
import socket
import subprocess
import sys
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
TEST_DIR = Path(__file__).resolve().parent
HARNESS_MANIFEST = TEST_DIR / "harness" / "Cargo.toml"
IMAGE = os.environ.get("KEVENT_PEER_CONTAINER_IMAGE", "ubuntu:24.04")
PREFIX = f"kevent-peer-{os.getpid()}"
NETWORK = f"{PREFIX}-net"
NODE_A = f"{PREFIX}-node-a"
NODE_B = f"{PREFIX}-node-b"


def run(command: list[str], **kwargs) -> subprocess.CompletedProcess:
    print("+", " ".join(command), flush=True)
    return subprocess.run(command, check=True, text=True, **kwargs)


def output(command: list[str], **kwargs) -> str:
    return run(command, stdout=subprocess.PIPE, **kwargs).stdout.strip()


def docker_available() -> None:
    run(["docker", "version"], stdout=subprocess.DEVNULL)


def build_harness() -> Path:
    target_dir = os.environ.get("CARGO_TARGET_DIR")
    if not target_dir:
        target_dir = str(ROOT / "src" / "target")
    env = os.environ.copy()
    env.setdefault("LIBCLANG_PATH", "/usr/lib/llvm-18/lib")
    env["CARGO_TARGET_DIR"] = target_dir

    run(
        [
            "cargo",
            "build",
            "--manifest-path",
            str(HARNESS_MANIFEST),
            "--bin",
            "kevent_peer_harness",
        ],
        cwd=ROOT,
        env=env,
    )
    binary = Path(target_dir) / "debug" / "kevent_peer_harness"
    if not binary.exists():
        raise RuntimeError(f"harness binary not found: {binary}")
    return binary


def remove_container(name: str) -> None:
    subprocess.run(["docker", "rm", "-f", name], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def remove_network(name: str) -> None:
    subprocess.run(["docker", "network", "rm", name], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def start_node(binary: Path, name: str, node_id: str, peer: str | None = None) -> None:
    command = [
        "docker",
        "run",
        "-d",
        "--name",
        name,
        "--network",
        NETWORK,
        "-v",
        f"{binary}:/usr/local/bin/kevent_peer_harness:ro",
        IMAGE,
        "/usr/local/bin/kevent_peer_harness",
        "server",
        "--node",
        node_id,
        "--listen",
        "0.0.0.0:3183",
    ]
    if peer is not None:
        command.extend(["--peer", peer])
    run(command)


def run_client(binary: Path) -> dict:
    completed = run(
        [
            "docker",
            "run",
            "--rm",
            "--network",
            NETWORK,
            "-v",
            f"{binary}:/usr/local/bin/kevent_peer_harness:ro",
            IMAGE,
            "/usr/local/bin/kevent_peer_harness",
            "client",
            "--node-a",
            f"{NODE_A}:3183",
            "--node-b",
            f"{NODE_B}:3183",
        ],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if completed.stderr:
        print(completed.stderr, file=sys.stderr, end="")
    print(completed.stdout, end="")
    lines = [line for line in completed.stdout.splitlines() if line.strip()]
    if not lines:
        raise RuntimeError("client produced no output")
    return json.loads(lines[-1])


def run_host_bridge_client(binary: Path) -> dict:
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        port = listener.getsockname()[1]

    server = subprocess.Popen(
        [
            str(binary),
            "server",
            "--node",
            "host_node",
            "--listen",
            f"0.0.0.0:{port}",
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
    )
    try:
        deadline = time.time() + 10
        while time.time() < deadline:
            if server.poll() is not None:
                stderr = server.stderr.read() if server.stderr else ""
                raise RuntimeError(f"host kevent server exited early: {stderr}")
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                    break
            except OSError:
                time.sleep(0.1)
        else:
            raise RuntimeError("host kevent server did not become ready")

        completed = run(
            [
                "docker",
                "run",
                "--rm",
                "--network",
                NETWORK,
                "--add-host",
                "host.docker.internal:host-gateway",
                "-v",
                f"{binary}:/usr/local/bin/kevent_peer_harness:ro",
                IMAGE,
                "/usr/local/bin/kevent_peer_harness",
                "sdk-client",
                "--daemon",
                f"host.docker.internal:{port}",
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        if completed.stderr:
            print(completed.stderr, file=sys.stderr, end="")
        print(completed.stdout, end="")
        lines = [line for line in completed.stdout.splitlines() if line.strip()]
        if not lines:
            raise RuntimeError("host bridge client produced no output")
        return json.loads(lines[-1])
    finally:
        server.terminate()
        try:
            server.wait(timeout=5)
        except subprocess.TimeoutExpired:
            server.kill()
            server.wait(timeout=5)


def main() -> int:
    docker_available()
    binary = build_harness()
    remove_container(NODE_A)
    remove_container(NODE_B)
    remove_network(NETWORK)

    try:
        run(["docker", "network", "create", NETWORK])
        start_node(binary, NODE_B, "node_b")
        start_node(binary, NODE_A, "node_a", f"{NODE_B}:3183")
        time.sleep(1)
        result = run_client(binary)
        if result.get("status") != "passed":
            raise RuntimeError(f"unexpected result: {result}")
        host_bridge_result = run_host_bridge_client(binary)
        if host_bridge_result.get("status") != "passed":
            raise RuntimeError(f"unexpected host bridge result: {host_bridge_result}")
        print(json.dumps({"status": "passed", "network": NETWORK, "image": IMAGE}, sort_keys=True))
        return 0
    finally:
        remove_container(NODE_A)
        remove_container(NODE_B)
        remove_network(NETWORK)


if __name__ == "__main__":
    raise SystemExit(main())
