"""Run with python3 test/object_url_dv/owner_provider_dv.py.

Build agent_tool_cli_dev's agent_tool binary under src and build the sibling
buckyos-websdk distribution first. The Rust CLI, Deno runner, and distributed
owner calendar action are real. HTTP fixtures supply the Zone identity and
Control Panel claims and serve two independently addressable calendar providers.
Control Panel's own CAS and history are covered in its owner module tests.
"""

import json
import os
import shutil
import subprocess
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

B = "obj://msg-center/fixtures/calendar-b"
C = "obj://msg-center/fixtures/calendar-c"
MARKERS = {B: "a" * 64, C: "b" * 64}
ROW = "https://provider.example/records/7#subsection"
ALICE = "did:bns:alice"


def tagged(provider):
    return f"{ROW}#buckyos-owner-v1={MARKERS[provider]}"


class Service(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def reply(self, value):
        body = json.dumps(value).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    @property
    def provider(self):
        return B if "calendar-b" in self.path else C

    def do_GET(self):
        state = self.server
        base = state.origin + "/kapi/msg-center/obj/fixtures/" + self.provider.rsplit("/", 1)[1]
        if self.path.endswith("/profile"):
            profile = json.loads(state.profile.read_text())
            actions = profile["actions"]
            if state.query_mode != "undeclared":
                actions["get_submission"] = {"x-buckyos:action": {"effect": "read"}}
            if state.policy == "read_only":
                actions.pop("update")
            if state.policy == "update_reads":
                actions["update"]["x-buckyos:action"]["effect"] = "read"
            if state.policy == "list_writes":
                actions["list"]["x-buckyos:action"]["effect"] = "write"
            if state.policy == "changed_slot":
                profile["x-buckyos:slot"] = "todo"
                profile["x-buckyos:traits"] = ["buckyos.todo"]
            for method, action in actions.items():
                action["forms"] = [{"href": base + "/execute/" + method, "op": ["invokeaction"]}]
            self.reply(profile)
            return
        self.reply({
            "@context": ["https://www.w3.org/ns/did/v1"], "id": "did:bns:fixture",
            "controller": ALICE, "alsoKnownAs": [self.provider, base],
            "service": [{"id": "#did-object", "type": "DIDObjectService", "serviceEndpoint": base,
                         "profile": base + "/profile", "kind": "calendar"}],
        })

    def do_POST(self):
        state = self.server
        msg = json.loads(self.rfile.read(int(self.headers["content-length"])))
        method, params = msg["method"], msg["params"]
        if "/execute/" in self.path:
            if self.headers.get("Authorization") != "Bearer fixture-only":
                self.send_error(403)
                return
            provider = self.provider
            state.requests.append((method, provider))
            if method == "get_submission":
                if state.query_mode == "revoked":
                    self.reply({"error": {"code": "PERMISSION_DENIED", "message": "access withdrawn"}})
                    return
                key = (provider, params["action"], params["idempotency_key"])
                value = state.receipts.get(key)
                self.reply({"result": {"status": "accepted", "result": value}
                            if value and state.query_mode != "not_found" else {"status": "not_found"}})
                return
            if method == "list":
                self.reply({"result": {"events": [{"title": "Listed", "record_ref": ROW, "open_url": ROW}],
                                       "next_cursor": None}})
                return
            key = (provider, method, msg["idempotency_key"])
            if key not in state.receipts:
                state.applied.append((key, params))
                state.receipts[key] = {"status": "created" if method == "add" else "updated",
                                       "record_ref": ROW, "open_url": ROW, "owner_did": ALICE,
                                       "idempotency_key": msg["idempotency_key"]}
            if state.drop_write_response:
                state.drop_write_response = False
                self.close_connection = True
                return
            self.reply({"result": state.receipts[key]})
            return

        if method == "owner.context":
            self.reply({"result": {"read_only": state.read_only, "owner_did": ALICE, "owner_user_id": "alice",
                                   "user_id": "alice", "caller_did": ALICE, "zone_did": "did:bns:zone",
                                   "object_alias": None}})
        elif method == "owner.prepare":
            key = (params["action"], params["idempotency_key"])
            if key in state.claims:
                self.reply({"result": {**state.claims[key], "replay": True}})
                return
            provider, original = state.current, {}
            if "record_ref" in params:
                raw, marker = params["record_ref"].rsplit("#buckyos-owner-v1=", 1)
                provider = next(p for p, m in MARKERS.items() if m == marker)
                original["record_ref"] = raw
            target = {"mode": "object", "provider_ref": provider, "record_binding": MARKERS[provider],
                      "provider_params": original, "binding_revision": 3 if provider == B else 4,
                      "replay": False, "idempotency_key": key[1], "input_digest": params["input_digest"]}
            if key[0] != "list":
                state.claims[key] = target
            self.reply({"result": target})
        else:
            self.send_error(404, "unexpected Zone method")


class OwnerProviderDV(unittest.TestCase):
    def setUp(self):
        repo = Path(__file__).resolve().parents[2]
        self.binary = repo / "src/target/debug" / ("agent_tool.exe" if os.name == "nt" else "agent_tool")
        sdk_dist = repo.parent / "buckyos-websdk/dist/owner-extensions"
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        fixture = Path(temp.name)
        self.cwd = fixture
        tool = fixture / "libexec/buckyos-tool"
        shutil.copytree(sdk_dist, tool / "dist/owner-extensions")
        runtime = repo / "src/rootfs/libexec/buckyos-tool/runtime"
        deno = "deno.exe" if os.name == "nt" else "deno"
        (tool / "runtime").mkdir()
        shutil.copy2(runtime / deno, tool / "runtime" / deno)
        server = ThreadingHTTPServer(("127.0.0.1", 0), Service)
        self.addCleanup(server.server_close)
        server.origin = f"http://127.0.0.1:{server.server_port}"
        server.profile = tool / "dist/owner-extensions/profiles/calendar.json"
        server.current, server.policy, server.query_mode = B, "writable", "available"
        server.claims, server.receipts, server.applied, server.requests = {}, {}, [], []
        server.drop_write_response = server.read_only = False
        threading.Thread(target=server.serve_forever, daemon=True).start()
        self.addCleanup(server.shutdown)
        self.state = server
        self.env = {**os.environ, "BUCKYOS_ROOT": str(fixture), "BUCKYOS_TOOL_ENDPOINT": server.origin,
                    "BUCKYOS_APPCLIENT_SESSION_TOKEN": "fixture-only"}

    def call(self, method, params, key, *, object_ref="owner.cal", error=None):
        run = subprocess.run([self.binary, "xcall", object_ref, method, "--params", json.dumps(params),
                              "--idempotency-key", key], cwd=self.cwd, env=self.env, capture_output=True,
                             text=True, timeout=30)
        output = json.loads(run.stdout)
        if error:
            self.assertNotEqual(run.returncode, 0, output)
            self.assertIn(error, run.stdout)
            return output
        self.assertEqual(run.returncode, 0, (output, run.stderr))
        return output["detail"]["result"]

    def test_rebinding_lost_reply_permissions_and_record_provenance(self):
        s = self.state
        event = {"title": "Fixture", "start": "2026-11-20T13:00:00Z", "end": "2026-11-20T14:00:00Z"}
        window = {"from": "2026-11-20T00:00:00Z", "to": "2026-11-21T00:00:00Z"}
        first = self.call("add", event, "add-b")
        self.assertEqual(first["record_ref"], tagged(B))
        self.assertEqual(first["provider_record_ref"], ROW)
        s.current = C
        self.assertEqual(self.call("add", event, "add-b"), first)
        self.assertEqual(s.requests, [("add", B), ("get_submission", B)])
        for mode, error in [("revoked", "PERMISSION_DENIED"), ("not_found", "WRITE_OUTCOME_UNKNOWN"),
                            ("undeclared", "WRITE_OUTCOME_UNKNOWN")]:
            s.query_mode = mode
            before = len(s.requests)
            result = self.call("add", event, "add-b", error=error)
            self.assertNotIn(ROW, json.dumps(result))
            self.assertEqual(s.requests[before:], [] if mode == "undeclared" else [("get_submission", B)])
        s.query_mode = "available"
        update = {"record_ref": tagged(B), "patch": {"title": "Change B"}}
        self.call("update", update, "update-b")
        self.assertEqual(s.applied[-1][0], (B, "update", "update-b"))
        self.assertEqual(s.applied[-1][1]["record_ref"], ROW)
        self.call("add", event, "add-b", object_ref=B)
        s.current, s.drop_write_response = B, True
        self.call("add", event, "lost-b", error="WRITE_OUTCOME_UNKNOWN")
        s.current = C
        before = len(s.requests)
        self.assertEqual(self.call("add", event, "lost-b")["record_ref"], tagged(B))
        self.assertEqual(s.requests[before:], [("get_submission", B)])
        for provider in (C, B):
            s.current = provider
            row = self.call("list", window, "list")["events"][0]
            self.assertEqual((row["record_ref"], row["provider_record_ref"], row["open_url"]),
                             (tagged(provider), ROW, ROW))
        self.call("update", {**update, "record_ref": tagged(C)}, "update-c")
        self.assertEqual([key for key, _ in s.applied],
                         [(B, "add", "add-b"), (B, "update", "update-b"),
                          (B, "add", "lost-b"), (C, "update", "update-c")])
        before = len(s.requests)
        for policy in ["read_only", "update_reads", "changed_slot"]:
            s.policy = policy
            self.call("update", update, policy,
                      error="PROVIDER_TYPE_MISMATCH" if policy == "changed_slot" else "UNSUPPORTED_ACTION")
        s.policy, s.read_only = "list_writes", True
        self.call("list", window, "unsafe-list", error="PROVIDER_TYPE_MISMATCH")
        self.assertEqual(s.requests[before:], [])


if __name__ == "__main__":
    unittest.main()
