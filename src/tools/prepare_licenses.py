#!/usr/bin/env python3

import argparse
import json
from pathlib import Path
import shutil
import subprocess


REPOSITORY = Path(__file__).resolve().parents[2]
SOURCE_URL = "https://github.com/buckyos/buckyos"


def source_record(repository: Path) -> dict:
    try:
        commit = subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=repository, text=True,
            stderr=subprocess.DEVNULL,
        ).strip()
        modified = bool(subprocess.check_output(
            ["git", "status", "--porcelain", "--untracked-files=normal"],
            cwd=repository, text=True, stderr=subprocess.DEVNULL,
        ).strip())
    except (OSError, subprocess.CalledProcessError):
        commit, modified = None, None
    return {
        "schema_version": 1,
        "scope": "packaging_checkout",
        "binary_source_verified": False,
        "license": "AGPL-3.0-only",
        "repository": SOURCE_URL,
        "commit": commit,
        "working_tree_modified": modified,
        "source_tree": f"{SOURCE_URL}/tree/{commit}" if commit else None,
        "corresponding_source": "This record identifies the packaging checkout; it does not verify that a binary was built from this source. Distributors must provide the actual source, resolved dependencies, patches and build materials. A source-tree link does not include local changes, dependencies or submodules. See SOURCE.md.",
    }


def prepare(output: Path, repository: Path = REPOSITORY) -> dict:
    sources = {
        "LICENSE": repository / "LICENSE",
        "NOTICE": repository / "NOTICE",
        "SOURCE.md": repository / "doc/licensing.md",
    }
    for source in sources.values():
        if not source.is_file():
            raise FileNotFoundError(f"missing distribution license material: {source}")
    output.mkdir(parents=True, exist_ok=True)
    for name, source in sources.items():
        shutil.copyfile(source, output / name)
    record = source_record(repository)
    (output / "source.json").write_text(
        json.dumps(record, indent=2) + "\n", encoding="utf-8",
    )
    return record


def main() -> None:
    parser = argparse.ArgumentParser(description="Stage BuckyOS license and source provenance for distribution")
    parser.add_argument("--output", type=Path, default=REPOSITORY / "src/rootfs/licenses/buckyos")
    args = parser.parse_args()
    prepare(args.output)


if __name__ == "__main__":
    main()
