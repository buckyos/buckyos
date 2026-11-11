#!/usr/bin/env python3
"""DV of the Goal gates (Goal Session next TODO, H acceptance) on the local Zone.

    python3 dv_gate.py builtin            # the built-in goals: package check scripts, checks run on the container's Deno
    python3 dv_gate.py create             # a test goal watching a folder, with a check script (prints its id)
    python3 dv_gate.py watch <goal> [n]   # n polls (10 s apart) of goal, schedule, state, activations
    python3 dv_gate.py break <goal>       # owner installs a broken check script
    python3 dv_gate.py set <goal> <json>  # goals.revise of schedule fields (owner)
    python3 dv_gate.py archive <goal>
"""
import base64
import hashlib
import json
import os
import ssl
import sys
import time
import urllib.request

AGENT = "jarvis.test.buckyos.io"
AGENT_ROOT = f"/opt/buckyos/data/home/devtest/agents/{AGENT}"
ZONE = os.environ.get("DV_ZONE", "https://test.buckyos.io")
CONTROL_PANEL = "http://127.0.0.1:4020/kapi/control-panel"
USER, PASSWORD = "devtest", "bucky2025"
INBOX = f"{AGENT_ROOT}/dv_inbox"
CTX = ssl.create_default_context()
CTX.check_hostname = False
CTX.verify_mode = ssl.CERT_NONE

CHECK = r'''// DV: wake the goal when a new file appears in dv_inbox/ of the AgentRoot.
export default async function check(ctx, state) {
  const dir = `${ctx.agent_root}/dv_inbox`;
  const seen = new Set((state && state.seen) || []);
  const now = [];
  try {
    for await (const e of Deno.readDir(dir)) if (e.isFile) now.push(e.name);
  } catch {
    return { run: false, state: { seen: [...seen] } };
  }
  const fresh = now.filter((n) => !seen.has(n));
  return fresh.length
    ? { run: true, reason: `new file(s) in dv_inbox: ${fresh.join(", ")}`, state: { seen: now } }
    : { run: false, state: { seen: now } };
}
'''

BROKEN = "// DV: broken on purpose\nexport default async function check(ctx, state) {\n  throw new Error('broken on purpose by the DV');\n}\n"


def login():
    nonce = int(time.time() * 1000)
    base = base64.b64encode(hashlib.sha256(f"{PASSWORD}{USER}.buckyos".encode()).digest()).decode()
    salted = base64.b64encode(hashlib.sha256(f"{base}{nonce}".encode()).digest()).decode()
    body = {"method": "auth.login", "params": {"username": USER, "password": salted, "appid": "control-panel",
            "target": {"kind": "system", "service_id": "control-panel"}, "login_nonce": nonce}, "sys": [nonce]}
    req = urllib.request.Request(CONTROL_PANEL, data=json.dumps(body).encode(), headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.load(r)["result"]["session_token"]


TOKEN = None


def call(method, params=None):
    global TOKEN
    TOKEN = TOKEN or login()
    seq = int(time.time() * 1000)
    body = {"method": method, "params": params or {}, "sys": [seq, TOKEN]}
    req = urllib.request.Request(f"{ZONE}/kapi/{AGENT}", data=json.dumps(body).encode(), headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=60, context=CTX) as r:
        out = json.load(r)
    if "error" in out and out["error"]:
        raise SystemExit(f"{method}: {out['error']}")
    return out.get("result")


def snap(goal_id, label):
    g = call("goals.get", {"goal_id": goal_id})
    s = call("goals.schedule", {"goal_id": goal_id}) or {}
    st = call("goals.state", {"goal_id": goal_id}) or {}
    acts = call("goals.activations", {"goal_id": goal_id, "query": {"limit": 10}}) or []
    print(json.dumps({
        "t": time.strftime("%H:%M:%S"), "label": label, "status": g["status"],
        "check": ((g.get("schedule") or {}).get("check") or {}).get("version"), "check_source": ((g.get("schedule") or {}).get("check") or {}).get("source"),
        "timetable": s.get("timetable"), "last_check": s.get("last_check"), "check_errors": s.get("check_errors"),
        "last_check_error": (s.get("last_check_error") or "")[:160], "stats": s.get("stats"),
        "skipped": [k["reason"] for k in (s.get("skipped") or [])][-6:], "holding": st.get("holding"),
        "no_action_in_a_row": st.get("consecutive_no_action"), "usage": st.get("usage"),
        "activations": [{"seq": a["seq"], "reasons": [(r["kind"], (r.get("note") or "")[:120]) for r in a["reasons"]],
                         "decision": a["decision"], "outcome": a["outcome"], "tokens": a["cost"]["total_tokens"]} for a in acts],
    }, ensure_ascii=False), flush=True)


def main():
    cmd = sys.argv[1]
    if cmd == "builtin":
        for g in call("goals.list"):
            ext = (g.get("extensions") or {}).get("opendan") or {}
            s = call("goals.schedule", {"goal_id": g["goal_id"]}) or {}
            print(json.dumps({"goal": g["goal_id"], "builtin": ext.get("builtin"), "status": g["status"],
                              "check": (g.get("schedule") or {}).get("check"), "timetable": s.get("timetable"),
                              "last_check": s.get("last_check"), "check_errors": s.get("check_errors"),
                              "last_check_error": s.get("last_check_error"), "stats": s.get("stats"),
                              "switched_off": s.get("switched_off"), "skipped": [k["reason"] for k in (s.get("skipped") or [])][-6:]},
                             ensure_ascii=False), flush=True)
        print(json.dumps({"skills_state_crystallize": (call("skills.state") or {}).get("crystallize")}))
    elif cmd == "create":
        os.makedirs(INBOX, exist_ok=True)
        req = {
            "key": f"dv-gate-{int(time.time())}",
            "title": "DV: watch dv_inbox",
            "objective": f"Whenever a new file appears in {INBOX}, append its name and size as one line to {AGENT_ROOT}/dv_seen.txt. Keep your check script working: when it fails, fix it.",
            "lifecycle": "maintain",
            "healthy_state": "every file in dv_inbox is listed in dv_seen.txt",
            "distance_evaluation": "files in dv_inbox not yet in dv_seen.txt",
            "schedule": {"every_secs": 60, "min_interval_secs": 60, "guarantee_secs": 600},
            "budget": {"activation": {"max_total_tokens": 200000, "max_runs": 6}, "period": {"max_total_tokens": 1500000, "max_works": 0}},
            "check_script": CHECK,
        }
        g = call("goals.create", {"request": req})
        print(g["goal_id"])
    elif cmd == "watch":
        n = int(sys.argv[3]) if len(sys.argv) > 3 else 6
        for i in range(n):
            snap(sys.argv[2], f"poll {i}")
            if i + 1 < n:
                time.sleep(10)
    elif cmd == "break":
        g = call("goals.set_check", {"goal_id": sys.argv[2], "request": {"script": BROKEN, "reason": "DV: broken on purpose"}})
        print(json.dumps((g.get("schedule") or {}).get("check")))
    elif cmd == "set":
        g = call("goals.get", {"goal_id": sys.argv[2]})
        sched = dict(g["schedule"])
        sched.update(json.loads(sys.argv[3]))
        sched.pop("check", None)
        g = call("goals.revise", {"goal_id": sys.argv[2], "request": {"expected_revision": g["revision"], "schedule": sched, "reason": "DV"}})
        print(json.dumps(g["schedule"]))
    elif cmd == "archive":
        print(call("goals.set_status", {"goal_id": sys.argv[2], "request": {"status": "archived", "reason": "DV done"}})["status"])
    elif cmd == "call":
        print(json.dumps(call(sys.argv[2], json.loads(sys.argv[3]) if len(sys.argv) > 3 else {}), ensure_ascii=False)[:4000])


if __name__ == "__main__":
    main()
