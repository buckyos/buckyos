#!/usr/bin/env python3

import json
import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
COMMIT = re.compile(r"[0-9a-f]{40}")
PACKAGE_SPEC = re.compile(r"github:buckyos/buckyos-websdk#([0-9a-f]{40})")
RAW_IMPORT = re.compile(
    r"https://raw\.githubusercontent\.com/buckyos/buckyos-websdk/([0-9a-f]{40})/"
)


def package_files() -> list[Path]:
    output = []
    for top in (ROOT / "src", ROOT / "test"):
        for path in top.rglob("package.json"):
            if not {"node_modules", "dist", "target"}.intersection(path.parts):
                output.append(path)
    return sorted(output)


def nearest_lock(directory: Path) -> Path | None:
    current = directory
    while current != ROOT.parent:
        candidate = current / "pnpm-lock.yaml"
        if candidate.is_file():
            return candidate
        if current == ROOT:
            break
        current = current.parent
    return None


def collect_package_refs() -> set[str]:
    refs = set()
    for path in package_files():
        package = json.loads(path.read_text(encoding="utf-8"))
        dependencies = {
            **package.get("dependencies", {}),
            **package.get("devDependencies", {}),
        }
        if "buckyos" not in dependencies:
            continue
        spec = dependencies["buckyos"]
        match = PACKAGE_SPEC.fullmatch(spec)
        if match is None:
            raise ValueError(f"{path.relative_to(ROOT)} has a moving or unsupported buckyos spec: {spec}")
        ref = match.group(1)
        refs.add(ref)
        lock = nearest_lock(path.parent)
        if lock is None:
            raise ValueError(f"{path.relative_to(ROOT)} has no pnpm lockfile")
        text = lock.read_text(encoding="utf-8")
        marker = f"buckyos@https://codeload.github.com/buckyos/buckyos-websdk/tar.gz/{ref}:"
        offset = text.find(marker)
        if offset < 0:
            raise ValueError(f"{lock.relative_to(ROOT)} does not lock {ref}")
        resolution = text[offset : offset + len(marker) + 512]
        if "integrity: sha512-" not in resolution:
            raise ValueError(f"{lock.relative_to(ROOT)} does not lock the SDK tarball integrity")
    return refs


def collect_deno_refs() -> set[str]:
    refs = set()
    for path in (ROOT / "src" / "deno.json", ROOT / "test" / "deno.json"):
        value = json.loads(path.read_text(encoding="utf-8"))
        for target in value.get("imports", {}).values():
            if "buckyos/buckyos-websdk" not in target:
                continue
            match = RAW_IMPORT.match(target)
            if match is None:
                raise ValueError(f"{path.relative_to(ROOT)} has a moving SDK import: {target}")
            ref = match.group(1)
            refs.add(ref)
            lock = path.with_name("deno.lock")
            if not lock.is_file() or target not in lock.read_text(encoding="utf-8"):
                raise ValueError(f"{lock.relative_to(ROOT)} does not lock {target}")
    return refs


def reject_moving_refs() -> None:
    patterns = (
        "buckyos-websdk#main",
        "buckyos-websdk#beta",
        "buckyos-websdk/main/",
        "buckyos-websdk/beta/",
    )
    for top in (ROOT / "src", ROOT / "test", ROOT / "harness", ROOT / "doc"):
        for path in top.rglob("*"):
            if not path.is_file() or {"node_modules", "dist", "target"}.intersection(path.parts):
                continue
            if path.suffix not in {".json", ".md", ".mjs", ".ts", ".yaml", ".yml"}:
                continue
            text = path.read_text(encoding="utf-8", errors="ignore")
            if any(pattern in text for pattern in patterns):
                raise ValueError(f"moving buckyos-websdk reference in {path.relative_to(ROOT)}")


def main() -> None:
    refs = collect_package_refs() | collect_deno_refs()
    if len(refs) != 1:
        raise ValueError(f"BuckyOS consumers do not share one SDK/Tool commit: {sorted(refs)}")
    ref = next(iter(refs))
    if COMMIT.fullmatch(ref) is None:
        raise ValueError(f"invalid SDK/Tool commit: {ref}")
    reject_moving_refs()
    print(f"SDK/Tool consumers share immutable commit {ref} with lockfile integrity")


if __name__ == "__main__":
    main()
