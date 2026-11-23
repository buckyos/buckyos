#!/usr/bin/env python3
"""List the persisted formats to review since the review recorded in readers.toml."""
import argparse
from pathlib import Path
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[2]
REGISTRY = Path("src/kernel/node_updater/readers.toml")


def git(checkout, *args):
    return subprocess.check_output(["git", "-C", str(checkout), *args], text=True, encoding="utf-8")


def changed_files(checkout, base):
    return set(git(checkout, "diff", "--name-only", "-z", base, "HEAD").split("\0")) - {""}


def affected(readers, changes, dependency=None):
    for reader in readers:
        if dependency:
            prefix = dependency + ":"
            paths = {p.removeprefix(prefix) for p in reader["audit"] if p.startswith(prefix)}
        else:
            implementation = reader["rust"].split("::")[0]
            paths = {f"src/{name}" for name in reader["audit"] if ":" not in name}
            if any(":" in name for name in reader["audit"]):
                paths.update({"src/Cargo.toml", "src/Cargo.lock", "src/kernel/node_updater/Cargo.toml"})
            paths.update({REGISTRY.as_posix(), "src/kernel/node_updater/src/readers/mod.rs",
                          f"src/kernel/node_updater/src/readers/{implementation}.rs"})
        if paths & changes:
            yield reader, sorted(paths & changes)


def report(registry, checkout, base, dependency=None):
    name = dependency or "buckyos"
    count = git(checkout, "rev-list", "--count", f"{base}..HEAD").strip()
    yield f"{name}: {count} commits since {base[:12]}"
    for reader, files in affected(registry["reader"], changed_files(checkout, base), dependency):
        yield f"Desktop schema review — {reader['id']} ({reader['owner']}), target {reader['schema']}"
        for file in files:
            yield f"    {file}"


def audit(root, base=None, dependency=None):
    with (root / REGISTRY).open("rb") as file:
        registry = tomllib.load(file)
    sources = registry["audited_sources"]
    if dependency:
        if dependency.name not in sources:
            raise ValueError(f"unregistered format source {dependency.name}")
        yield from report(registry, dependency, base or sources[dependency.name], dependency.name)
        return
    if base:
        yield from report(registry, root, base)
        return
    review = registry["review"]
    yield f"Last review {review['date']}, readers last changed {review['changed']}: {review['conclusion'].strip()}"
    yield from report(registry, root, review["commit"])
    for name, revision in sources.items():
        if (root.parent / name / ".git").exists():
            yield from report(registry, root.parent / name, revision, name)
        else:
            yield f"{name}: not checked, no checkout at {root.parent / name}"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", help="Compare to this commit instead of the recorded review")
    parser.add_argument("--dependency", type=Path, help="Audit only this registered dependency checkout")
    args = parser.parse_args()
    try:
        for line in audit(ROOT, args.base, args.dependency and args.dependency.resolve()):
            print(line)
    except ValueError as error:
        parser.error(str(error))


if __name__ == "__main__":
    main()
