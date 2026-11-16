#!/usr/bin/env python3
"""Agent browser DV: the checks that send nothing to a model (see README.md).

    AGENT_BROWSER_DV=1 python3 test/agent_browser_dv/main.py

Without AGENT_BROWSER_DV=1 (as `test/run.py -p agent_browser_dv` runs it) it
reports NOT RUN (exit 77): the checks need a running Zone with an Agent whose
runtime has ext-core with the browser. The real-model steps are not run from
here: they cost tokens and are judged from the worklog (README.md, "Steps").

Checks:
  tools     the Agent's container has chromium, agent-browser, page-render and they start
  screen    the owner opens the test site's sign-in page through the screen, types (by the keyboard) and signs in
  render    a page rendered by Workflow's media.page reaches no network; its processes are not the service's user
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
EXIT_NOT_RUN = 77
ZONE = os.environ.get("AB_DV_ZONE", "test.buckyos.io")
AGENT = os.environ.get("AB_DV_AGENT", f"agenty.{ZONE}")
CONTAINER = os.environ.get("AB_DV_CONTAINER", "buckyos-app-" + AGENT.split(".")[0])
PORT = int(os.environ.get("AB_DV_SITE_PORT", "18741"))
SITE_FROM_AGENT = os.environ.get("AB_DV_SITE_HOST", "172.17.0.1")
PASSWORD = "Pw-dvc-83Kq7Lm2"


def log(msg: str) -> None:
    print(f"[agent_browser_dv] {msg}", flush=True)


def run(cmd: list[str], env: dict[str, str], timeout: int = 300) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, env=env, capture_output=True, text=True, timeout=timeout)


def main() -> int:
    if os.environ.get("AGENT_BROWSER_DV") != "1":
        log("NOT RUN: set AGENT_BROWSER_DV=1 (needs a running Zone, see README.md)")
        return EXIT_NOT_RUN
    out = Path(os.environ.get("AB_DV_OUT") or tempfile.mkdtemp(prefix="agent_browser_dv_"))
    out.mkdir(parents=True, exist_ok=True)
    env = {**os.environ, "AB_DV_OUT": str(out), "AB_DV_AGENT": AGENT, "AB_DV_ZONE": ZONE}
    failures: list[str] = []

    # tools: what a session of the Agent finds on its PATH
    tools = run(["docker", "exec", CONTAINER, "sh", "-c",
                 "ls /opt/buckyos/tools/bin | grep -E '^(chromium|agent-browser|page-render)$' | sort | tr '\\n' ' ';"
                 " /opt/buckyos/tools/bin/chromium --version && /opt/buckyos/tools/bin/agent-browser version"], env)
    (out / "tools.txt").write_text(tools.stdout + tools.stderr)
    if tools.returncode != 0 or "agent-browser chromium page-render" not in tools.stdout or "playwright-core" not in tools.stdout:
        failures.append(f"tools: {tools.stdout.strip()} {tools.stderr.strip()}")
    log(f"tools: {' | '.join(tools.stdout.split(chr(10)))}")

    site_dir = out / "site"
    site = subprocess.Popen(["node", str(HERE / "site.mjs"), str(PORT), str(site_dir), "--password", PASSWORD],
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    try:
        site.stdout.readline()
        # screen: the owner's path to a password field
        base = f"http://{SITE_FROM_AGENT}:{PORT}"
        # In a tab of its own (the Agent's other tabs are the user's), by the keyboard: a headed browser shows its own
        # toolbar above the page, so page coordinates are not the screen's.
        screen = run(["node", str(HERE / "screen.mjs"), "--out", str(out / "shots"),
                      f"open={base}/login", "key=Tab", "text=dvuser", "key=Tab", f"text={PASSWORD}",
                      "shot=login-filled", "key=Enter", "wait=1500", "shot=orders", "close-mine"], env)
        (out / "screen.txt").write_text(screen.stdout + screen.stderr)
        requests = [json.loads(line) for line in (site_dir / "requests.jsonl").read_text().splitlines()]
        signed_in = any(r["path"] == "/login" and r.get("login") == "ok" for r in requests)
        if screen.returncode != 0 or not signed_in or "我的订单" not in screen.stdout:
            failures.append("screen: the owner could not sign in through the screen (screen.txt)")
        log(f"screen: signed in through the screen = {signed_in}")
        urllib.request.urlopen(f"http://127.0.0.1:{PORT}/__reset").read()

        # render: media.page is cut off from the network
        deno = shutil.which("deno") or str(Path(os.environ.get("BUCKYOS_ROOT", "/opt/buckyos")) / "tools/bin/deno")
        render = run([deno, "run", "-A", "--unsafely-ignore-certificate-errors", "--config", str(HERE / "deno.json"),
                      str(HERE / "render_isolation.ts"), str(PORT), str(site_dir), str(out / "render")], env)
        (out / "render.txt").write_text(render.stdout + render.stderr)
        if render.returncode != 0:
            failures.append("render: " + (render.stdout.strip().splitlines() or ["no output"])[-1])
        log("render: " + (render.stdout.strip().splitlines() or ["no output"])[-1])
    finally:
        site.terminate()
        time.sleep(0.2)

    log(f"evidence in {out}")
    if failures:
        for failure in failures:
            log(f"FAILED {failure}")
        return 1
    log("PASSED (the real-model steps are run by hand: README.md)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
