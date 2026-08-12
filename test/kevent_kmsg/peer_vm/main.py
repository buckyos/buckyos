#!/usr/bin/env python3

from __future__ import annotations

import functools
import http.server
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import traceback
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
TEST_DIR = Path(__file__).resolve().parent
HARNESS_MANIFEST = ROOT / "test" / "kevent_kmsg" / "peer_container" / "harness" / "Cargo.toml"
CACHE_DIR = Path(os.environ.get("KEVENT_PEER_VM_CACHE", "/tmp/kevent_peer_vm_cache"))
IMAGE_URL = os.environ.get(
    "KEVENT_PEER_VM_IMAGE_URL",
    "https://cloud-images.ubuntu.com/releases/noble/release-20260518/ubuntu-24.04-server-cloudimg-amd64.img",
)
QEMU = (
    os.environ.get("QEMU_SYSTEM_X86_64")
    or shutil.which("qemu-system-x86_64")
    or "/snap/multipass/16300/usr/bin/qemu-system-x86_64"
)
QEMU_IMG = (
    os.environ.get("QEMU_IMG")
    or shutil.which("qemu-img")
    or "/snap/multipass/16300/usr/bin/qemu-img"
)
GENISOIMAGE = os.environ.get("GENISOIMAGE", "genisoimage")


def run(command: list[str], **kwargs) -> subprocess.CompletedProcess:
    print("+", " ".join(command), flush=True)
    return subprocess.run(command, check=True, text=True, **kwargs)


def build_harness() -> Path:
    target_dir = os.environ.get("CARGO_TARGET_DIR") or str(ROOT / "src" / "target")
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


def ensure_base_image() -> Path:
    CACHE_DIR.mkdir(parents=True, exist_ok=True)
    image = CACHE_DIR / "ubuntu-24.04-server-cloudimg-amd64.img"
    if image.exists() and image.stat().st_size > 100 * 1024 * 1024:
        return image
    tmp = image.with_suffix(".img.tmp")
    run(["curl", "-L", "--fail", "-o", str(tmp), IMAGE_URL])
    tmp.replace(image)
    return image


def create_seed_iso(work_dir: Path, node: str, peer: str | None, http_port: int) -> Path:
    seed_dir = work_dir / f"seed-{node}"
    seed_dir.mkdir()
    (seed_dir / "meta-data").write_text(
        f"instance-id: {node}\nlocal-hostname: {node}\n",
        encoding="utf-8",
    )
    peer_args = f" --peer {peer}" if peer else ""
    user_data = f"""#cloud-config
package_update: false
runcmd:
  - [ sh, -c, "until curl -fsS http://10.0.2.2:{http_port}/kevent_peer_harness -o /usr/local/bin/kevent_peer_harness; do sleep 1; done" ]
  - [ chmod, "+x", /usr/local/bin/kevent_peer_harness ]
  - [ sh, -c, "nohup /usr/local/bin/kevent_peer_harness server --node {node} --listen 0.0.0.0:3183{peer_args} > /var/log/kevent_peer_harness.log 2>&1 &" ]
"""
    (seed_dir / "user-data").write_text(user_data, encoding="utf-8")
    seed_iso = work_dir / f"{node}-seed.iso"
    run(
        [
            GENISOIMAGE,
            "-quiet",
            "-output",
            str(seed_iso),
            "-volid",
            "cidata",
            "-joliet",
            "-rock",
            str(seed_dir / "user-data"),
            str(seed_dir / "meta-data"),
        ]
    )
    return seed_iso


def start_http_server(binary: Path) -> tuple[http.server.ThreadingHTTPServer, int, threading.Thread]:
    handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=str(binary.parent))
    server = http.server.ThreadingHTTPServer(("0.0.0.0", 0), handler)
    port = server.server_address[1]
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    return server, port, thread


def create_overlay(base_image: Path, work_dir: Path, name: str) -> Path:
    overlay = work_dir / f"{name}.qcow2"
    run(
        [
            QEMU_IMG,
            "create",
            "-f",
            "qcow2",
            "-F",
            "qcow2",
            "-b",
            str(base_image),
            str(overlay),
        ]
    )
    return overlay


def start_vm(
    name: str,
    disk: Path,
    seed_iso: Path,
    kevent_host_port: int,
    ssh_host_port: int,
    log_file: Path,
) -> subprocess.Popen:
    command = [
        QEMU,
        "-enable-kvm",
        "-m",
        "1024",
        "-smp",
        "1",
        "-display",
        "none",
        "-serial",
        f"file:{log_file}",
        "-drive",
        f"file={disk},format=qcow2,if=virtio",
        "-drive",
        f"file={seed_iso},format=raw,media=cdrom,readonly=on",
        "-netdev",
        f"user,id=net0,hostfwd=tcp::{kevent_host_port}-:3183,hostfwd=tcp::{ssh_host_port}-:22",
        "-device",
        "virtio-net-pci,netdev=net0",
        "-name",
        name,
    ]
    print("+", " ".join(command), flush=True)
    return subprocess.Popen(command, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def wait_for_port(port: int, timeout_seconds: int = 420) -> None:
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
            sock.settimeout(1)
            if sock.connect_ex(("127.0.0.1", port)) == 0:
                return
        time.sleep(1)
    raise TimeoutError(f"port {port} did not open within {timeout_seconds}s")


def main() -> int:
    binary = build_harness()
    base_image = ensure_base_image()
    server, http_port, _ = start_http_server(binary)
    procs: list[subprocess.Popen] = []
    work_dir = Path(tempfile.mkdtemp(prefix="kevent-peer-vm-"))
    keep_work_dir = False
    try:
        try:
            node_b_disk = create_overlay(base_image, work_dir, "node-b")
            node_a_disk = create_overlay(base_image, work_dir, "node-a")
            node_b_seed = create_seed_iso(work_dir, "node_b", None, http_port)
            node_a_seed = create_seed_iso(work_dir, "node_a", "10.0.2.2:23183", http_port)
            procs.append(
                start_vm(
                    "kevent-peer-node-b",
                    node_b_disk,
                    node_b_seed,
                    23183,
                    22222,
                    work_dir / "node-b.log",
                )
            )
            procs.append(
                start_vm(
                    "kevent-peer-node-a",
                    node_a_disk,
                    node_a_seed,
                    13183,
                    12222,
                    work_dir / "node-a.log",
                )
            )
            wait_for_port(23183)
            wait_for_port(13183)
            completed = run(
                [
                    str(binary),
                    "client",
                    "--node-a",
                    "127.0.0.1:13183",
                    "--node-b",
                    "127.0.0.1:23183",
                ],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            if completed.stderr:
                print(completed.stderr, file=sys.stderr, end="")
            print(completed.stdout, end="")
            lines = [line for line in completed.stdout.splitlines() if line.strip()]
            result = json.loads(lines[-1])
            if result.get("status") != "passed":
                raise RuntimeError(f"unexpected result: {result}")
            print(
                json.dumps(
                    {
                        "status": "passed",
                        "backend": "qemu-kvm",
                        "node_a_port": 13183,
                        "node_b_port": 23183,
                    },
                    sort_keys=True,
                )
            )
            return 0
        except Exception:
            keep_work_dir = True
            print(f"VM test failed; preserving work directory: {work_dir}", file=sys.stderr)
            for log_name in ("node-a.log", "node-b.log"):
                log_path = work_dir / log_name
                if log_path.exists():
                    print(f"--- {log_name} tail ---", file=sys.stderr)
                    try:
                        print("\n".join(log_path.read_text(errors="replace").splitlines()[-120:]), file=sys.stderr)
                    except Exception as log_error:
                        print(f"failed to read {log_path}: {log_error}", file=sys.stderr)
            traceback.print_exc()
            return 1
        finally:
            server.shutdown()
            for proc in procs:
                proc.terminate()
            for proc in procs:
                try:
                    proc.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait(timeout=10)
    finally:
        if not keep_work_dir:
            shutil.rmtree(work_dir, ignore_errors=True)


if __name__ == "__main__":
    raise SystemExit(main())
