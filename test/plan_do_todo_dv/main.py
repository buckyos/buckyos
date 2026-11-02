#!/usr/bin/env python3
"""Real-model DV of the plan session (plan_simple: a TodoList whose Todos the
host runs in work sessions of their own; see README.md).

    python3 test/plan_do_todo_dv/main.py all [--variant normal|failure|user-update] [--case DIR]
    python3 test/plan_do_todo_dv/main.py prepare|run|collect|verify --case DIR

Without arguments (as `test/run.py -p plan_do_todo_dv` runs it) the normal
case runs only when PLAN_DO_DV=1; otherwise it reports NOT RUN (exit 77).
A missing provider or credential fails explicitly; nothing falls back to a
mock.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
JARVIS = REPO / "src" / "apps" / "jarvis_agent" / "agent"
AGENT_DID = "did:bns:jarvis.todo-test"
WHO = "app:xagent-test@local"
MARKERS = {"A": "ACCEPT-A-7Q2", "B": "ACCEPT-B-5M8", "C": "ACCEPT-C-3K9"}
# What the user asks for while investigation B runs (variant user-update).
UPDATE_MARKER = "ACCEPT-B-EXTRA-9Z1"
USER_UPDATE = f"补充要求：调查 B 的复现脚本运行时还要再打印一行 `{UPDATE_MARKER} <参与测试的租户数>`，并在报告里说明这个数。"
DEFAULT_BASE_URL = "http://127.0.0.1:8317/v1"
KEYS_ENV = Path("/opt/cli-proxy-api/keys.env")
RUN_TIMEOUT_SECS = int(os.environ.get("PLAN_DO_TIMEOUT_SECS", "3600"))
EXIT_NOT_RUN = 77


def log(msg: str) -> None:
    print(f"[plan_do_dv] {msg}", flush=True)


def fail(msg: str, code: int = 2) -> None:
    print(f"[plan_do_dv] FAILED: {msg}", file=sys.stderr, flush=True)
    sys.exit(code)


def read_json(p: Path, default=None):
    try:
        return json.loads(p.read_text(encoding="utf-8"))
    except FileNotFoundError:
        return default


def write_json(p: Path, v) -> None:
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(v, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def read_jsonl(p: Path) -> list[dict]:
    out = []
    try:
        for line in p.read_text(encoding="utf-8").splitlines():
            line = line.strip()
            if line:
                try:
                    out.append(json.loads(line))
                except json.JSONDecodeError:
                    pass
    except FileNotFoundError:
        pass
    return out


def sha256_file(p: Path) -> str:
    return hashlib.sha256(p.read_bytes()).hexdigest()


# ---------------------------------------------------------------------------
# Provider
# ---------------------------------------------------------------------------


def api_key() -> str | None:
    key = os.environ.get("PLAN_DO_API_KEY")
    if key:
        return key
    try:
        for line in KEYS_ENV.read_text().splitlines():
            if line.startswith("API_KEY="):
                return line.split("=", 1)[1].strip().strip('"')
    except OSError:
        pass
    return None


def provider() -> dict:
    base_url = os.environ.get("PLAN_DO_BASE_URL", DEFAULT_BASE_URL).rstrip("/")
    key = api_key()
    if not key:
        fail(f"no credential: set PLAN_DO_API_KEY (or make {KEYS_ENV} readable)")
    plan_model = os.environ.get("PLAN_DO_PLAN_MODEL", "gpt-6-sol")
    code_model = os.environ.get("PLAN_DO_CODE_MODEL", "gpt-6-sol")
    try:
        req = urllib.request.Request(f"{base_url}/models", headers={"Authorization": f"Bearer {key}"})
        with urllib.request.urlopen(req, timeout=20) as r:
            models = [m.get("id") for m in json.load(r).get("data", [])]
    except Exception as e:  # noqa: BLE001
        fail(f"provider {base_url} is not reachable: {e}")
    for m in {plan_model, code_model}:
        if m not in models:
            fail(f"model `{m}` is not offered by {base_url} (offered: {', '.join(models)})")
    return {"base_url": base_url, "key": key, "plan_model": plan_model, "code_model": code_model}


# ---------------------------------------------------------------------------
# prepare
# ---------------------------------------------------------------------------


def build_xagent() -> Path:
    log("building xagent")
    out = subprocess.run(
        [
            "cargo", "build", "--manifest-path", str(REPO / "src" / "Cargo.toml"),
            "-p", "libopendan", "--bin", "xagent", "--message-format=json-render-diagnostics",
        ],
        cwd=REPO, capture_output=True, text=True,
    )
    if out.returncode != 0:
        sys.stderr.write(out.stderr[-4000:])
        fail("cargo build of xagent failed")
    exe = None
    for line in out.stdout.splitlines():
        try:
            m = json.loads(line)
        except json.JSONDecodeError:
            continue
        if m.get("reason") == "compiler-artifact" and m.get("target", {}).get("name") == "xagent" and m.get("executable"):
            exe = m["executable"]
    if not exe:
        fail("cargo did not report the xagent executable")
    return Path(exe)


MODEL_ALIASES = {"llm.plan": "plan_model", "llm.code": "code_model", "llm.chat": "code_model"}


def map_models(behaviors: Path, prov: dict) -> dict:
    """The provider is reached directly: the package's model aliases become
    the provider's model ids in this case's copy (recorded)."""
    mapping = {}
    for f in sorted(behaviors.glob("*.toml")):
        text = f.read_text(encoding="utf-8")
        new = text
        for alias, key in MODEL_ALIASES.items():
            new = new.replace(f'preferred = "{alias}"', f'preferred = "{prov[key]}"')
        if new != text:
            m = re.search(r'preferred = "([^"]+)"', new)
            orig = re.search(r'preferred = "([^"]+)"', text)
            mapping[f.name] = {"from": orig.group(1) if orig else None, "to": m.group(1) if m else None}
            f.write_text(new, encoding="utf-8")
    return mapping


def prepare(case: Path, variant: str) -> None:
    if case.exists() and any(case.iterdir()):
        fail(f"{case} is not empty: every run starts from a new case directory")
    prov = provider()
    exe = build_xagent()
    case.mkdir(parents=True, exist_ok=True)
    for d in ["agent", "sessions", "kmsg", "config", "parent", "children", "serial", "workspace"]:
        (case / d).mkdir(parents=True, exist_ok=True)
    # The agent: the Jarvis package under test, model aliases mapped.
    agent = case / "agent"
    for name in ["agent.toml", "role.md", "self.md"]:
        if (JARVIS / name).exists():
            shutil.copy2(JARVIS / name, agent / name)
    shutil.copytree(JARVIS / "behaviors", agent / "behaviors")
    if (JARVIS / "i18n").exists():
        shutil.copytree(JARVIS / "i18n", agent / "i18n")
    shutil.copytree(JARVIS / "behaviors", case / "config" / "behaviors.package")
    mapping = map_models(agent / "behaviors", prov)
    shutil.copytree(agent / "behaviors", case / "config" / "behaviors.case")
    # The fixture: a fresh read-only copy; the failure variant lacks the
    # idempotency module the second exploration needs.
    fixture = case / "fixture"
    shutil.copytree(HERE / "fixture", fixture, ignore=shutil.ignore_patterns("__pycache__", "*.pyc"))
    if variant == "failure":
        (fixture / "idempotency.py").unlink()
    hashes = {str(p.relative_to(fixture)): sha256_file(p) for p in sorted(fixture.rglob("*")) if p.is_file()}
    combined = hashlib.sha256(json.dumps(hashes, sort_keys=True).encode()).hexdigest()
    for p in fixture.rglob("*"):
        perm = p.stat().st_mode
        p.chmod(perm & ~(stat.S_IWUSR | stat.S_IWGRP | stat.S_IWOTH))
    task = (HERE / "task_simple.md").read_text(encoding="utf-8").replace("{{FIXTURE}}", str(fixture))
    (case / "task.txt").write_text(task, encoding="utf-8")
    llm_context = {
        "provider": {"type": "openai", "base_url": prov["base_url"], "api_key_env": "PLAN_DO_API_KEY"},
        "model": prov["code_model"],
        "max_tokens": 16384,
        "llm_timeout": 900,
        "timeout": RUN_TIMEOUT_SECS,
        "tools": {"enabled": True},
    }
    write_json(case / "llm-context.json", llm_context)
    write_json(case / "config" / "case.json", {
        "mode": "plan_simple",
        "variant": variant,
        "agent_did": AGENT_DID,
        "who": WHO,
        "provider": {k: v for k, v in prov.items() if k != "key"},
        "model_map": mapping,
        "fixture": {"path": str(fixture), "files": hashes, "sha256": combined},
        "xagent": {"path": str(exe), "sha256": sha256_file(exe)},
        "behaviors_sha256": {f.name: sha256_file(f) for f in sorted((agent / "behaviors").glob("*.toml"))},
        "timeout_secs": RUN_TIMEOUT_SECS,
        "prepared_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    })
    log(f"prepared {case} ({variant}); fixture sha256 {combined[:12]}, xagent {exe}")


# ---------------------------------------------------------------------------
# run
# ---------------------------------------------------------------------------


def case_env(case: Path) -> dict:
    env = dict(os.environ)
    key = api_key()
    if not key:
        fail("no credential for the run (PLAN_DO_API_KEY)")
    env.update({
        "OPENDAN_AGENT_ROOT": str(case / "agent"),
        "OPENDAN_AGENT_DID": AGENT_DID,
        "LIBOPENDAN_WHO": WHO,
        "LIBOPENDAN_QUEUE_DIR": str(case / "kmsg"),
        "PLAN_DO_API_KEY": key,
    })
    env.pop("OPENDAN_SESSION_ID", None)
    env.pop("LIBOPENDAN_FAULT", None)
    return env


def xagent(case: Path) -> str:
    return read_json(case / "config" / "case.json")["xagent"]["path"]


def todo_running(sdir: Path, marker: str) -> dict | None:
    """The running attempt of the Todo whose body names `marker`."""
    lst = read_json(sdir / ".opendan_agent_session" / "todos.json", {}) or {}
    for t in lst.get("todos", []):
        a = (t.get("attempts") or [None])[-1]
        if marker in t.get("content", "") and a and a.get("status") == "running" and a.get("child_sid"):
            return {"todo_id": t["todo_id"], "attempt": a["attempt"], "child_sid": a["child_sid"]}
    return None


def run(case: Path) -> int:
    env = case_env(case)
    exe = xagent(case)
    task = (case / "task.txt").read_text(encoding="utf-8")
    variant = read_json(case / "config" / "case.json")["variant"]
    # The deliverables' workspace (on this machine), registered like a
    # user's project; the plan session is created by its class.
    r = subprocess.run(
        [exe, "workspace", "import", "--key", "plan-simple-dv", "--runtime", "container",
         "--directory", str(case / "workspace"), "--name", "review deliverables"],
        env=env, capture_output=True, text=True,
    )
    if r.returncode != 0:
        fail(f"xagent workspace import failed ({r.returncode}): {r.stderr}{r.stdout}")
    write_json(case / "workspace.json", json.loads(r.stdout))
    ws = read_json(case / "workspace.json")["workspace_id"]
    with open(case / "create.json", "w") as out:
        r = subprocess.run(
            [exe, "new", "--class", "plan_simple", "--workspace", ws, "--objective", task,
             "--parent", str(case / "sessions"), "--llm-context", f"@{case / 'llm-context.json'}", "--no-run"],
            env=env, stdout=out, stderr=subprocess.PIPE, text=True,
        )
    if r.returncode != 0:
        fail(f"xagent new failed ({r.returncode}): {r.stderr}")
    created = read_json(case / "create.json")
    sid, sdir = created["session_id"], Path(created["path"])
    log(f"session {sid}: driving (timeout {RUN_TIMEOUT_SECS}s)")
    started = time.time()
    posted = None
    with open(case / "drive.json", "w") as out, open(case / "drive.stderr.log", "w") as err:
        proc = subprocess.Popen(
            [exe, "run", sid, "--until", "finished", "--max-children", "4", "--max-wait", str(RUN_TIMEOUT_SECS)],
            env=env, stdout=out, stderr=err,
        )
        while proc.poll() is None:
            if time.time() - started > RUN_TIMEOUT_SECS + 300:
                proc.kill()
                break
            # The user changes investigation B while it runs: the message
            # goes to the plan's queue, as the conversation forwards it.
            if variant == "user-update" and posted is None:
                running = todo_running(sdir, MARKERS["B"])
                if running:
                    p = subprocess.run([exe, "post", sid, "--msg", USER_UPDATE], env=env, capture_output=True, text=True)
                    posted = {"at_ms": int(time.time() * 1000), "while": running, "exit": p.returncode, "out": p.stdout + p.stderr}
                    write_json(case / "user_update.json", posted)
                    log(f"posted the user's change while {running['todo_id']} runs in {running['child_sid']}")
            time.sleep(2)
        code = proc.returncode if proc.returncode is not None else -1
    write_json(case / "run.json", {"session_id": sid, "exit_code": code, "seconds": round(time.time() - started, 1)})
    log(f"xagent run exited {code} after {time.time() - started:.0f}s")
    collect(case)
    return code


# ---------------------------------------------------------------------------
# collect
# ---------------------------------------------------------------------------


def location(case: Path, sid: str) -> Path:
    for e in read_json(case / "sessions.json", []) or []:
        if e.get("session_id") == sid:
            return Path(e["location"])
    return case / "sessions" / sid


def capture(case: Path, args: list[str], path: Path) -> None:
    r = subprocess.run([xagent(case), *args], env=case_env(case), capture_output=True, text=True)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(r.stdout if r.returncode == 0 else r.stdout + r.stderr, encoding="utf-8")


def copy_session(src: Path, dst: Path) -> None:
    if dst.exists():
        shutil.rmtree(dst)
    shutil.copytree(src, dst, symlinks=True, ignore=shutil.ignore_patterns("*.lock", "lease.json"))


def collect(case: Path) -> None:
    sid = read_json(case / "run.json")["session_id"]
    capture(case, ["list"], case / "sessions.json")
    pdir = location(case, sid)
    capture(case, ["status", sid, "--worklog", "200", "--report", "--run", "--events"], case / "parent" / "status.json")
    capture(case, ["todo", "list", sid, "--json"], case / "parent" / "todos.json")
    capture(case, ["todo", "list", sid], case / "parent" / "todo_view.xml")
    copy_session(pdir, case / "parent" / "session")
    todos = read_json(case / "parent" / "todos.json", {}) or {}
    for t in todos.get("todos", []):
        for a in t.get("attempts", []):
            c = a.get("child_sid")
            if c:
                cdir = location(case, c)
                if cdir.exists():
                    copy_session(cdir, case / "children" / c / "session")
                capture(case, ["status", c, "--report", "--worklog", "200", "--run"], case / "children" / c / "status.json")
                write_json(case / "children" / c / "binding.json", {"todo_id": t["todo_id"], "attempt": a})
    for name in ["review.md", "decisions.md", "report.md"]:
        if (pdir / name).exists():
            shutil.copy2(pdir / name, case / name if name != "report.md" else case / "parent" / "report.md")
        if name != "report.md" and (case / "workspace" / name).exists():
            shutil.copy2(case / "workspace" / name, case / name)
    write_json(case / "timeline.json", timeline(case, pdir, todos))
    log(f"evidence collected in {case}")


def outputs_of(sdir: Path) -> str:
    """Every tool result a session recorded (its run directories are pruned)."""
    return "\n".join(
        str(e.get("result", ""))
        for e in read_jsonl(sdir / ".opendan_agent_session" / "worklog.jsonl")
        if e.get("t") == "action_result"
    )


def child_window(cdir: Path) -> dict:
    wl = read_jsonl(cdir / ".opendan_agent_session" / "worklog.jsonl")
    starts = [e["at_ms"] for e in wl if e.get("t") == "turn_started"]
    ends = [e["at_ms"] for e in wl if e.get("t") == "turn_ended"]
    rounds = [u.get("at_ms") for u in read_jsonl(cdir / ".opendan_agent_session" / "usage.jsonl") if u.get("at_ms")]
    tools = []
    for f in (cdir / ".opendan_agent_session" / "runs").glob("*/exec/*/command"):
        tools.append(int(f.stat().st_mtime * 1000))
    return {
        "start_ms": min(starts) if starts else None,
        "first_round_ms": min(rounds) if rounds else None,
        "first_tool_ms": min(tools) if tools else None,
        "rounds": len(rounds),
        "tool_calls": len(tools),
        "end_ms": max(ends) if ends else None,
    }


def timeline(case: Path, pdir: Path, todos: dict) -> list[dict]:
    events = []
    for e in read_jsonl(pdir / ".opendan_agent_session" / "worklog.jsonl"):
        t = e.get("t")
        if t in ("turn_started", "input_batch") and e.get("at_ms"):
            srcs = sorted({i.get("src") for i in e.get("inputs", [])})
            events.append({"at_ms": e["at_ms"], "who": "parent", "what": f"{t} {e.get('hook')} inputs={srcs}"})
        elif t == "turn_ended":
            events.append({"at_ms": e["at_ms"], "who": "parent", "what": f"turn_ended {e.get('status')}"})
    for u in read_jsonl(pdir / ".opendan_agent_session" / "usage.jsonl"):
        events.append({"at_ms": u.get("at_ms"), "who": "parent", "what": f"round {u.get('model')} run {u.get('run_id')}"})
    for t in todos.get("todos", []):
        for a in t.get("attempts", []):
            events.append({"at_ms": a.get("started_at_ms"), "who": t["todo_id"], "what": f"attempt {a['attempt']} {a['mode']} started ({a.get('child_sid')})"})
            if a.get("ended_at_ms"):
                events.append({"at_ms": a["ended_at_ms"], "who": t["todo_id"], "what": f"attempt {a['attempt']} {a['status']}: {a.get('summary')}"})
        for u in t.get("updates", []):
            events.append({"at_ms": u["at_ms"], "who": t["todo_id"], "what": f"update {','.join(u['fields'])}: {u['reason'][:120]}"})
        for a in t.get("attempts", []):
            c = a.get("child_sid")
            if c:
                w = child_window(location(case, c))
                for k in ("start_ms", "first_round_ms", "first_tool_ms", "end_ms"):
                    if w.get(k):
                        events.append({"at_ms": w[k], "who": c[:16], "what": k})
    events = [e for e in events if e.get("at_ms")]
    events.sort(key=lambda e: e["at_ms"])
    return events


# ---------------------------------------------------------------------------
# verify
# ---------------------------------------------------------------------------


class Checks:
    def __init__(self) -> None:
        self.items: list[dict] = []

    def check(self, cid: str, ok: bool, detail) -> bool:
        self.items.append({"id": cid, "ok": bool(ok), "detail": detail})
        log(f"{'PASS' if ok else 'FAIL'} {cid}: {detail if not ok else ''}".rstrip(": "))
        return bool(ok)


def tool_calls(entries: list[dict]) -> list[tuple[dict, dict]]:
    """(assistant_message entry, tool call) of a function call context."""
    return [(e, tc) for e in entries if e.get("t") == "assistant_message" for tc in e.get("tool_calls", []) or []]


def verify(case: Path) -> int:
    """The plan session: a whole TodoList in its first Turn, every Todo in a
    work session of its own started by the host as soon as its
    prerequisites completed (before the plan infers again), no dispatch /
    wait / review, a verification Todo, the deliverables written by the
    Todo that inherited the workspace the plan only reads, Turns bounded by
    what came back; user-update: the user's change reaches the running
    investigation as a Todo update."""
    c = Checks()
    meta = read_json(case / "config" / "case.json")
    variant = meta["variant"]
    runinfo = read_json(case / "run.json", {})
    sid = runinfo.get("session_id")
    pdir = case / "parent" / "session"
    state = read_json(pdir / ".opendan_agent_session" / "state.json", {})
    cfg = read_json(pdir / ".opendan_agent_session" / "session_config.json", {}) or {}
    wl = read_jsonl(pdir / ".opendan_agent_session" / "worklog.jsonl")
    todos = (read_json(case / "parent" / "todos.json", {}) or {}).get("todos", [])
    by_id = {t["todo_id"]: t for t in todos}
    ws_id = (read_json(case / "workspace.json", {}) or {}).get("workspace_id")
    ok_run = variant == "failure" or runinfo.get("exit_code") == 0
    c.check("run.exit", ok_run, runinfo)
    c.check("parent.finished", state.get("run_state") == "finished", state.get("one_line_status"))
    session = cfg.get("session", {})
    c.check("parent.class", session.get("class") == "plan_simple" and session.get("policy", {}).get("completion") == "explicit_report" and bool(cfg.get("channels", {}).get("inputs")), session.get("class"))
    c.check("parent.workspace_read_only", (cfg.get("workspace_binding") or {}).get("access") == "read_only", cfg.get("workspace_binding"))
    kinds = [e.get("t") for e in wl]
    started, ended = kinds.count("turn_started"), kinds.count("turn_ended")
    c.check("turn.all_closed", started >= 1 and started == ended == state.get("turn_seq"), {"started": started, "ended": ended, "turn_seq": state.get("turn_seq"), "turns_completed": state.get("turns_completed")})
    c.check("protocol.no_behavior_steps", "step" not in kinds, kinds.count("step"))

    # Every call states its intent; a call without one was not executed.
    results = {e.get("call_id"): str(e.get("result", "")) for e in wl if e.get("t") == "action_result"}
    calls = tool_calls(wl)
    missing, refused = [], 0
    for (_, tc) in calls:
        intent = (tc.get("args") or {}).get("intent")
        if isinstance(intent, str) and intent.strip():
            continue
        if "this call was not executed" in results.get(tc.get("call_id"), ""):
            refused += 1
        else:
            missing.append(tc.get("call_id"))
    c.check("intent.every_call", bool(calls) and not missing, {"calls": len(calls), "refused_without_intent": refused, "missing": missing[:5]})
    write_json(case / "parent" / "intents.json", [{"call_id": tc.get("call_id"), "tool": tc.get("tool"), "intent": (tc.get("args") or {}).get("intent"), "command": (tc.get("args") or {}).get("command")} for (_, tc) in calls])
    commands = [str((tc.get("args") or {}).get("command", "")) for (_, tc) in calls if tc.get("tool") == "shell"]
    gone = [x for x in commands if re.search(r"todo dispatch|todo review|agent-session wait", x)]
    c.check("plan.no_dispatch_wait_review", not gone, gone[:4])
    c.check("plan.no_polling", not [x for x in commands if re.search(r"\bsleep\b", x)], [x for x in commands if "sleep" in x][:3])
    writes = [x for x in commands if re.search(r"(^|[;&|]\s*)(cat\s*>|tee\s|cp\s|mv\s|python3?\s.*(open|write))", x) and "agent-session" not in x]
    c.check("plan.read_only", not writes, writes[:3])

    # The whole list in the first Turn.
    first_end = next((e.get("at_ms") for e in wl if e.get("t") == "turn_ended"), None)
    in_first = [t["todo_id"] for t in todos if first_end and t.get("created_at_ms", 1 << 62) <= first_end]
    c.check("plan.whole_list_first_turn", len(in_first) >= (4 if variant != "failure" else 3), in_first)

    explores = [t for t in todos if t.get("mode") == "explore"]
    serial = [t for t in todos if t.get("mode") == "serial" and t.get("attempts")]
    c.check("todo.explorations", len(explores) >= 2, [t["todo_id"] for t in explores])
    all_children = [a["child_sid"] for t in todos for a in t.get("attempts", []) if a.get("child_sid")]
    c.check("todo.distinct_children", len(set(all_children)) == len(all_children), all_children)
    c.check("serial.behavior", all(a.get("behavior") == "work_simple" for t in serial for a in t["attempts"]), [(t["todo_id"], [a.get("behavior") for a in t["attempts"]]) for t in serial])
    windows = {}
    for t in todos:
        for a in t.get("attempts", []):
            if a.get("child_sid"):
                windows[a["child_sid"]] = child_window(case / "children" / a["child_sid"] / "session")
    ex = [windows[a["child_sid"]] for t in explores for a in t["attempts"][:1] if a.get("child_sid") in windows][:2]
    overlap = len(ex) == 2 and all(w["first_tool_ms"] and w["end_ms"] for w in ex) and max(w["first_tool_ms"] for w in ex) < min(w["end_ms"] for w in ex)
    c.check("parallel.overlap", overlap, ex)

    # The host starts a Todo as soon as its prerequisites completed: before
    # the plan's next input batch after the last of them ended.
    # Input batches: the one that opens a Turn is its `turn_started`.
    batch_entries = [e for e in wl if e.get("t") in ("turn_started", "input_batch") and e.get("at_ms")]
    batches = sorted(e["at_ms"] for e in batch_entries)
    late = []
    for t in todos:
        deps = [by_id[d] for d in t.get("after", []) if d in by_id]
        if not deps or not t.get("attempts"):
            continue
        ends = [a.get("ended_at_ms") for d in deps for a in d.get("attempts", [])[-1:] if a.get("status") == "completed"]
        if len(ends) != len(deps):
            continue
        dep_end = max(ends)
        if t.get("created_at_ms", 0) > dep_end:
            continue  # added after its prerequisites ended: it starts when added
        a0 = next((a for a in t["attempts"] if a["started_at_ms"] >= dep_end - 5), None)
        nxt = next((b for b in batches if b >= dep_end), None)
        if a0 is None or (nxt is not None and a0["started_at_ms"] > nxt and not t.get("updates")):
            late.append({"todo": t["todo_id"], "deps_end": dep_end, "started": a0 and a0["started_at_ms"], "next_batch": nxt})
    c.check("schedule.before_plan_infers", not late, late[:4])
    verify_todos = [t for t in todos if t.get("after") and (MARKERS["C"] in t.get("content", "") or re.search(r"verif|验证|复核", t.get("title", "") + t.get("content", ""), re.I))]
    c.check("todo.verification", bool(verify_todos), [t["todo_id"] for t in verify_todos])
    c.check("workspace.explorations_without", all(not a.get("workspace_id") for t in explores for a in t["attempts"]), [(t["todo_id"], [a.get("workspace_id") for a in t["attempts"]]) for t in explores])

    review = (case / "workspace" / "review.md").read_text(encoding="utf-8") if (case / "workspace" / "review.md").exists() else ""
    decisions = (case / "workspace" / "decisions.md").read_text(encoding="utf-8") if (case / "workspace" / "decisions.md").exists() else ""
    if variant != "failure":
        writers = [t["todo_id"] for t in serial if any(a.get("workspace_id") == ws_id for a in t["attempts"])]
        c.check("workspace.writer_inherited", bool(writers), writers)
        c.check("deliverable.review_md", bool(review), len(review))
        c.check("deliverable.decisions_md", bool(decisions), len(decisions))
        c.check("review.defect_inventory", "inventory.py" in review, "inventory.py")
        c.check("review.defect_idempotency", "idempotency.py" in review and bool(re.search(r"tenant|租户", review)), "idempotency.py + tenant")
        # The investigations the review rests on (a later check of the
        # deliverable cannot be named in it).
        untraced = [t["todo_id"] for t in explores if (MARKERS["A"] in t.get("content", "") or MARKERS["B"] in t.get("content", "")) and t["todo_id"] not in review]
        c.check("review.traceable", not untraced, untraced)
        c.check("parent.succeeded", state.get("outcome") == "succeeded", state.get("outcome"))
        outs = {k: "".join(outputs_of(case / "children" / a["child_sid"] / "session") for t in todos if MARKERS[k] in t.get("content", "") for a in t.get("attempts", []) if a.get("child_sid")) for k in MARKERS}
        for k in MARKERS:
            c.check(f"marker.{k}.in_output", re.search(MARKERS[k] + r" \S", outs[k]) is not None, MARKERS[k])
    else:
        final = (state.get("final_report") or {}).get("report", "")
        blocked = re.compile(r"ModuleNotFoundError|missing|not found|缺少|缺失|不存在", re.I)
        c.check("failure.reported_honestly", bool(blocked.search(final + review)), final[:300])
        c.check("failure.verification_todo", bool(verify_todos), [t["todo_id"] for t in verify_todos])

    if variant == "user-update":
        posted = read_json(case / "user_update.json", {}) or {}
        c.check("update.posted", posted.get("exit") == 0, posted)
        b = [t for t in todos if MARKERS["B"] in t.get("content", "")]
        updated = [t["todo_id"] for t in b if t.get("updates")]
        c.check("update.todo_updated", bool(updated), [(t["todo_id"], t.get("updates")) for t in b])
        delivered = []
        for t in b:
            for a in t.get("attempts", []):
                cwl = read_jsonl(case / "children" / a["child_sid"] / "session" / ".opendan_agent_session" / "worklog.jsonl")
                if any("#updated@" in (i.get("key") or "") for e in cwl if e.get("t") == "input_batch" for i in e.get("inputs", [])):
                    delivered.append(a["child_sid"])
        retried = any(len(t.get("attempts", [])) > 1 for t in b)
        c.check("update.reached_work_session", bool(delivered) or retried, {"delivered": delivered, "retried": retried})
        b_out = "".join(outputs_of(case / "children" / a["child_sid"] / "session") + json.dumps(read_json(case / "children" / a["child_sid"] / "status.json", {}), ensure_ascii=False) for t in b for a in t.get("attempts", []))
        c.check("update.followed", UPDATE_MARKER in b_out, UPDATE_MARKER)

    # Turns: the list, what came back, the user's messages, ticks, the end.
    inputs = [i for e in batch_entries for i in e.get("inputs", [])]
    heard = sum(1 for i in inputs if (i.get("src") or "").startswith("_child"))
    msgs = sum(1 for i in inputs if i.get("src") == "q")
    ticks = sum(1 for i in inputs if (i.get("src") or "").startswith("_timer"))
    c.check("turn.bounded", (state.get("turn_seq") or 0) <= len(all_children) + msgs + ticks + 3, {"turns": state.get("turn_seq"), "children": len(all_children), "heard": heard, "msgs": msgs, "ticks": ticks})
    summary = [e.get("detail", {}).get("summary") for e in wl if e.get("t") == "control_applied" and e.get("command") == "todo_summary"]
    c.check("todo.summary_recorded", bool(summary), summary)
    backgrounds = [e.get("content", "") for e in wl if e.get("t") == "user_message" and e.get("content", "").startswith("<<background_context>>")]
    c.check("background.injected", len(backgrounds) >= 2, len(backgrounds))
    write_json(case / "parent" / "backgrounds.json", backgrounds)

    errors = []
    for d in [pdir] + [case / "children" / s / "session" for s in all_children]:
        for e in read_jsonl(d / ".opendan_agent_session" / "worklog.jsonl"):
            text = json.dumps(e, ensure_ascii=False)
            if re.search(r"\b403\b|refus|forbidden|invalid_prompt", text, re.I) and e.get("t") in ("outcome", "turn_ended", "error"):
                errors.append({"session": d.name[:20], "t": e.get("t"), "text": text[:300]})
    stderr = (case / "drive.stderr.log").read_text(encoding="utf-8", errors="replace") if (case / "drive.stderr.log").exists() else ""
    errors += [{"stderr": l[:300]} for l in stderr.splitlines() if re.search(r"\b403\b|invalid_prompt", l)]
    c.check("provider.refusals_recorded", True, errors[:10] or "none")

    total = {"input_tokens": 0, "output_tokens": 0, "total_tokens": 0}
    for d in [pdir] + [case / "children" / s / "session" for s in all_children]:
        st = read_json(d / ".opendan_agent_session" / "static.json", {}) or {}
        for k in total:
            total[k] += st.get(k, 0)
    passed = all(i["ok"] for i in c.items)
    write_json(case / "verification.json", {
        "session_id": sid, "mode": "plan_simple", "variant": variant, "passed": passed,
        "checks": c.items, "usage": total, "run": runinfo,
        "models": meta["provider"], "fixture_sha256": meta["fixture"]["sha256"],
    })
    log(f"verification {'PASSED' if passed else 'FAILED'}: {sum(i['ok'] for i in c.items)}/{len(c.items)} (tokens {total['total_tokens']})")
    return 0 if passed else 1

# ---------------------------------------------------------------------------


def default_case(variant: str) -> Path:
    root = Path(os.environ.get("PLAN_DO_CASE_ROOT", str(Path(os.environ.get("TMPDIR", "/tmp")) / "plan_do_todo_dv")))
    return root / f"{time.strftime('%Y%m%dT%H%M%S')}-{variant}"


def main() -> int:
    if len(sys.argv) == 1:
        if os.environ.get("PLAN_DO_DV") != "1":
            print("[plan_do_dv] NOT RUN: real-model DV; set PLAN_DO_DV=1 (and the provider, see README.md)")
            return EXIT_NOT_RUN
        sys.argv.append("all")
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("phase", choices=["all", "prepare", "run", "collect", "verify"])
    p.add_argument("--case", type=Path)
    p.add_argument("--variant", choices=["normal", "failure", "user-update"], default="normal")
    a = p.parse_args()
    case = (a.case or (default_case(f"plan_simple-{a.variant}") if a.phase in ("all", "prepare") else None))
    if case is None:
        fail("--case <dir> is required for this phase")
    case = case.resolve()
    if a.phase in ("all", "prepare"):
        prepare(case, a.variant)
    if a.phase in ("all", "run"):
        run(case)
    if a.phase == "collect":
        collect(case)
    if a.phase in ("all", "verify"):
        return verify(case)
    return 0


if __name__ == "__main__":
    sys.exit(main())
