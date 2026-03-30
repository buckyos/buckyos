#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path


TEST_ROOT = Path(__file__).resolve().parent


@dataclass
class RunnerSpec:
    module_name: str
    path: Path
    kind: str
    command: list[str]
    cwd: Path


def load_package_json(package_json_path: Path) -> dict:
    try:
        return json.loads(package_json_path.read_text(encoding="utf-8"))
    except Exception as error:
        raise RuntimeError(f"failed to read {package_json_path}: {error}") from error


def detect_runner(path: Path) -> RunnerSpec | None:
    if path.name == "__pycache__":
        return None

    if path.is_file():
        if path.suffix == ".sh":
            return RunnerSpec(
                module_name=path.stem,
                path=path,
                kind="shell",
                command=["bash", path.name],
                cwd=path.parent,
            )
        if path.suffix == ".py":
            return RunnerSpec(
                module_name=path.stem,
                path=path,
                kind="python",
                command=[sys.executable, path.name],
                cwd=path.parent,
            )
        return None

    if not path.is_dir():
        return None

    package_json_path = path / "package.json"
    cargo_toml_path = path / "Cargo.toml"
    main_py_path = path / "main.py"
    python_tests = sorted(path.glob("test_*.py"))

    if package_json_path.exists():
        package_json = load_package_json(package_json_path)
        scripts = package_json.get("scripts") or {}
        if "test" in scripts:
            install_cmd = ["pnpm", "install"]
            if (path / "pnpm-lock.yaml").exists():
                install_cmd.append("--frozen-lockfile")
            test_cmd = ["pnpm", "test"]
            return RunnerSpec(
                module_name=path.name,
                path=path,
                kind="node",
                command=["bash", "-lc", f"{' '.join(install_cmd)} && {' '.join(test_cmd)}"],
                cwd=path,
            )

    if cargo_toml_path.exists():
        return RunnerSpec(
            module_name=path.name,
            path=path,
            kind="cargo",
            command=["cargo", "test", "--manifest-path", str(cargo_toml_path)],
            cwd=TEST_ROOT.parent,
        )

    if main_py_path.exists():
        return RunnerSpec(
            module_name=path.name,
            path=path,
            kind="python-main",
            command=[sys.executable, str(main_py_path)],
            cwd=TEST_ROOT.parent,
        )

    if python_tests:
        return RunnerSpec(
            module_name=path.name,
            path=path,
            kind="python-unittest",
            command=[
                sys.executable,
                "-m",
                "unittest",
                "discover",
                "-s",
                str(path),
                "-p",
                "test_*.py",
            ],
            cwd=TEST_ROOT.parent,
        )

    return None


def discover_modules() -> dict[str, RunnerSpec]:
    modules: dict[str, RunnerSpec] = {}

    for child in sorted(TEST_ROOT.iterdir()):
        if child.is_file() and child.suffix != ".sh":
            continue
        runner = detect_runner(child)
        if runner is None:
            continue
        modules[runner.module_name] = runner

    return modules


def resolve_module(name: str, modules: dict[str, RunnerSpec]) -> RunnerSpec:
    if name in modules:
        return modules[name]

    normalized = name.removesuffix(".sh").removesuffix(".py")
    if normalized in modules:
        return modules[normalized]

    exact_path = TEST_ROOT / name
    runner = detect_runner(exact_path)
    if runner is not None:
        return runner

    raise KeyError(name)


def run_module(spec: RunnerSpec) -> int:
    print(
        f"[run] module={spec.module_name} kind={spec.kind} cwd={spec.cwd}",
        flush=True,
    )
    print(f"[cmd] {' '.join(spec.command)}", flush=True)
    completed = subprocess.run(spec.command, cwd=spec.cwd)
    return completed.returncode


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Run tests under ./test with cargo-like -p selection.",
    )
    parser.add_argument(
        "-p",
        "--package",
        action="append",
        dest="packages",
        help="Module to run, for example: -p aicc_test",
    )
    parser.add_argument(
        "--list",
        action="store_true",
        help="List runnable test modules",
    )
    args = parser.parse_args()

    modules = discover_modules()

    if args.list:
        for name in sorted(modules):
            spec = modules[name]
            print(f"{name}\t{spec.kind}\t{spec.path.relative_to(TEST_ROOT.parent)}")
        return 0

    if not args.packages:
        parser.error("at least one -p/--package is required unless --list is used")

    exit_code = 0
    for package in args.packages:
        try:
            spec = resolve_module(package, modules)
        except KeyError:
            print(f"unknown test module: {package}", file=sys.stderr)
            print("available modules:", file=sys.stderr)
            for name in sorted(modules):
                print(f"  {name}", file=sys.stderr)
            return 2

        code = run_module(spec)
        if code != 0:
            exit_code = code
            break

    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
