#!/usr/bin/env python3
"""Real-model DV of the TodoList-driven PLAN / DO and of plan_simple (see
README.md).

    python3 test/plan_do_todo_dv/main.py all [--mode plan|plan_simple] [--variant normal|failure] [--case DIR]
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


def prepare(case: Path, variant: str, mode: str = "plan") -> None:
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
    task_file = "task_simple.md" if mode == "plan_simple" else "task.md"
    task = (HERE / task_file).read_text(encoding="utf-8").replace("{{FIXTURE}}", str(fixture))
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
        "mode": mode,
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
    log(f"prepared {case} ({mode}, {variant}); fixture sha256 {combined[:12]}, xagent {exe}")


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


def mode_of(case: Path) -> str:
    return (read_json(case / "config" / "case.json") or {}).get("mode", "plan")


def run(case: Path) -> int:
    env = case_env(case)
    exe = xagent(case)
    task = (case / "task.txt").read_text(encoding="utf-8")
    if mode_of(case) == "plan_simple":
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
        create = [exe, "new", "--class", "plan_simple", "--workspace", ws]
    else:
        create = [exe, "new", "--class", "work", "--behavior", "plan"]
    with open(case / "create.json", "w") as out:
        r = subprocess.run(
            [*create, "--objective", task,
             "--parent", str(case / "sessions"), "--llm-context", f"@{case / 'llm-context.json'}", "--no-run"],
            env=env, stdout=out, stderr=subprocess.PIPE, text=True,
        )
    if r.returncode != 0:
        fail(f"xagent new failed ({r.returncode}): {r.stderr}")
    sid = read_json(case / "create.json")["session_id"]
    log(f"session {sid}: driving (timeout {RUN_TIMEOUT_SECS}s)")
    started = time.time()
    with open(case / "drive.json", "w") as out, open(case / "drive.stderr.log", "w") as err:
        try:
            r = subprocess.run(
                [exe, "run", sid, "--until", "finished", "--max-children", "4", "--max-wait", str(RUN_TIMEOUT_SECS)],
                env=env, stdout=out, stderr=err, timeout=RUN_TIMEOUT_SECS + 300,
            )
            code = r.returncode
        except subprocess.TimeoutExpired:
            code = -1
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
    # Every decision boundary's view of the list, as the PLAN got it.
    views = []
    for e in read_jsonl(pdir / ".opendan_agent_session" / "worklog.jsonl"):
        if e.get("t") == "user_message" and "<todo_list" in e.get("content", ""):
            m = re.search(r"<todo_list rev=\"\d+\">.*?</todo_list>", e["content"], re.S)
            views.append({"seq": e["seq"], "run_id": e.get("run_id"), "view": m.group(0) if m else None})
    write_json(case / "parent" / "views.json", views)
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
            elif a.get("run_id"):
                run_id = a["run_id"]
                d = case / "serial" / run_id
                entries = [e for e in read_jsonl(pdir / ".opendan_agent_session" / "worklog.jsonl") if e.get("run_id") == run_id]
                sub_task = [e["content"] for e in entries if e.get("t") == "user_message" and "<sub_task" in e.get("content", "")]
                write_json(d / "sub_task.json", sub_task)
                write_json(d / "process_done.json", [e for e in entries if e.get("t") == "outcome"])
                write_json(d / "binding.json", {"todo_id": t["todo_id"], "attempt": a})
                rep = a.get("report") or {}
                sub = pdir / ".opendan_agent_session" / "reports" / rep.get("report_id", "-") / "submission.json"
                if sub.exists():
                    shutil.copy2(sub, d / "report.json")
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
            events.append({"at_ms": a.get("started_at_ms"), "who": t["todo_id"], "what": f"attempt {a['attempt']} {a['mode']} started ({a.get('child_sid') or a.get('caller')})"})
            if a.get("ended_at_ms"):
                events.append({"at_ms": a["ended_at_ms"], "who": t["todo_id"], "what": f"attempt {a['attempt']} {a['status']}: {a.get('summary')}"})
        for r in t.get("reviews", []):
            events.append({"at_ms": r["at_ms"], "who": t["todo_id"], "what": f"review {r['verdict']}: {r['reason'][:120]}"})
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


def texts_of(entries: list[dict]) -> list[tuple[int, str, str]]:
    """(seq, kind, text) of what entered the PLAN's context."""
    out = []
    for e in entries:
        t = e.get("t")
        if t == "user_message":
            out.append((e["seq"], t, e.get("content", "")))
        elif t == "action_result":
            out.append((e["seq"], t, str(e.get("result", ""))))
    return out


def commands_by_call(entries: list[dict]) -> dict:
    out = {}
    for e in entries:
        if e.get("t") in ("step", "assistant_message"):
            for a in e.get("actions", []) or e.get("tool_calls", []) or []:
                args = a.get("args") or {}
                out[(e.get("run_id"), a.get("call_id"))] = str(args.get("command") or args)
    return out


def verify(case: Path) -> int:
    if mode_of(case) == "plan_simple":
        return verify_simple(case)
    c = Checks()
    meta = read_json(case / "config" / "case.json")
    variant = meta["variant"]
    runinfo = read_json(case / "run.json", {})
    sid = runinfo.get("session_id")
    pdir = case / "parent" / "session"
    state = read_json(pdir / ".opendan_agent_session" / "state.json", {})
    wl = read_jsonl(pdir / ".opendan_agent_session" / "worklog.jsonl")
    todos = (read_json(case / "parent" / "todos.json", {}) or {}).get("todos", [])
    c.check("run.exit", runinfo.get("exit_code") == 0 or variant == "failure", runinfo)
    c.check("parent.finished", state.get("run_state") == "finished", state.get("one_line_status"))

    explores = [t for t in todos if t.get("mode") == "explore"]
    serial = [t for t in todos if t.get("mode") == "serial" and t.get("attempts")]
    c.check("todo.explorations", len(explores) >= 2, [t["todo_id"] for t in explores])
    sids = [a["child_sid"] for t in explores for a in t["attempts"] if a.get("child_sid")]
    c.check("todo.distinct_children", len(set(sids)) == len(sids) and len(sids) >= 2, sids)

    # Which exploration carries which acceptance marker (only the Todo body had it).
    by_marker = {}
    for t in explores:
        for a in t["attempts"]:
            cdir = case / "children" / a["child_sid"] / "session"
            cwl = read_jsonl(cdir / ".opendan_agent_session" / "worklog.jsonl")
            first = next((e.get("content", "") for e in cwl if e.get("t") == "user_message"), "")
            cfg = read_json(cdir / ".opendan_agent_session" / "session_config.json", {}) or {}
            seen = first + cfg.get("session", {}).get("objective", "")
            for k in ("A", "B"):
                if MARKERS[k] in seen:
                    by_marker.setdefault(k, []).append((t, a, cdir))
    for k in ("A", "B"):
        c.check(f"marker.{k}.in_child_input", k in by_marker, MARKERS[k])

    # Real work in every exploration, and real overlap.
    windows = {}
    for t in explores:
        for a in t["attempts"]:
            cdir = case / "children" / a["child_sid"] / "session"
            w = child_window(cdir)
            windows[a["child_sid"]] = w
            c.check(f"child.{a['child_sid'][:12]}.real", w["rounds"] >= 1 and w["tool_calls"] >= 1, w)
    first_two = [windows[s] for s in sids[:2] if s in windows]
    if len(first_two) == 2 and all(w["first_tool_ms"] and w["end_ms"] for w in first_two):
        overlap = max(w["first_tool_ms"] for w in first_two) < min(w["end_ms"] for w in first_two)
    else:
        overlap = False
    c.check("parallel.overlap", overlap, first_two)

    # The PLAN waited by not deciding and was woken by the ends.
    waits = [e for e in wl if e.get("t") == "outcome" and e.get("kind") == "wait"]
    c.check("plan.waited", len(waits) >= 1, len(waits))
    woken = [e for e in wl if e.get("t") in ("input_batch", "turn_started") and any(i.get("src") == "_child" for i in e.get("inputs", []))]
    c.check("plan.woken_by_child_end", len(woken) >= 1, len(woken))
    cmds = commands_by_call(wl)
    polling = [v for v in cmds.values() if re.search(r"\bsleep\b|agent-session wait\b", v)]
    c.check("plan.no_polling", not polling, polling[:3])

    # Review before building on a result; the serial Todo after the reviews.
    reviewed = [r for t in explores for r in t.get("reviews", [])]
    c.check("review.explorations", len(reviewed) >= 2, [(r["attempt"], r["verdict"]) for r in reviewed])
    if variant == "normal":
        c.check("todo.serial", len(serial) >= 1, [t["todo_id"] for t in serial])
        accepted_at = [r["at_ms"] for t in explores for r in t.get("reviews", []) if r["verdict"] == "accept"]
        carries_c = []
        for t in serial:
            a = t["attempts"][0]
            c.check(f"serial.{t['todo_id']}.after_reviews", accepted_at and a["started_at_ms"] >= max(accepted_at[:2]), (a["started_at_ms"], accepted_at))
            sub = read_json(case / "serial" / a.get("run_id", "-") / "sub_task.json", []) or []
            text = "\n".join(sub)
            header = f"Todo {t['todo_id']} (attempt {a['attempt']}, serial)"
            c.check(f"serial.{t['todo_id']}.sub_task_has_body", "<sub_task" in text and header in text and t["content"][:40] in text, text[:300])
            refs = any(s in text for s in sids) or any(x["todo_id"] in text for x in explores)
            c.check(f"serial.{t['todo_id']}.sub_task_has_reviewed_refs", refs, text[:300])
            if MARKERS["C"] in text:
                carries_c.append(t["todo_id"])
        # The verification Todo carried its Todo-only requirement to DO.
        c.check("marker.C.in_sub_task", bool(carries_c), carries_c)
        c.check("marker.C.in_output", re.search(MARKERS["C"] + r" \S", outputs_of(pdir)) is not None, MARKERS["C"])
        for k in ("A", "B"):
            outs = "".join(outputs_of(cdir) for (_, _, cdir) in by_marker.get(k, []))
            c.check(f"marker.{k}.in_output", re.search(MARKERS[k] + r" \S", outs) is not None, MARKERS[k])
        # Converged and accepted.
        open_todos = [t["todo_id"] for t in todos if t["status"] not in ("completed", "cancelled")]
        c.check("todo.converged", not open_todos, open_todos)
        c.check("parent.succeeded", state.get("outcome") == "succeeded", state.get("outcome"))
        review = (case / "review.md").read_text(encoding="utf-8") if (case / "review.md").exists() else ""
        decisions = (case / "decisions.md").read_text(encoding="utf-8") if (case / "decisions.md").exists() else ""
        c.check("deliverable.review_md", bool(review), len(review))
        c.check("deliverable.decisions_md", bool(decisions), len(decisions))
        c.check("review.defect_inventory", "inventory.py" in review, "inventory.py")
        c.check("review.defect_idempotency", "idempotency.py" in review and re.search(r"tenant|租户", review), "idempotency.py + tenant")
        # Every accepted result is traceable: its Todo and its execution.
        def accepted(t):
            r = t.get("reviews") or []
            return bool(r) and r[-1]["verdict"] == "accept"
        untraced = []
        # Findings come from the explorations and the verification Todo.
        findings = explores + [t for t in serial if t["todo_id"] in carries_c]
        for t in [x for x in findings if accepted(x)]:
            a = t["attempts"][-1]
            ref = a.get("child_sid") or a.get("run_id") or ""
            if t["todo_id"] not in review or (ref and ref not in review):
                untraced.append({"todo": t["todo_id"], "ref": ref})
        c.check("review.traceable", not untraced, untraced)
    else:
        # The failure variant: the branch without its input says so (no
        # fabricated result), and the PLAN either rejects / supplements it
        # or accepts it only as a blocker and reports the claim unverified.
        b = by_marker.get("B", [])
        final = (state.get("final_report") or {}).get("report", "")
        review = (case / "review.md").read_text(encoding="utf-8") if (case / "review.md").exists() else ""
        blocked = re.compile(r"ModuleNotFoundError|missing|not found|缺少|缺失|不存在", re.I)
        unverified = re.compile(r"无法(判断|验证|确认)|不能(判断|验证|声称)|未(能)?(证实|验证)|cannot (verify|judge|determine)|unverified", re.I)
        reports_b = []
        for (t, a, cdir) in b:
            cst = read_json(cdir / ".opendan_agent_session" / "state.json", {}) or {}
            reports_b.append((cst.get("final_report") or {}).get("report", ""))
        c.check("failure.blocker_reported_by_branch", any(blocked.search(r) for r in reports_b), [r[:160] for r in reports_b])
        verdicts = [r["verdict"] for (t, _, _) in b for r in t.get("reviews", [])]
        rejected_or_followed = any(v in ("reject", "need_evidence") for v in verdicts) or any(len(t["attempts"]) > 1 for (t, _, _) in b)
        honest = bool(blocked.search(final + review)) and bool(unverified.search(final + review))
        c.check("failure.handled", rejected_or_followed or honest, {"verdicts": verdicts, "final": final[:300]})
        c.check("failure.reported_honestly", honest, final[:300])

    # V12: the PLAN's context held no child tool output, and a child report's
    # body only after the PLAN asked for it. The PLAN's context is its own
    # runs; a serial Todo's DO context (a sub context) reads what its Todo
    # needs.
    plan_runs = {e.get("run_id") for e in wl if e.get("t") == "step" and e.get("behavior") == "plan"}
    plan_entries = [e for e in wl if e.get("run_id") in plan_runs]
    entered = texts_of(plan_entries)
    injected = [(seq, text) for (seq, kind, text) in entered if kind == "user_message"]
    leaks = []
    late = []
    reads = []
    for t in explores:
        for a in t["attempts"]:
            cdir = case / "children" / a["child_sid"] / "session"
            cstate = read_json(cdir / ".opendan_agent_session" / "state.json", {}) or {}
            report = (cstate.get("final_report") or {}).get("report", "")
            body_lines = [l.strip() for l in report.splitlines()[1:] if len(l.strip()) >= 30]
            outs = [l.strip() for l in outputs_of(cdir).splitlines() if len(l.strip()) >= 30]
            # What the host put into the PLAN's context: input batches (the
            # view, child-end events, sub context returns).
            # The one-line summaries are meant to be shown; paths of the
            # case itself appear in the host's own runtime block.
            summaries = [x.get("summary") or "" for x in t["attempts"]]
            def designed(line):
                return any(line in sm or sm in line for sm in summaries if sm) or (" " not in line and line.startswith(str(case)))
            for line in outs:
                if line not in report and not designed(line) and any(line in text for (_, text) in injected):
                    leaks.append(line[:80])
            for line in [l for l in body_lines if not designed(l)][:5]:
                if any(line in text for (_, text) in injected):
                    late.append({"todo": t["todo_id"], "line": line[:80], "injected": True})
                    continue
                first = next(((seq, kind) for (seq, kind, text) in entered if line in text), None)
                if first:
                    entry = next(e for e in plan_entries if e.get("seq") == first[0])
                    cmd = cmds.get((entry.get("run_id"), entry.get("call_id")), "")
                    reads.append({"todo": t["todo_id"], "seq": first[0], "cmd": cmd[:80],
                                  "on_demand": bool(re.search(r"todo show .*--report|read-session", cmd, re.S))})
    c.check("view.no_child_tool_output_injected", not leaks, leaks[:5])
    c.check("view.report_not_injected", not late, late[:5])
    bad_reads = [r for r in reads if not r["on_demand"]]
    c.check("view.reports_read_on_demand", bool(reads) and not bad_reads, bad_reads[:6] or reads[:4])
    views = read_json(case / "parent" / "views.json", []) or []
    c.check("view.assembled", len(views) >= 2, len(views))
    bad = [v for v in views if v.get("view") and re.search(r"<(worklog|action_result|step)", v["view"])]
    c.check("view.compact_fields_only", not bad, bad[:2])

    total = {"input_tokens": 0, "output_tokens": 0, "total_tokens": 0}
    dirs = [pdir] + [case / "children" / s / "session" for s in sids]
    for d in dirs:
        st = read_json(d / ".opendan_agent_session" / "static.json", {}) or {}
        for k in total:
            total[k] += st.get(k, 0)
    passed = all(i["ok"] for i in c.items)
    write_json(case / "verification.json", {
        "session_id": sid, "variant": variant, "passed": passed,
        "checks": c.items, "usage": total, "run": runinfo,
        "models": meta["provider"], "fixture_sha256": meta["fixture"]["sha256"],
    })
    log(f"verification {'PASSED' if passed else 'FAILED'}: {sum(i['ok'] for i in c.items)}/{len(c.items)} (tokens {total['total_tokens']})")
    return 0 if passed else 1


def tool_calls(entries: list[dict]) -> list[tuple[dict, dict]]:
    """(assistant_message entry, tool call) of a function call context."""
    return [(e, tc) for e in entries if e.get("t") == "assistant_message" for tc in e.get("tool_calls", []) or []]


def verify_simple(case: Path) -> int:
    """plan_simple: one Turn of native calls with intents; every Todo in a
    sub session of its own; waits as calls; reviews before building on a
    result; the background after tool results; the deliverables in the
    workspace through the Todo that inherited it."""
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
    ws_id = (read_json(case / "workspace.json", {}) or {}).get("workspace_id")
    c.check("run.exit", runinfo.get("exit_code") == 0 or variant == "failure", runinfo)
    c.check("parent.finished", state.get("run_state") == "finished", state.get("one_line_status"))
    c.check("parent.class", cfg.get("session", {}).get("class") == "plan_simple" and cfg.get("session", {}).get("policy", {}).get("completion") == "explicit_report", cfg.get("session", {}).get("class"))
    kinds = [e.get("t") for e in wl]
    c.check("turn.single", kinds.count("turn_started") == 1 and kinds.count("turn_ended") == 1 and state.get("turn_seq") == 1, {"started": kinds.count("turn_started"), "ended": kinds.count("turn_ended"), "turn_seq": state.get("turn_seq")})
    c.check("protocol.no_behavior_steps", "step" not in kinds, kinds.count("step"))
    behaviors = (cfg.get("extensions", {}).get("opendan", {}) or {}).get("behaviors", {}) or {}
    c.check("protocol.no_do", "do" not in behaviors and behaviors.get("plan_simple", {}).get("call_intent") is True, sorted(behaviors))

    # Every call states its intent; a call without one was not executed.
    results = {e.get("call_id"): str(e.get("result", "")) for e in wl if e.get("t") == "action_result"}
    calls = tool_calls(wl)
    missing = []
    refused = 0
    for (_, tc) in calls:
        intent = (tc.get("args") or {}).get("intent")
        if isinstance(intent, str) and intent.strip():
            continue
        if "this call was not executed" in results.get(tc.get("call_id"), ""):
            refused += 1
        else:
            missing.append(tc.get("call_id"))
    c.check("intent.every_call", bool(calls) and not missing, {"calls": len(calls), "refused_without_intent": refused, "missing": missing[:5]})
    intents = [(tc.get("args") or {}).get("intent") for (_, tc) in calls]
    write_json(case / "parent" / "intents.json", [{"call_id": tc.get("call_id"), "tool": tc.get("tool"), "intent": (tc.get("args") or {}).get("intent"), "command": (tc.get("args") or {}).get("command")} for (_, tc) in calls])
    c.check("intent.not_reasoning_dump", all(len(i) <= 600 for i in intents if isinstance(i, str)), max((len(i) for i in intents if isinstance(i, str)), default=0))

    commands = [str((tc.get("args") or {}).get("command", "")) for (_, tc) in calls if tc.get("tool") == "shell"]
    waits = [x for x in commands if "agent-session wait" in x]
    standalone = re.compile(r"^\s*agent-session wait \S+(\s+--wait-ms \d+)?\s*$")
    c.check("wait.as_calls", bool(waits) and all(standalone.match(x) for x in waits), waits[:6])
    c.check("plan.no_polling", not [x for x in commands if re.search(r"\bsleep\b", x)], [x for x in commands if "sleep" in x][:3])

    explores = [t for t in todos if t.get("mode") == "explore"]
    serial = [t for t in todos if t.get("mode") == "serial" and t.get("attempts")]
    c.check("todo.explorations", len(explores) >= 2, [t["todo_id"] for t in explores])
    c.check("todo.serial_sessions", len(serial) >= (2 if variant == "normal" else 0) and all(a.get("child_sid") and not a.get("run_id") for t in serial for a in t["attempts"]), [(t["todo_id"], [a.get("child_sid", "")[:16] for a in t["attempts"]]) for t in serial])
    all_children = [a["child_sid"] for t in todos for a in t.get("attempts", []) if a.get("child_sid")]
    c.check("todo.distinct_children", len(set(all_children)) == len(all_children), all_children)
    c.check("serial.behavior", all(a.get("behavior") == "work_simple" for t in serial for a in t["attempts"]), [(t["todo_id"], [a.get("behavior") for a in t["attempts"]]) for t in serial])

    # Parallel explorations; serial Todos one at a time, after their
    # prerequisites were accepted.
    windows = {}
    for t in todos:
        for a in t.get("attempts", []):
            if a.get("child_sid"):
                w = child_window(case / "children" / a["child_sid"] / "session")
                windows[a["child_sid"]] = w
                c.check(f"child.{a['child_sid'][:12]}.real", w["rounds"] >= 1 and w["tool_calls"] >= 1, w)
    ex = [windows[a["child_sid"]] for t in explores for a in t["attempts"][:1] if a.get("child_sid") in windows][:2]
    overlap = len(ex) == 2 and all(w["first_tool_ms"] and w["end_ms"] for w in ex) and max(w["first_tool_ms"] for w in ex) < min(w["end_ms"] for w in ex)
    c.check("parallel.overlap", overlap, ex)
    spans = sorted((a["started_at_ms"], a.get("ended_at_ms") or 1 << 62, t["todo_id"]) for t in serial for a in t["attempts"])
    c.check("serial.one_at_a_time", all(spans[i][0] >= spans[i - 1][1] for i in range(1, len(spans))), spans)
    by_id = {t["todo_id"]: t for t in todos}
    early = []
    for t in serial:
        for a in t["attempts"]:
            for d in t.get("after", []):
                acc = [r["at_ms"] for r in by_id.get(d, {}).get("reviews", []) if r["verdict"] == "accept"]
                if not acc or a["started_at_ms"] < min(acc):
                    early.append((t["todo_id"], d))
    accepts = sorted(r["at_ms"] for t in explores for r in t.get("reviews", []) if r["verdict"] == "accept")
    first_serial = min((a["started_at_ms"] for t in serial for a in t["attempts"]), default=None)
    ordered = variant != "normal" or (len(accepts) >= 2 and first_serial is not None and first_serial >= accepts[1])
    c.check("serial.after_accepted_prerequisites", not early and ordered, {"early": early, "accepts": accepts[:4], "first_serial": first_serial})

    # Workspace assignment: explorations read without the workspace; the
    # Todo that writes the deliverables inherited it.
    c.check("workspace.explorations_without", all(not a.get("workspace_id") for t in explores for a in t["attempts"]), [(t["todo_id"], [a.get("workspace_id") for a in t["attempts"]]) for t in explores])
    writers = [t["todo_id"] for t in serial if any(a.get("workspace_id") == ws_id for a in t["attempts"])]
    review = (case / "workspace" / "review.md").read_text(encoding="utf-8") if (case / "workspace" / "review.md").exists() else ""
    decisions = (case / "workspace" / "decisions.md").read_text(encoding="utf-8") if (case / "workspace" / "decisions.md").exists() else ""
    if variant == "normal":
        c.check("workspace.writer_inherited", bool(writers), writers)
        c.check("deliverable.review_md", bool(review), len(review))
        c.check("deliverable.decisions_md", bool(decisions), len(decisions))
        c.check("review.defect_inventory", "inventory.py" in review, "inventory.py")
        c.check("review.defect_idempotency", "idempotency.py" in review and re.search(r"tenant|租户", review), "idempotency.py + tenant")
        untraced = [t["todo_id"] for t in explores + serial[:1] if t["todo_id"] not in review or (t["attempts"][-1].get("child_sid") or "") not in review]
        c.check("review.traceable", not untraced, untraced)
        open_todos = [t["todo_id"] for t in todos if t["status"] not in ("completed", "cancelled")]
        c.check("todo.converged", not open_todos, open_todos)
        c.check("parent.succeeded", state.get("outcome") == "succeeded", state.get("outcome"))
        outs_serial = "".join(outputs_of(case / "children" / a["child_sid"] / "session") for t in serial for a in t["attempts"])
        c.check("marker.C.in_output", re.search(MARKERS["C"] + r" \S", outs_serial) is not None, MARKERS["C"])
    for k in ("A", "B"):
        hit = []
        for t in explores:
            for a in t["attempts"]:
                cdir = case / "children" / a["child_sid"] / "session"
                ccfg = read_json(cdir / ".opendan_agent_session" / "session_config.json", {}) or {}
                if MARKERS[k] in ccfg.get("session", {}).get("objective", ""):
                    hit.append(cdir)
        c.check(f"marker.{k}.in_child_input", bool(hit), MARKERS[k])
        if variant == "normal" or k == "A":
            c.check(f"marker.{k}.in_output", any(re.search(MARKERS[k] + r" \S", outputs_of(d)) for d in hit), MARKERS[k])

    # Every latest result was reviewed; reports were read before they were
    # judged; the background never carried a child's output.
    unreviewed = [t["todo_id"] for t in todos if t.get("attempts") and t["status"] != "cancelled" and not any(r["attempt"] == t["attempts"][-1]["attempt"] for r in t.get("reviews", []))]
    c.check("review.every_result", not unreviewed, unreviewed)
    shows = [i for i, x in enumerate(commands) if re.search(r"todo show \S+ .*--report|read-session", x)]
    c.check("review.reports_read", len(shows) >= len([t for t in todos if t.get("reviews")]), {"reads": len(shows)})
    backgrounds = [e.get("content", "") for e in wl if e.get("t") == "user_message" and e.get("content", "").startswith("<<background_context>>")]
    c.check("background.injected", len(backgrounds) >= 2, len(backgrounds))
    leaks = []
    # A Todo's one-line result (its report's first line) is meant to be
    # shown; anything else of a child's output is not.
    summaries = [a.get("summary") for t in todos for a in t.get("attempts", []) if a.get("summary")]
    for t in todos:
        for a in t.get("attempts", []):
            if not a.get("child_sid"):
                continue
            for line in [l.strip() for l in outputs_of(case / "children" / a["child_sid"] / "session").splitlines() if len(l.strip()) >= 30]:
                if any(line in sm for sm in summaries):
                    continue
                if any(line in bg for bg in backgrounds):
                    leaks.append(line[:80])
    c.check("background.no_child_output", not leaks, leaks[:5])
    write_json(case / "parent" / "backgrounds.json", backgrounds)
    if variant == "failure":
        final = (state.get("final_report") or {}).get("report", "")
        blocked = re.compile(r"ModuleNotFoundError|missing|not found|缺少|缺失|不存在", re.I)
        b = [t for t in explores if MARKERS["B"] in t.get("content", "")]
        verdicts = [r["verdict"] for t in b for r in t.get("reviews", [])]
        handled = any(v in ("reject", "need_evidence") for v in verdicts) or any(len(t["attempts"]) > 1 for t in b) or bool(blocked.search(final + review))
        c.check("failure.handled", handled, {"verdicts": verdicts, "final": final[:300]})
        c.check("failure.reported_honestly", bool(blocked.search(final + review)), final[:300])

    # Provider refusals (the reason for this mode): recorded, not judged
    # from a status code alone.
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
    p.add_argument("--variant", choices=["normal", "failure"], default="normal")
    p.add_argument("--mode", choices=["plan", "plan_simple"], default="plan")
    a = p.parse_args()
    case = (a.case or (default_case(f"{a.mode}-{a.variant}") if a.phase in ("all", "prepare") else None))
    if case is None:
        fail("--case <dir> is required for this phase")
    case = case.resolve()
    if a.phase in ("all", "prepare"):
        prepare(case, a.variant, a.mode)
    if a.phase in ("all", "run"):
        run(case)
    if a.phase == "collect":
        collect(case)
    if a.phase in ("all", "verify"):
        return verify(case)
    return 0


if __name__ == "__main__":
    sys.exit(main())
