#!/usr/bin/env python3
import argparse
import json
from pathlib import Path, PurePosixPath
import re

import yaml


def path_value(value):
    if not isinstance(value, str) or not re.fullmatch(r"[A-Za-z0-9._/+\-]+", value):
        raise ValueError("Invalid install path")
    path = PurePosixPath(value)
    if not path.parts or path.is_absolute() or ".." in path.parts or path.parts[0].startswith(".install-") or path.parts[0] == ".buckyos_installer_defaults":
        raise ValueError("Unsafe install path")
    return value


def create(project, component, version, platform, root):
    if component not in ("buckyos", "cyfs-gateway") or platform not in ("amd64-linux", "arm64-linux"):
        raise ValueError("Unsupported component or Linux platform")
    if not re.fullmatch(r"[0-9]+(?:\.[0-9]+)*\+build[0-9]{6}", version):
        raise ValueError("Invalid component version")
    app = project["apps"][component]
    modules, data_paths = app["modules"], app.get("data_paths", [])
    if not isinstance(modules, dict) or not isinstance(data_paths, list):
        raise ValueError("Invalid project installation layout")
    for value in [*modules.values(), *data_paths]:
        path_value(value)
        source = root / value
        if not source.exists():
            raise ValueError("Declared staged path is missing: " + value)
        if not source.resolve().is_relative_to(root.resolve()):
            raise ValueError("Staged path escapes its root: " + value)
    return {"schema": 1, "component": component, "version": version, "platform": platform,
            "modules": modules, "data_paths": list(dict.fromkeys(data_paths))}


def main():
    parser = argparse.ArgumentParser(description="Add an installation description to an existing staged root")
    parser.add_argument("--project", type=Path, required=True)
    parser.add_argument("--component", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--platform", required=True)
    parser.add_argument("--root", type=Path, required=True)
    args = parser.parse_args()
    descriptor = create(yaml.safe_load(args.project.read_text()), args.component, args.version, args.platform, args.root)
    destination = args.root / (".install-" + args.component + ".json")
    if destination.is_symlink():
        raise ValueError("Installation descriptor destination must not be a symlink")
    destination.write_text(json.dumps(descriptor, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
