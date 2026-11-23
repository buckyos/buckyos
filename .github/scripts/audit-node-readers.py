#!/usr/bin/env python3
"""List persisted formats to review for a staged change or a PR."""
import argparse
from pathlib import Path
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[2]
REGISTRY = Path("src/kernel/node_updater/readers.toml")


def affected(readers, changes, dependency=None):
    for reader in readers:
        if dependency:
            prefix = dependency + ":"
            if changes & {p.removeprefix(prefix) for p in reader["audit"] if p.startswith(prefix)}:
                yield reader
            continue
        implementation = reader["rust"].split("::")[0]
        paths = {f"src/{name}" for name in reader["audit"] if ":" not in name}
        if any(name.startswith("cyfs-ndn:") for name in reader["audit"]):
            paths.update({"src/Cargo.toml", "src/Cargo.lock", "src/kernel/node_updater/Cargo.toml"})
        paths.update({REGISTRY.as_posix(), "src/kernel/node_updater/src/readers/mod.rs",
                      f"src/kernel/node_updater/src/readers/{implementation}.rs"})
        if paths & changes:
            yield reader


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", help="Compare a committed tree to this base (defaults to staged changes)")
    parser.add_argument("--dependency", type=Path, help="Audit changes in a registered dependency checkout")
    args = parser.parse_args()
    with (ROOT / REGISTRY).open("rb") as file:
        registry = tomllib.load(file)
    checkout = args.dependency.resolve() if args.dependency else ROOT
    dependency = checkout.name if args.dependency else None
    if dependency and dependency not in registry["audited_sources"]:
        parser.error(f"unregistered format source {dependency}")
    diff = ["git", "diff", "--name-only", "-z"]
    diff.extend([args.base, "HEAD"] if args.base else ["--cached"])
    changes = set(subprocess.check_output(diff, cwd=checkout).decode().split("\0"))
    for reader in affected(registry["reader"], changes, dependency):
        print(f"Desktop schema review — {reader['id']} ({reader['owner']}), target {reader['schema']}")


if __name__ == "__main__":
    main()
