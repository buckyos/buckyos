#!/usr/bin/env python3
"""DV of the extension package system on a running Zone (ExtTools TODO X2 / X3 / X4).

Publishes small throwaway packages (a new version number each run: a Repo version never changes) and walks the desired
state through enable with a requires chain, upgrade, going back without a download, refused disable, disable and import
of an unchanged .pikg, watching what the node reports and what a worker container then runs.

    uv run test/ext_tools_dv/main.py            # needs a Zone: uv run src/check.py; a built `buckyos` in $BUCKYOS_ROOT/bin
"""
import base64
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

ROOT = Path(os.environ.get("BUCKYOS_ROOT", "/opt/buckyos"))
CLI = str(ROOT / "bin" / "buckyos")
ENDPOINT = os.environ.get("BUCKYOS_TEST_ENDPOINT", "http://127.0.0.1:3180")
CONTROL_PANEL = "http://127.0.0.1:4020/kapi/control-panel"
USER = os.environ.get("BUCKYOS_TEST_ADMIN_USER", "devtest")
PASSWORD = os.environ.get("BUCKYOS_TEST_ADMIN_PASSWORD", "bucky2025")
WORKER = os.environ.get("BUCKYOS_TEST_WORKER", "buckyos-app-extv")
WAIT_SECONDS = 180
STAMP = int(time.time()) % 100000
failures = []


def check(ok, what):
    print(("  ok   " if ok else "  FAIL ") + what, flush=True)
    if not ok:
        failures.append(what)


def login():
    nonce = int(time.time() * 1000)
    base = base64.b64encode(hashlib.sha256(f"{PASSWORD}{USER}.buckyos".encode()).digest()).decode()
    salted = base64.b64encode(hashlib.sha256(f"{base}{nonce}".encode()).digest()).decode()
    body = {
        "method": "auth.login",
        "params": {"username": USER, "password": salted, "appid": "control-panel", "target": {"kind": "system", "service_id": "control-panel"}, "login_nonce": nonce},
        "sys": [nonce],
    }
    request = urllib.request.Request(CONTROL_PANEL, data=json.dumps(body).encode(), headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.load(response)["result"]["session_token"]


class Cli:
    def __init__(self, token_file):
        self.token_file = token_file

    def local(self, *args, cwd=None):
        return subprocess.run([CLI, "--non-interactive", "--output", "json", *args], cwd=cwd, capture_output=True, text=True)

    def run(self, *args, cwd=None):
        result = subprocess.run(
            [CLI, "--endpoint", ENDPOINT, "--session-token-file", self.token_file, "--non-interactive", "--output", "json", *args],
            cwd=cwd, capture_output=True, text=True,
        )
        try:
            return json.loads(result.stdout.strip().splitlines()[-1])
        except (IndexError, ValueError):
            return {"ok": False, "error": {"message": f"{result.stdout} {result.stderr}"}}

    def ok(self, *args, cwd=None):
        out = self.run(*args, cwd=cwd)
        if not out.get("ok"):
            raise SystemExit(f"{' '.join(args)}: {json.dumps(out)[:600]}")
        return out["data"]


def make_package(cli, work, package_id, version, commands, requires=()):
    """A package whose commands print `<command> <version>`, found through store/<id> relative to the launcher."""
    project = work / f"{package_id}-{version}"
    project.mkdir(parents=True)
    init = cli.local("pikg", "init", ".", "--name", package_id, "--owner", f"did:bns:{USER}", "--kind", "ext-tools", "--source", ".", cwd=project)
    if init.returncode != 0:
        raise SystemExit(f"pikg init: {init.stdout} {init.stderr}")
    manifest_path = project / "ext-manifest.json"
    manifest = json.loads(manifest_path.read_text())
    platform = manifest["platforms"][0]
    manifest["version"] = version
    manifest["provides"] = [{"command": c, "version_probe": f"{c} --version", "smoke": f"smoke/{package_id}.sh"} for c in commands]
    manifest["requires"] = [{"id": r, "version": "*"} for r in requires]
    manifest_path.write_text(json.dumps(manifest, indent=2))
    payload = project / "payload" / platform
    for leftover in (payload / "bin").iterdir():
        leftover.unlink()
    shutil.copy(manifest_path, payload / "ext-manifest.json")
    (payload / "store" / package_id).mkdir(parents=True, exist_ok=True)
    (payload / "store" / package_id / "VERSION").write_text(version)
    for command in commands:
        launcher = payload / "bin" / command
        launcher.write_text(f'#!/bin/sh\nhere="$(cd "${{0%/*}}" && pwd -P)"\necho {command} $(cat "$here/../store/{package_id}/VERSION")\n')
        launcher.chmod(0o755)
    for smoke in (payload / "smoke").iterdir():
        smoke.unlink()
    (payload / "smoke" / f"{package_id}.sh").write_text("#!/bin/sh\nset -eu\n" + "".join(f"{c} --version | grep -q {version}\n" for c in commands))
    app_path = project / "dapp_meta" / "app.json"
    app = json.loads(app_path.read_text())
    app["version"] = version
    app_path.write_text(json.dumps(app, indent=2))
    meta = project / "dapp_meta"
    pikg_path = meta / "pikg.json"
    pikg = json.loads(pikg_path.read_text())
    pikg["pikg_file"] = f"{package_id}-{version}.pikg"
    pikg_path.write_text(json.dumps(pikg, indent=2))
    for step in (["pikg", "doctor", "dapp_meta"], ["pikg", "build", "dapp_meta"], ["pikg", "pack", "dapp_dist"]):
        result = cli.local(*step, cwd=project)
        if result.returncode != 0:
            raise SystemExit(f"{' '.join(step)} in {project}: {result.stdout} {result.stderr}")
    return project


def publish(cli, project):
    return cli.ok("app", "publish", str(project), "--app-doc", "dapp_dist/APPDOC.json", cwd=project)


def node_state(cli, package_id):
    status = cli.ok("ext-tools", "status")
    for package in status["packages"]:
        if package["id"] == package_id:
            return package
    return None


def wait_for(cli, package_id, what, predicate):
    deadline = time.time() + WAIT_SECONDS
    last = None
    while time.time() < deadline:
        last = node_state(cli, package_id)
        if predicate(last):
            check(True, what)
            return last
        time.sleep(3)
    check(False, f"{what} (last: {json.dumps(last)[:500]})")
    return last


def on_node(package, version):
    nodes = (package or {}).get("nodes") or {}
    return bool(nodes) and all(n["stage"] == "active" and n["version"] == version and all(t.get("version") == version for t in n["targets"].values()) for n in nodes.values())


def gone(package):
    return not (package or {}).get("nodes")


def in_worker(command):
    result = subprocess.run(["docker", "exec", WORKER, "sh", "-c", command], capture_output=True, text=True)
    return result.stdout.strip()


def main():
    work = Path(tempfile.mkdtemp(prefix="ext-tools-dv-"))
    token_file = work / "token"
    token_file.write_text(login())
    token_file.chmod(0o600)
    cli = Cli(str(token_file))
    base_id, tool_id = f"ext-dv-base-{STAMP}", f"ext-dv-tool-{STAMP}"
    old, new = "1.0.0", "1.1.0"
    base_cmd, tool_cmd = f"dvbase{STAMP}", f"dvtool{STAMP}"

    print("== publish")
    for package_id, version, commands, requires in (
        (base_id, old, [base_cmd], ()),
        (base_id, new, [base_cmd], ()),
        (tool_id, old, [tool_cmd], (base_id,)),
    ):
        publish(cli, make_package(cli, work, package_id, version, commands, requires))
    catalog = {p["id"]: p for p in cli.ok("ext-tools", "list")["packages"]}
    check(catalog[base_id]["versions"] == [new, old], "the catalog lists both versions, newest first")
    check(catalog[tool_id]["latest"]["manifest"]["provides"][0]["command"] == tool_cmd, "the catalog knows what a package provides")

    print("== a package that shadows a standard command never reaches the Repo")
    shadow = make_package(cli, work, f"ext-dv-shadow-{STAMP}", old, ["harmless"])
    manifest = json.loads((shadow / "ext-manifest.json").read_text())
    manifest["provides"][0]["command"] = "xagent"
    platform = manifest["platforms"][0]
    (shadow / "ext-manifest.json").write_text(json.dumps(manifest))
    (shadow / "payload" / platform / "ext-manifest.json").write_text(json.dumps(manifest))
    (shadow / "payload" / platform / "bin" / "harmless").rename(shadow / "payload" / platform / "bin" / "xagent")
    refused = cli.run("app", "publish", str(shadow), "--app-doc", "dapp_dist/APPDOC.json", cwd=shadow)
    check(not refused.get("ok") and "standard" in json.dumps(refused), "publish refuses it")

    print("== enable brings the requires chain")
    plan = cli.ok("ext-tools", "enable", tool_id, "--version", old, "--dry-run")
    check([e["manifest"]["id"] for e in plan["enable"]] == [base_id, tool_id], "the plan lists the dependency first")
    cli.ok("ext-tools", "enable", tool_id, "--version", old)
    wait_for(cli, tool_id, "the node installs the package", lambda p: on_node(p, old))
    wait_for(cli, base_id, "the node installs its dependency, at the newest version", lambda p: on_node(p, new))
    check(in_worker(f"{tool_cmd} --version") == f"{tool_cmd} {old}", "a worker that was already running sees the new command")
    check(in_worker(f"{base_cmd} --version") == f"{base_cmd} {new}", "and the dependency's")

    print("== upgrade and back")
    cli.ok("ext-tools", "enable", base_id, "--version", old)
    wait_for(cli, base_id, "going back to the old version", lambda p: on_node(p, old))
    check(in_worker(f"{base_cmd} --version") == f"{base_cmd} {old}", "the worker runs the old version")
    cli.ok("ext-tools", "enable", base_id, "--version", new)
    wait_for(cli, base_id, "going forward again", lambda p: on_node(p, new))
    check(in_worker(f"{base_cmd} --version") == f"{base_cmd} {new}", "the worker runs the new version")

    print("== disable")
    refused = cli.run("ext-tools", "disable", base_id)
    check(not refused.get("ok") and "requires" in json.dumps(refused), "a package something requires stays")
    cli.ok("ext-tools", "disable", tool_id)
    wait_for(cli, tool_id, "the node removes the package", gone)
    check(in_worker(f"command -v {tool_cmd} || echo none") == "none", "the command is gone from the worker")
    cli.ok("ext-tools", "disable", base_id)
    wait_for(cli, base_id, "and its dependency once nothing needs it", gone)

    print("== import keeps the bytes")
    imported_project = make_package(cli, work, f"ext-dv-import-{STAMP}", old, [f"dvimport{STAMP}"])
    pikg_file = next((imported_project / "dapp_dist").glob("*.pikg"))
    digest = "sha256:" + hashlib.sha256(pikg_file.read_bytes()).hexdigest()
    imported = cli.ok("--allow-read", str(pikg_file.parent), "ext-tools", "import", str(pikg_file))
    check(imported["pikg_digest"] == digest, "the Repo's digest is the file's")
    catalog = {p["id"]: p for p in cli.ok("ext-tools", "list")["packages"]}
    check(catalog[f"ext-dv-import-{STAMP}"]["latest"]["pikg_digest"] == digest, "the catalog shows it")

    shutil.rmtree(work, ignore_errors=True)
    print(f"\n{len(failures)} failures" if failures else "\nall passed")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
