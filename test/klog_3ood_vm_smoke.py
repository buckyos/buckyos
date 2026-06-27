#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any
from urllib.parse import urlencode


REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_NODE_SPEC = "klogood1:ood1,klogood2:ood2,klogood3:ood3"
REQUIRED_DIAG_FIELDS = (
    "raft_metrics",
    "current_term",
    "vote",
    "last_log_index",
    "last_applied",
    "millis_since_quorum_ack",
)

VM_HTTP = r"""
import json
import sys
import urllib.error
import urllib.request

method, url, body, timeout = sys.argv[1], sys.argv[2], sys.argv[3], float(sys.argv[4])
data = body.encode("utf-8") if body else None
req = urllib.request.Request(
    url,
    data=data,
    headers={"Content-Type": "application/json"},
    method=method,
)
try:
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        out = {
            "status": resp.status,
            "body": resp.read().decode("utf-8"),
        }
except urllib.error.HTTPError as err:
    out = {
        "status": err.code,
        "body": err.read().decode("utf-8", errors="replace"),
    }
except Exception as err:
    out = {
        "status": 0,
        "body": "",
        "error": f"{type(err).__name__}: {err}",
    }
print(json.dumps(out))
"""


@dataclass(frozen=True)
class Node:
    vm: str
    node_name: str


def parse_nodes(raw: str) -> list[Node]:
    nodes: list[Node] = []
    for token in raw.split(","):
        token = token.strip()
        if not token:
            continue
        if ":" not in token:
            raise ValueError(f"invalid node spec {token!r}, expected vm:node_name")
        vm, node_name = token.split(":", 1)
        nodes.append(Node(vm=vm.strip(), node_name=node_name.strip()))
    if len(nodes) < 3:
        raise ValueError(f"expected at least 3 nodes, got {len(nodes)} from {raw!r}")
    return nodes


class VmSmoke:
    def __init__(self, nodes: list[Node], report_dir: Path, http_timeout: float) -> None:
        self.nodes = nodes
        self.report_dir = report_dir
        self.http_timeout = http_timeout
        self.cluster_dir = report_dir / "cluster_state"
        self.rpc_dir = report_dir / "rpc"
        self.log_dir = report_dir / "logs"
        self.cluster_dir.mkdir(parents=True, exist_ok=True)
        self.rpc_dir.mkdir(parents=True, exist_ok=True)
        self.log_dir.mkdir(parents=True, exist_ok=True)

    def run(self, cmd: list[str], timeout: float = 30.0) -> str:
        completed = subprocess.run(
            cmd,
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
            timeout=timeout,
        )
        if completed.returncode != 0:
            raise RuntimeError(
                "command failed: {}\nstdout:\n{}\nstderr:\n{}".format(
                    " ".join(cmd),
                    completed.stdout.strip(),
                    completed.stderr.strip(),
                )
            )
        return completed.stdout

    def vm_exec(self, node: Node, args: list[str], timeout: float = 30.0) -> str:
        return self.run(["multipass", "exec", node.vm, "--", *args], timeout=timeout)

    def vm_http(
        self,
        node: Node,
        method: str,
        url: str,
        body: dict[str, Any] | None = None,
    ) -> Any:
        payload = json.dumps(body, separators=(",", ":")) if body is not None else ""
        out = self.vm_exec(
            node,
            ["python3", "-c", VM_HTTP, method, url, payload, str(self.http_timeout)],
            timeout=self.http_timeout + 10,
        )
        envelope = json.loads(out)
        if envelope.get("status") == 0:
            raise RuntimeError(f"{node.vm} {method} {url} failed: {envelope.get('error')}")
        status = int(envelope["status"])
        text = envelope.get("body") or ""
        if status < 200 or status >= 300:
            raise RuntimeError(f"{node.vm} {method} {url} status={status} body={text}")
        if not text:
            return None
        return json.loads(text)

    def save_json(self, subdir: Path, name: str, value: Any) -> None:
        path = subdir / f"{safe_name(name)}.json"
        path.write_text(json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True), encoding="utf-8")

    def save_text(self, subdir: Path, name: str, value: str) -> None:
        path = subdir / f"{safe_name(name)}.log"
        path.write_text(value, encoding="utf-8")

    def cluster_state(self, node: Node) -> dict[str, Any]:
        value = self.vm_http(
            node,
            "GET",
            "http://127.0.0.1:21003/klog/admin/cluster-state",
        )
        if not isinstance(value, dict):
            raise RuntimeError(f"{node.vm} cluster-state is not an object: {value!r}")
        validate_cluster_state(value, node.vm)
        return value

    def gateway_cluster_state(self, source: Node, target: Node) -> dict[str, Any]:
        value = self.vm_http(
            source,
            "GET",
            f"http://127.0.0.1:3180/.cluster/klog/{target.node_name}/admin/cluster-state",
        )
        if not isinstance(value, dict):
            raise RuntimeError(f"{source.vm}->{target.node_name} gateway cluster-state is not an object")
        validate_cluster_state(value, f"{source.vm}->{target.node_name}")
        return value

    def gateway_inter_query(
        self,
        source_node: Node,
        target_node: Node,
        log_id: int,
        log_source: str,
    ) -> dict[str, Any]:
        query = urlencode(
            {
                "start_id": log_id,
                "end_id": log_id,
                "limit": 4,
                "desc": "false",
                "level": "INFO",
                "source": log_source,
                "strong_read": "true",
            }
        )
        value = self.vm_http(
            source_node,
            "GET",
            f"http://127.0.0.1:3180/.cluster/klog/{target_node.node_name}/inter/query?{query}",
        )
        if not isinstance(value, dict):
            raise RuntimeError(
                f"{source_node.vm}->{target_node.node_name} inter query returned non-object: {value!r}"
            )
        return value

    def sample_cluster(self, stage: str, save: bool = True) -> dict[str, dict[str, Any]]:
        states: dict[str, dict[str, Any]] = {}
        for node in self.nodes:
            states[node.vm] = self.cluster_state(node)
        if save:
            for vm, state in states.items():
                self.save_json(self.cluster_dir, f"{stage}_{vm}", state)
        print_cluster_summary(stage, states)
        return states

    def collect_klog_logs(self, stage: str, lines: int) -> None:
        if lines <= 0:
            return
        for node in self.nodes:
            try:
                output = self.vm_exec(
                    node,
                    [
                        "sh",
                        "-lc",
                        f"sudo tail -n {int(lines)} /opt/buckyos/logs/klog-service/native-detached.log 2>/dev/null || true",
                    ],
                    timeout=10,
                )
            except Exception as err:
                output = f"<failed to collect klog log: {err}>"
            self.save_text(self.log_dir, f"{stage}_{node.vm}_klog", output)

    def validate_gateway_routes(self, stage: str) -> None:
        source = self.nodes[0]
        for target in self.nodes:
            state = self.gateway_cluster_state(source, target)
            self.save_json(self.cluster_dir, f"{stage}_gateway_{source.vm}_to_{target.node_name}", state)
        print(f"[ok] gateway admin routes from {source.vm} reached all klog nodes")

    def wait_cluster_ready(self, stage: str, timeout: float) -> dict[str, dict[str, Any]]:
        deadline = time.monotonic() + timeout
        last_error = ""
        attempt = 0
        while time.monotonic() < deadline:
            attempt += 1
            try:
                states = self.sample_cluster(f"{stage}_{attempt:03d}", save=True)
                assert_ready(states, len(self.nodes))
                print(f"[ready] {stage}")
                return states
            except Exception as err:
                last_error = str(err)
                print(f"[wait] {stage}: {last_error}")
                time.sleep(2)
        raise RuntimeError(f"timeout waiting cluster ready at {stage}: {last_error}")

    def json_rpc(self, node: Node, method: str, params: dict[str, Any]) -> Any:
        body = {
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": 1,
        }
        value = self.vm_http(node, "POST", "http://127.0.0.1:3180/kapi/klog-service", body)
        if not isinstance(value, dict):
            raise RuntimeError(f"{node.vm} json-rpc returned non-object: {value!r}")
        if value.get("error"):
            raise RuntimeError(f"{node.vm} json-rpc {method} failed: {value['error']}")
        if "result" not in value:
            raise RuntimeError(f"{node.vm} json-rpc {method} missing result: {value!r}")
        return value["result"]

    def wait_json_rpc_ready(self, stage: str, timeout: float) -> None:
        deadline = time.monotonic() + timeout
        last_error = ""
        attempt = 0
        probe_node = self.nodes[0]
        while time.monotonic() < deadline:
            attempt += 1
            try:
                self.json_rpc(
                    probe_node,
                    "klog.log.query",
                    {
                        "start_id": 0,
                        "end_id": 0,
                        "limit": 1,
                        "desc": False,
                        "level": None,
                        "source": None,
                        "attr_key": None,
                        "attr_value": None,
                        "strong_read": True,
                    },
                )
                print(f"[ready] {stage}: /kapi/klog-service json-rpc")
                return
            except Exception as err:
                last_error = str(err)
                print(f"[wait] {stage}: /kapi/klog-service not ready: {last_error}")
                if attempt % 5 == 0:
                    try:
                        self.sample_cluster(f"{stage}_rpc_wait_{attempt:03d}", save=True)
                    except Exception as sample_err:
                        print(f"[wait] {stage}: cluster sample failed: {sample_err}")
                time.sleep(2)
        raise RuntimeError(f"timeout waiting /kapi/klog-service json-rpc ready at {stage}: {last_error}")

    def append_and_query(self, stage: str) -> int:
        source = "klog-3ood-vm-smoke"
        request_id = f"{stage}-{int(time.time() * 1000)}"
        append_node = self.nodes[0]
        try:
            append_result = self.json_rpc(
                append_node,
                "klog.log.append",
                {
                    "message": f"{stage} message",
                    "timestamp": None,
                    "node_name": append_node.node_name,
                    "level": "INFO",
                    "source": source,
                    "attrs": {"stage": stage},
                    "request_id": request_id,
                },
            )
        except Exception as err:
            self.record_rpc_error(stage, "append", append_node, err)
            raise
        self.save_json(self.rpc_dir, f"{stage}_append_{append_node.vm}", append_result)
        log_id = int(append_result["id"])
        for node in self.nodes:
            try:
                query_result = self.json_rpc(
                    node,
                    "klog.log.query",
                    {
                        "start_id": log_id,
                        "end_id": log_id,
                        "limit": 4,
                        "desc": False,
                        "level": "INFO",
                        "source": source,
                        "attr_key": None,
                        "attr_value": None,
                        "strong_read": True,
                    },
                )
            except Exception as err:
                self.record_rpc_error(stage, "query", node, err, {"log_id": log_id})
                raise
            self.save_json(self.rpc_dir, f"{stage}_query_{node.vm}", query_result)
            items = query_result.get("items") if isinstance(query_result, dict) else None
            if not items:
                err = RuntimeError(f"{node.vm} query did not return log id {log_id}: {query_result!r}")
                self.record_rpc_error(stage, "query", node, err, {"log_id": log_id})
                raise err
            if not any(int(item.get("id", -1)) == log_id and item.get("request_id") == request_id for item in items):
                err = RuntimeError(f"{node.vm} query mismatch for log id {log_id}: {query_result!r}")
                self.record_rpc_error(stage, "query", node, err, {"log_id": log_id})
                raise err
        for target_node in self.nodes:
            try:
                query_result = self.gateway_inter_query(append_node, target_node, log_id, source)
            except Exception as err:
                self.record_rpc_error(
                    stage,
                    "cluster_inter_query",
                    target_node,
                    err,
                    {"gateway_vm": append_node.vm, "log_id": log_id},
                )
                raise
            self.save_json(
                self.rpc_dir,
                f"{stage}_inter_query_{append_node.vm}_to_{target_node.node_name}",
                query_result,
            )
            items = query_result.get("items") if isinstance(query_result, dict) else None
            if not items:
                err = RuntimeError(
                    f"{append_node.vm}->{target_node.node_name} inter query did not return log id {log_id}: {query_result!r}"
                )
                self.record_rpc_error(
                    stage,
                    "cluster_inter_query",
                    target_node,
                    err,
                    {"gateway_vm": append_node.vm, "log_id": log_id},
                )
                raise err
            if not any(int(item.get("id", -1)) == log_id and item.get("request_id") == request_id for item in items):
                err = RuntimeError(
                    f"{append_node.vm}->{target_node.node_name} inter query mismatch for log id {log_id}: {query_result!r}"
                )
                self.record_rpc_error(
                    stage,
                    "cluster_inter_query",
                    target_node,
                    err,
                    {"gateway_vm": append_node.vm, "log_id": log_id},
                )
                raise err
        print(f"[ok] {stage}: append/query log_id={log_id} matched on service and cluster inter routes")
        return log_id

    def record_rpc_error(
        self,
        stage: str,
        action: str,
        node: Node,
        err: Exception,
        extra: dict[str, Any] | None = None,
    ) -> None:
        payload: dict[str, Any] = {
            "stage": stage,
            "action": action,
            "vm": node.vm,
            "node_name": node.node_name,
            "error": str(err),
            "time_utc": datetime.now(timezone.utc).isoformat(),
        }
        if extra:
            payload.update(extra)
        self.save_json(self.rpc_dir, f"{stage}_{action}_{node.vm}_error", payload)
        try:
            self.sample_cluster(f"{stage}_{action}_{node.vm}_error", save=True)
        except Exception as sample_err:
            payload["cluster_sample_error"] = str(sample_err)
            self.save_json(self.rpc_dir, f"{stage}_{action}_{node.vm}_error", payload)

    def restart_klog(self, node: Node, stage: str, timeout: float) -> dict[str, dict[str, Any]]:
        print(f"[restart] {stage}: kill klog_daemon on {node.vm}/{node.node_name}")
        self.vm_exec(
            node,
            [
                "sh",
                "-lc",
                "sudo pkill -TERM -x klog_daemon || true",
            ],
            timeout=10,
        )
        time.sleep(1)
        return self.wait_cluster_ready(stage, timeout)

    def soak(self, seconds: float, interval: float) -> None:
        if seconds <= 0:
            print("[skip] soak disabled")
            return
        deadline = time.monotonic() + seconds
        round_no = 0
        while time.monotonic() < deadline:
            round_no += 1
            stage = f"soak_{round_no:03d}"
            self.append_and_query(stage)
            self.sample_cluster(stage, save=True)
            sleep_for = min(interval, max(0.0, deadline - time.monotonic()))
            if sleep_for > 0:
                time.sleep(sleep_for)
        print(f"[ok] soak completed: rounds={round_no}, seconds={seconds:g}")


def validate_cluster_state(state: dict[str, Any], label: str) -> None:
    missing = [field for field in REQUIRED_DIAG_FIELDS if field not in state]
    if missing:
        raise RuntimeError(f"{label} cluster-state missing diagnostic fields: {missing}")
    if not isinstance(state.get("raft_metrics"), str) or not state["raft_metrics"]:
        raise RuntimeError(f"{label} cluster-state has empty raft_metrics")
    if not isinstance(state.get("current_term"), int):
        raise RuntimeError(f"{label} cluster-state current_term is not int: {state.get('current_term')!r}")
    if not isinstance(state.get("vote"), str):
        raise RuntimeError(f"{label} cluster-state vote is not str: {state.get('vote')!r}")


def assert_ready(states: dict[str, dict[str, Any]], expected_voters: int) -> None:
    leaders = {state.get("current_leader") for state in states.values()}
    if len(leaders) != 1 or None in leaders:
        raise RuntimeError(f"leader view is not converged: {leaders}")
    voters_sets = {tuple(sorted(state.get("voters") or [])) for state in states.values()}
    if len(voters_sets) != 1:
        raise RuntimeError(f"voter sets differ: {voters_sets}")
    voters = next(iter(voters_sets))
    if len(voters) != expected_voters:
        raise RuntimeError(f"unexpected voter count: expected={expected_voters}, voters={voters}")
    if leaders.pop() not in voters:
        raise RuntimeError(f"leader is not in voters: voters={voters}")
    learners = {tuple(sorted(state.get("learners") or [])) for state in states.values()}
    if learners != {()}:
        raise RuntimeError(f"expected no learners in baseline cluster: {learners}")


def find_leader_node(states: dict[str, dict[str, Any]], nodes: list[Node]) -> Node:
    leader = next(iter(states.values())).get("current_leader")
    for node in nodes:
        if states[node.vm].get("node_id") == leader:
            return node
    raise RuntimeError(f"leader id {leader!r} is not one of local VM nodes")


def find_follower_node(states: dict[str, dict[str, Any]], nodes: list[Node]) -> Node:
    leader = next(iter(states.values())).get("current_leader")
    for node in nodes:
        if states[node.vm].get("node_id") != leader:
            return node
    raise RuntimeError("no follower node found")


def print_cluster_summary(stage: str, states: dict[str, dict[str, Any]]) -> None:
    print(f"[cluster] {stage}")
    for vm, state in states.items():
        print(
            "  {vm}: node_id={node_id} state={server_state} leader={leader} "
            "term={term} vote={vote} last_log={last_log} last_applied={last_applied} "
            "quorum_ack_ms={quorum}".format(
                vm=vm,
                node_id=state.get("node_id"),
                server_state=state.get("server_state"),
                leader=state.get("current_leader"),
                term=state.get("current_term"),
                vote=state.get("vote"),
                last_log=state.get("last_log_index"),
                last_applied=state.get("last_applied"),
                quorum=state.get("millis_since_quorum_ack"),
            )
        )


def safe_name(name: str) -> str:
    return "".join(ch if ch.isalnum() or ch in "._-" else "_" for ch in name)


def default_report_dir() -> Path:
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    return REPO_ROOT / "test" / "reports" / "klog_3ood_vm_smoke" / stamp


def main() -> int:
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(line_buffering=True)

    parser = argparse.ArgumentParser(description="Run klog three-OOD VM smoke/soak checks.")
    parser.add_argument(
        "--nodes",
        default=os.environ.get("KLOG_3OOD_VM_NODES", DEFAULT_NODE_SPEC),
        help="Comma-separated vm:node_name list, default: %(default)s",
    )
    parser.add_argument(
        "--report-dir",
        type=Path,
        default=Path(os.environ["KLOG_3OOD_VM_REPORT_DIR"]) if os.environ.get("KLOG_3OOD_VM_REPORT_DIR") else default_report_dir(),
    )
    parser.add_argument("--http-timeout", type=float, default=float(os.environ.get("KLOG_3OOD_VM_HTTP_TIMEOUT", "5")))
    parser.add_argument("--ready-timeout", type=float, default=float(os.environ.get("KLOG_3OOD_VM_READY_TIMEOUT", "90")))
    parser.add_argument("--restart-timeout", type=float, default=float(os.environ.get("KLOG_3OOD_VM_RESTART_TIMEOUT", "150")))
    parser.add_argument("--soak-seconds", type=float, default=float(os.environ.get("KLOG_3OOD_VM_SOAK_SECONDS", "300")))
    parser.add_argument("--soak-interval", type=float, default=float(os.environ.get("KLOG_3OOD_VM_SOAK_INTERVAL", "5")))
    parser.add_argument("--log-lines", type=int, default=int(os.environ.get("KLOG_3OOD_VM_LOG_LINES", "300")))
    parser.add_argument("--skip-restarts", action="store_true", help="Only run readiness and append/query checks.")
    args = parser.parse_args()

    nodes = parse_nodes(args.nodes)
    smoke = VmSmoke(nodes, args.report_dir, args.http_timeout)
    print(f"[diag] nodes={args.nodes}")
    print(f"[diag] report_dir={args.report_dir}")

    states = smoke.wait_cluster_ready("initial", args.ready_timeout)
    smoke.collect_klog_logs("initial", args.log_lines)
    smoke.validate_gateway_routes("initial")
    smoke.wait_json_rpc_ready("initial", args.ready_timeout)
    smoke.append_and_query("initial")

    if not args.skip_restarts:
        follower = find_follower_node(states, nodes)
        states = smoke.restart_klog(follower, "after_follower_restart", args.restart_timeout)
        smoke.wait_json_rpc_ready("after_follower_restart", args.restart_timeout)
        smoke.append_and_query("after_follower_restart")

        leader = find_leader_node(states, nodes)
        states = smoke.restart_klog(leader, "after_leader_restart", args.restart_timeout)
        smoke.wait_json_rpc_ready("after_leader_restart", args.restart_timeout)
        smoke.append_and_query("after_leader_restart")

    smoke.soak(args.soak_seconds, args.soak_interval)
    states = smoke.wait_cluster_ready("final", args.ready_timeout)
    smoke.collect_klog_logs("final", args.log_lines)
    assert_ready(states, len(nodes))
    print("[ok] klog three-OOD VM smoke completed")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        print("[error] interrupted", file=sys.stderr)
        raise SystemExit(130)
    except Exception as err:
        print(f"[error] {err}", file=sys.stderr)
        raise SystemExit(1)
