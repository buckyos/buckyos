#!/usr/bin/env python3
"""Exercise the staged, native executable with a disposable Desktop installation."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time


def verify(staged_root):
    binary = staged_root / "bin/node-updater" / ("node_updater.exe" if sys.platform == "win32" else "node_updater")
    release = json.loads((binary.parent / "release.json").read_bytes())
    build = release["target_build"]
    with tempfile.TemporaryDirectory() as temp:
        work = Path(temp)
        root = work / "node"
        security = work / "external-security"
        retained = root / "etc/start_config.json"
        personal = root / "data/home/untouched.txt"
        retained.parent.mkdir(parents=True)
        personal.parent.mkdir(parents=True)
        retained.write_bytes(b'{"schema":"buckyos.start_config.v1","cluster":"test"}\n')
        personal.write_bytes(b"personal")

        def migrate(expect):
            result = work / "result.json"
            result.unlink(missing_ok=True)
            run = subprocess.run([str(binary.resolve()), "migrate", "--root", str(root),
                                  "--security-root", str(security), "--target-build", build, "--mode", "silent",
                                  "--profile", "full", "--result", str(result)], cwd=work,
                                 stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=60)
            report = json.loads(result.read_bytes())
            if report["target_build"] != build or (run.returncode, report["status"]) != expect:
                raise ValueError(f"unexpected migration result {report}: {run.stderr}")
            return report

        account = root / "etc/host_account.json"
        receipt = root / "etc/.last-buckyos-install.json"
        account.write_bytes(b'{"account":"alice"}\n')
        failed = migrate((1, "unsupported_schema"))
        if failed["module"] != "host-account" or failed["recovery"] != "unchanged" or security.exists():
            raise ValueError("unversioned host account was accepted or data was modified")
        versioned = b'{"schema":"buckyos.host_account.v1","account":"alice"}\n'
        account.write_bytes(versioned)

        # The later-owned invalid executor must leave the earlier location move uncommitted.
        executor = root / "local/node_daemon/executor"
        executor.mkdir(parents=True)
        failed = migrate((1, "unsupported_schema"))
        if failed["recovery"] != "unchanged" or not retained.is_file() or security.exists():
            raise ValueError("failed prepare changed live data")
        executor.rmdir()
        migrate((0, "success"))
        retry = migrate((0, "success"))
        if (retained.exists() or (security / retained.name).read_bytes() != b'{"schema":"buckyos.start_config.v1","cluster":"test"}\n'
                or retry["changed_areas"] or personal.read_bytes() != b"personal"
                or account.read_bytes() != versioned or (account.parent / "host_account.schema.json").exists()
                or receipt.exists()):
            raise ValueError("migration retry changed retained files")
        start = int(time.time())
        subprocess.run([str(binary.resolve()), "record-install", "--root", str(root),
                        "--target-build", build], cwd=work, check=True, timeout=60)
        completed = json.loads(receipt.read_bytes())
        if (completed["schema"] != "buckyos.completed_install.v1" or completed["target_build"] != build
                or not completed["runtime_build"].startswith(build)
                or not start <= completed["completed_at"] <= int(time.time())):
            raise ValueError(f"unexpected completed installation record: {completed}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("staged_root", type=Path)
    verify(parser.parse_args().staged_root)
