#!/usr/bin/env python3
"""Finish a distributed copy of the buckyos-agent commands.

A distributed copy (`libexec/buckyos-agent` on a host, `bin/opendan/buckyos-agent`
in the agent image) runs without a package cache and without the network. Every
`npm:` specifier in its `deno.json` becomes one bundled module in `vendor/`,
with the license texts of what went into it beside it, and the import is
rewritten to that file. What only development uses (the lock file, the default
site template's tests) is left out. `buckyos-build.py` and `build_aios` both
call this on the directory they copied.
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

NPM_IMPORT = re.compile(r"""(?:from|import)\s*\(?\s*["']npm:""")
LICENSE_NAMES = ("license", "licence", "copying", "notice")


def _run(command: list[str], cwd: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(command, cwd=cwd, text=True, capture_output=True)


def _licenses(deno: str, entry: Path) -> str:
    info = _run([deno, "info", "--json", entry.name], entry.parent)
    cache = _run([deno, "info", "--json"], entry.parent)
    if info.returncode != 0 or cache.returncode != 0:
        raise RuntimeError(f"deno info failed: {(info.stderr or cache.stderr).strip()}")
    registry = Path(json.loads(cache.stdout)["npmCache"]) / "registry.npmjs.org"
    parts: list[str] = []
    for package in sorted(json.loads(info.stdout).get("npmPackages", {}).values(), key=lambda p: (p["name"], p["version"])):
        root = registry / package["name"] / package["version"]
        declared = "unknown"
        if (root / "package.json").is_file():
            declared = str(json.loads((root / "package.json").read_text(encoding="utf-8")).get("license", "unknown"))
        texts = sorted(f for f in root.iterdir() if f.is_file() and f.name.lower().split(".")[0] in LICENSE_NAMES) if root.is_dir() else []
        body = "\n\n".join(f.read_text(encoding="utf-8", errors="replace").strip() for f in texts)
        parts.append(f"{package['name']}@{package['version']} — {declared}\n\n{body or '(the package ships no license file)'}")
    return ("\n\n" + "-" * 78 + "\n\n").join(parts) + "\n"


DEVELOPMENT_ONLY = ("deno.lock", "site-template/test")


def finish(directory: Path, deno: str | None) -> int:
    for name in DEVELOPMENT_ONLY:
        path = directory / name
        if path.is_dir():
            shutil.rmtree(path)
        else:
            path.unlink(missing_ok=True)
    config_path = directory / "deno.json"
    config = json.loads(config_path.read_text(encoding="utf-8"))
    imports = config.get("imports", {})
    wanted = {name: spec for name, spec in imports.items() if isinstance(spec, str) and spec.startswith("npm:")}
    direct = [
        str(path.relative_to(directory))
        for path in sorted(directory.rglob("*.ts"))
        if "vendor" not in path.relative_to(directory).parts and NPM_IMPORT.search(path.read_text(encoding="utf-8"))
    ]
    if direct:
        print("npm packages are imported through deno.json, not by an `npm:` specifier in the source: " + ", ".join(direct))
        return 2
    if not wanted:
        return 0
    deno = deno or shutil.which("deno")
    if not deno:
        print("Deno is needed to bundle the npm imports of the buckyos-agent commands (" + ", ".join(wanted) + ").")
        return 127
    vendor_dir = directory / "vendor"
    vendor_dir.mkdir(exist_ok=True)
    for name, spec in wanted.items():
        file_name = re.sub(r"[^A-Za-z0-9._-]", "_", name)
        with tempfile.TemporaryDirectory() as temporary:
            entry = Path(temporary) / "entry.ts"
            entry.write_text(f'export * from "{spec}";\n', encoding="utf-8")
            bundled = _run([deno, "bundle", "--platform", "deno", "--output", str(vendor_dir / f"{file_name}.mjs"), entry.name], entry.parent)
            if bundled.returncode != 0:
                print(f"deno bundle {spec} failed:\n{bundled.stderr.strip()}")
                return bundled.returncode
            (vendor_dir / f"{file_name}.LICENSES.txt").write_text(_licenses(deno, entry), encoding="utf-8")
        imports[name] = f"./vendor/{file_name}.mjs"
    config_path.write_text(json.dumps(config, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path, help="the copied buckyos-agent directory (holds deno.json)")
    parser.add_argument("--deno", help="the Deno to bundle with (default: deno on PATH)")
    args = parser.parse_args()
    return finish(args.directory.resolve(), args.deno)


if __name__ == "__main__":
    sys.exit(main())
