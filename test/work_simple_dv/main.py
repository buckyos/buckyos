#!/usr/bin/env python3
"""work_simple_dv: the asynchronous simple Work Session with a real model.

notepads/done/opendan-simple-worksession-async-todo.md §6: a `work_simple`
session of the Jarvis package builds two slow crates in parallel. Each build
outlives the 25 s in-call wait and becomes a task of the runner process; the
session closes a Turn with a progress reply, a task's end (`<<task_result>>`)
or a tick every `tick_secs` opens the next Turn, the new build output is shown
in `<<background_context>>`, and only `report(is_end=true)` ends it. Nothing
waits inside the loop: the model never calls `wait_task` / `task wait`.

Variants:
- normal: one `xagent run` drives the session to its end;
- restart: the driving process is killed (SIGKILL) once the first Turn has
  closed while both builds run; a new `xagent run` picks the session up —
  a build that ended meanwhile is read from its execution directory, one
  still running comes back as `unknown` (simple Work Session TODO Q1).

    cd buckyos
    python3 test/work_simple_dv/main.py all --variant normal|restart

The model is real (an OpenAI compatible endpoint, default the local
CLIProxyAPI); a missing credential or endpoint fails (exit 2), nothing falls
back to a mock. Evidence lands in the case directory.
"""

import argparse
import hashlib
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
JARVIS = REPO / "src" / "apps" / "jarvis_agent" / "agent"
CASE_ROOT = Path(os.environ.get("WORK_SIMPLE_DV_CASE_ROOT", "/home/lzc/project/work_simple_dv_runs"))
BASE_URL = os.environ.get("WORK_SIMPLE_DV_BASE_URL", "http://127.0.0.1:8317/v1")
MODEL = os.environ.get("WORK_SIMPLE_DV_MODEL", "gpt-6-sol")
KEYS_ENV = Path("/opt/cli-proxy-api/keys.env")
TIMEOUT_SECS = int(os.environ.get("WORK_SIMPLE_DV_TIMEOUT_SECS", "1500"))
AGENT_DID = "did:bns:jarvis.work-simple-dv"
WHO = "app:xagent-test@local"

OBJECTIVE = """In {fixture} there are two Rust crates, `alpha` and `beta`. Their builds are slow (their build scripts take one to two minutes).
Build both, in parallel — one shell call for each crate, started together:
  cargo build -vv --offline --manifest-path {fixture}/alpha/Cargo.toml --target-dir {case}/target/alpha
  cargo build -vv --offline --manifest-path {fixture}/beta/Cargo.toml --target-dir {case}/target/beta
Then report whether each build succeeded and how long each took."""


def log(msg: str) -> None:
    print(f"[work_simple_dv] {msg}", file=sys.stderr, flush=True)


def fail(msg: str, code: int = 2) -> None:
    log(f"FAIL: {msg}")
    sys.exit(code)


def write_json(p: Path, v) -> None:
    p.write_text(json.dumps(v, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def read_json(p: Path, default=None):
    try:
        return json.loads(p.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return default


def api_key() -> str:
    key = os.environ.get("WORK_SIMPLE_DV_API_KEY")
    if key:
        return key
    try:
        for line in KEYS_ENV.read_text().splitlines():
            if line.startswith("API_KEY="):
                return line.split("=", 1)[1].strip()
    except OSError:
        pass
    fail(f"no credential: set WORK_SIMPLE_DV_API_KEY (or make {KEYS_ENV} readable)")
    return ""


def build_xagent() -> str:
    log("building xagent")
    r = subprocess.run(
        ["cargo", "build", "--manifest-path", str(REPO / "src" / "Cargo.toml"), "-p", "libopendan",
         "--bin", "xagent", "--message-format=json-render-diagnostics"],
        capture_output=True, text=True,
    )
    if r.returncode != 0:
        fail(f"cargo build of xagent failed:\n{r.stderr[-2000:]}")
    for line in r.stdout.splitlines():
        try:
            m = json.loads(line)
        except ValueError:
            continue
        if m.get("reason") == "compiler-artifact" and m.get("target", {}).get("name") == "xagent" and m.get("executable"):
            return m["executable"]
    fail("cargo did not report the xagent executable")
    return ""


def prepare(case: Path, variant: str) -> None:
    if case.exists() and any(case.iterdir()):
        fail(f"{case} is not empty")
    exe = build_xagent()
    for d in ["agent", "sessions", "kmsg", "target"]:
        (case / d).mkdir(parents=True, exist_ok=True)
    agent = case / "agent"
    for name in ["agent.toml", "role.md", "self.md"]:
        if (JARVIS / name).exists():
            shutil.copy2(JARVIS / name, agent / name)
    shutil.copytree(JARVIS / "behaviors", agent / "behaviors")
    if (JARVIS / "cmd_skills").exists():
        shutil.copytree(JARVIS / "cmd_skills", agent / "cmd_skills")
    # The package's model aliases become the provider's model id.
    for f in (agent / "behaviors").glob("*.toml"):
        text = f.read_text(encoding="utf-8")
        new = re.sub(r'preferred = "llm\.[a-z]+"', f'preferred = "{MODEL}"', text)
        if new != text:
            f.write_text(new, encoding="utf-8")
    fixture = case / "fixture"
    shutil.copytree(HERE / "fixture", fixture)
    write_json(case / "llm-context.json", {
        "provider": {"type": "openai", "base_url": BASE_URL, "api_key_env": "WORK_SIMPLE_DV_API_KEY"},
        "model": MODEL,
        "max_tokens": 16384,
        "llm_timeout": 900,
        "timeout": TIMEOUT_SECS,
        "tools": {"enabled": True},
    })
    (case / "objective.txt").write_text(OBJECTIVE.format(fixture=fixture, case=case), encoding="utf-8")
    write_json(case / "case.json", {
        "variant": variant,
        "model": MODEL,
        "base_url": BASE_URL,
        "xagent": exe,
        "work_simple_sha256": hashlib.sha256((agent / "behaviors" / "work_simple.toml").read_bytes()).hexdigest(),
        "prepared_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    })
    log(f"prepared {case} ({variant})")


def env_of(case: Path) -> dict:
    env = dict(os.environ)
    env.update({
        "OPENDAN_AGENT_ROOT": str(case / "agent"),
        "OPENDAN_AGENT_DID": AGENT_DID,
        "LIBOPENDAN_WHO": WHO,
        "LIBOPENDAN_QUEUE_DIR": str(case / "kmsg"),
        "WORK_SIMPLE_DV_API_KEY": api_key(),
    })
    for k in ["OPENDAN_SESSION_ID", "LIBOPENDAN_FAULT"]:
        env.pop(k, None)
    return env


def session_dir(case: Path, sid: str) -> Path:
    return case / "sessions" / sid


def state_of(case: Path, sid: str) -> dict:
    return read_json(session_dir(case, sid) / ".opendan_agent_session" / "state.json", {}) or {}


def drive(case: Path, sid: str, name: str, stop_when=None) -> int:
    """`xagent run` until the session finishes; `stop_when(state)` kills it
    (SIGKILL, a crash) as soon as it holds."""
    case_cfg = read_json(case / "case.json")
    with open(case / f"{name}.json", "w") as out, open(case / f"{name}.stderr.log", "w") as err:
        p = subprocess.Popen(
            [case_cfg["xagent"], "run", sid, "--until", "finished", "--max-wait", str(TIMEOUT_SECS)],
            env=env_of(case), stdout=out, stderr=err, start_new_session=True,
        )
        started = time.time()
        while p.poll() is None:
            if time.time() - started > TIMEOUT_SECS + 120:
                p.kill()
                fail(f"{name}: timed out")
            if stop_when and stop_when(state_of(case, sid)):
                os.kill(p.pid, signal.SIGKILL)
                p.wait()
                log(f"{name}: killed the driving process at turn {state_of(case, sid).get('turns_completed')}")
                return -9
            time.sleep(1)
    log(f"{name}: exited {p.returncode} after {time.time() - started:.0f}s")
    return p.returncode


def run(case: Path) -> None:
    cfg = read_json(case / "case.json")
    with open(case / "create.json", "w") as out:
        r = subprocess.run(
            [cfg["xagent"], "new", "--class", "work", "--behavior", "work_simple",
             "--objective", (case / "objective.txt").read_text(encoding="utf-8"),
             "--parent", str(case / "sessions"), "--llm-context", f"@{case / 'llm-context.json'}", "--no-run"],
            env=env_of(case), stdout=out, stderr=subprocess.PIPE, text=True,
        )
    if r.returncode != 0:
        fail(f"xagent new failed: {r.stderr}")
    sid = read_json(case / "create.json")["session_id"]
    log(f"session {sid}")
    started = time.time()
    if cfg["variant"] == "restart":
        # Kill once the first Turn closed while both builds run.
        code = drive(case, sid, "drive1", lambda st: st.get("turns_completed", 0) >= 1
                     and st.get("run_state") == "waiting" and len(st.get("watched_tasks", [])) >= 1)
        time.sleep(5)
        code = drive(case, sid, "drive2")
    else:
        code = drive(case, sid, "drive1")
    write_json(case / "run.json", {"session_id": sid, "exit_code": code, "seconds": round(time.time() - started, 1)})


def worklog(case: Path, sid: str) -> list:
    p = session_dir(case, sid) / ".opendan_agent_session" / "worklog.jsonl"
    out = []
    for line in p.read_text(encoding="utf-8").splitlines():
        try:
            out.append(json.loads(line))
        except ValueError:
            pass
    return out


def verify(case: Path) -> int:
    cfg = read_json(case / "case.json")
    sid = read_json(case / "run.json")["session_id"]
    st = state_of(case, sid)
    wl = worklog(case, sid)
    bodies = wl
    kind = lambda b: b.get("t")
    users = [b["content"] for b in bodies if kind(b) == "user_message"]
    calls = [c for b in bodies if kind(b) == "assistant_message" for c in b.get("tool_calls", [])]
    results = {b["call_id"]: b for b in bodies if kind(b) == "action_result"}
    outcomes = [b for b in bodies if kind(b) == "outcome"]
    turns = [b for b in bodies if kind(b) == "turn_ended"]
    task_results = [u for u in users if "<<task_result" in u]
    ticks = [u for u in users if "<tick timer=" in u]
    nudges = [u for u in users if "<runner_notice>" in u]
    backgrounds = [u for u in users if u.startswith("<<background_context>>")]
    shells = [c for c in calls if c["tool"] == "shell"]
    waits = [c for c in calls if c["tool"] == "wait_task"
             or "task wait" in json.dumps(c.get("args", {}))]
    became_tasks = [c for c in shells if "it continues as task" in results.get(c["call_id"], {}).get("result", "")]
    progress = [o.get("report") for o in outcomes if o.get("kind") == "wait" and o.get("report")]
    statics = read_json(session_dir(case, sid) / ".opendan_agent_session" / "static.json", {}) or {}
    checks = []

    def check(name, ok, detail=""):
        checks.append({"check": name, "ok": bool(ok), "detail": detail})

    check("session.finished_succeeded", st.get("run_state") == "finished" and st.get("outcome") == "succeeded",
          {"run_state": st.get("run_state"), "outcome": st.get("outcome"), "last_error": st.get("last_error")})
    check("session.final_report", bool((st.get("final_report") or {}).get("report")),
          (st.get("final_report") or {}).get("report", "")[:400])
    check("turns.many", st.get("turns_completed", 0) >= 2 and len(turns) == st.get("turn_seq"),
          {"turns_completed": st.get("turns_completed"), "turn_seq": st.get("turn_seq"), "turn_ended": len(turns)})
    check("shell.two_tasks", len(became_tasks) >= 2, [c["args"].get("command", "")[:120] for c in became_tasks])
    check("no_wait_calls", not waits, waits)
    check("progress_replies", len(progress) >= 1, progress[:5])
    expected_results = 2 if cfg["variant"] == "normal" else 1
    check("task_results", len(task_results) >= expected_results,
          [re.search(r"<<task_result[^>]*>>", t).group(0) for t in task_results if re.search(r"<<task_result[^>]*>>", t)])
    check("ticks", len(ticks) >= 1, len(ticks))
    # New output is shown at an inference while a task that prints runs: in
    # practice a tick's Turn (a model may redirect a build into a log file;
    # after a restart the old tasks are unknown to the new process).
    printing = [c for c in became_tasks if ">" not in c["args"].get("command", "")]
    expected = cfg["variant"] == "normal" and printing and ticks
    check("background_new_output", any("new output" in b for b in backgrounds) or not expected,
          {"backgrounds": len(backgrounds), "tasks_printing_while_running": len(printing), "ticks": len(ticks)})
    check("intents_recorded", any(c.get("intent") for c in calls), [c.get("intent") for c in calls if c.get("intent")][:6])
    if cfg["variant"] == "restart":
        check("restart.second_drive", (case / "drive2.json").exists(), "")
        check("restart.read_from_exec_dir_or_unknown",
              any("previous executor" in t or "status=\"unknown\"" in t for t in task_results),
              [t[:300] for t in task_results])
    summary = {
        "case": str(case), "variant": cfg["variant"], "model": cfg["model"], "session_id": sid,
        "passed": sum(c["ok"] for c in checks), "total": len(checks),
        "inferences": statics.get("rounds"), "tokens": statics.get("total_tokens"),
        "turns_completed": st.get("turns_completed"), "ticks": len(ticks), "nudges": len(nudges),
        "backgrounds": len(backgrounds), "seconds": read_json(case / "run.json", {}).get("seconds"),
        "checks": checks,
    }
    write_json(case / "verify.json", summary)
    write_json(case / "evidence.json", {
        "inputs": [u[:3000] for u in users],
        "calls": calls,
        "results": {k: v.get("result", "")[:2000] for k, v in results.items()},
        "outcomes": outcomes,
    })
    for c in checks:
        log(f"{'PASS' if c['ok'] else 'FAIL'} {c['check']}: {json.dumps(c['detail'], ensure_ascii=False)[:300]}")
    log(f"{summary['passed']}/{summary['total']} passed; {summary['inferences']} inferences, {summary['tokens']} tokens, {summary['seconds']}s")
    return 0 if summary["passed"] == summary["total"] else 1


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("phase", choices=["prepare", "run", "verify", "all"])
    ap.add_argument("--case")
    ap.add_argument("--variant", choices=["normal", "restart"], default="normal")
    a = ap.parse_args()
    case = Path(a.case) if a.case else CASE_ROOT / f"{time.strftime('%Y%m%dT%H%M%S')}-{a.variant}"
    if a.phase in ("prepare", "all"):
        prepare(case, a.variant)
    if a.phase in ("run", "all"):
        run(case)
    if a.phase in ("verify", "all"):
        sys.exit(verify(case))


if __name__ == "__main__":
    main()
