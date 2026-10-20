#!/usr/bin/env python3
"""Build and check the paios image manifest (bin/paios-images/paios_images.json).

platform  one architecture entry from `docker image inspect` and the exported tar
fragment  the images.<name> section from metadata.env and platform entries
assemble  the manifest from fragments (freshly built or inherited from the R2 pointer)
validate  structure, optionally tar URLs (HEAD) and bundled tar files
validate-fragment  one images.<name> section, e.g. the R2 pointer

Format: notepads/paios-image-release-todo.md §3.2. Rust reader:
src/kernel/node_daemon/src/paios_images.rs (same fixture in testdata/).
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import re
import sys
import urllib.error
import urllib.request

SCHEMA = "buckyos.paios-images/v1"
ARCHES = ("amd64", "aarch64")
NAMES = ("aios", "exttool")
SHA256_HEX = re.compile(r"[0-9a-f]{64}\Z")
DIGEST = re.compile(r"sha256:[0-9a-f]{64}\Z")
REVISION = re.compile(r"[0-9a-f]{40}\Z")
DEFAULT_BASE_URL = "https://buckyos-installer.tbudr.top"


def sha256_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def layers_sha256(layers):
    return "sha256:" + hashlib.sha256("\n".join(layers).encode()).hexdigest()


def read_env(path):
    values = {}
    for line in Path(path).read_text().splitlines():
        if "=" in line and not line.lstrip().startswith("#"):
            key, value = line.split("=", 1)
            values[key.strip()] = value.strip()
    return values


def platform_entry(name, arch, ref, inspect, manifest_digest, tar, base_url):
    data = json.loads(Path(inspect).read_text())
    if isinstance(data, list):
        if len(data) != 1:
            raise ValueError("inspect output must describe one image")
        data = data[0]
    layers = data["RootFS"]["Layers"]
    sha256, tar = sha256_file(tar), Path(tar)
    path = f"paios-images/{name}/{arch}/{sha256}.tar.gz"
    entry = {"ref": ref, "image_id": data["Id"], "manifest_digest": manifest_digest,
             "rootfs_layers_sha256": layers_sha256(layers),
             "tar": {"file": tar.name, "sha256": sha256, "size": tar.stat().st_size,
                     "path": path, "url": base_url.rstrip("/") + "/" + path}}
    validate_platform(entry, f"{name}.{arch}")
    return entry


def fragment(name, repo, metadata, platforms):
    meta = read_env(metadata)
    image = {"repo": repo, "version": meta["version"], "revision": meta["revision"],
             "created": meta["created"], "platforms": dict(sorted(platforms.items()))}
    validate_image(name, image, require_arches=True)
    return image


def assemble(fragments, buckyos_revision, generated_at=None):
    if not REVISION.fullmatch(buckyos_revision):
        raise ValueError("buckyos_revision must be a 40-hex commit")
    manifest = {"schema": SCHEMA, "buckyos_revision": buckyos_revision,
                "generated_at": generated_at or datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
                "images": {name: fragments[name] for name in NAMES if name in fragments}}
    validate(manifest, require_arches=True)
    return manifest


def validate_platform(entry, where):
    ref = entry.get("ref")
    if not isinstance(ref, str) or not ref.strip() or "@" in ref:
        raise ValueError(where + ": ref must be a tag reference")
    for field in ("image_id", "manifest_digest", "rootfs_layers_sha256"):
        value = entry.get(field)
        if (value is not None or field != "manifest_digest") and not (isinstance(value, str) and DIGEST.fullmatch(value)):
            raise ValueError(f"{where}: {field} must be sha256:<64 hex>")
    tar = entry.get("tar")
    if not isinstance(tar, dict):
        raise ValueError(where + ": tar is missing")
    file = tar.get("file")
    if not isinstance(file, str) or file in ("", ".", "paios_images.json") or ".." in file or "/" in file or "\\" in file:
        raise ValueError(f"{where}: tar.file {file!r} must be a plain file name")
    if not (isinstance(tar.get("sha256"), str) and SHA256_HEX.fullmatch(tar["sha256"])):
        raise ValueError(where + ": tar.sha256 must be 64 hex")
    if type(tar.get("size")) is not int or tar["size"] <= 0:
        raise ValueError(where + ": tar.size must be a positive integer")
    path = tar.get("path")
    if path is not None and (not isinstance(path, str) or not path.startswith("paios-images/") or ".." in path.split("/")):
        raise ValueError(where + ": tar.path must stay under paios-images/")
    url = tar.get("url")
    if url is not None and (not isinstance(url, str) or not url.startswith("https://") or (path and not url.endswith("/" + path))):
        raise ValueError(where + ": tar.url must be HTTPS and end with tar.path")


def validate_image(name, image, require_arches=False):
    if not isinstance(image, dict) or not image.get("repo") or not image.get("revision"):
        raise ValueError(f"images.{name} needs repo and revision")
    platforms = image.get("platforms")
    if not isinstance(platforms, dict):
        raise ValueError(f"images.{name}.platforms is missing")
    for arch, entry in platforms.items():
        if arch not in ARCHES:
            raise ValueError(f"images.{name}.platforms has unknown arch {arch!r}")
        validate_platform(entry, f"images.{name}.platforms.{arch}")
    if require_arches:
        if not REVISION.fullmatch(image["revision"]):
            raise ValueError(f"images.{name}.revision must be a clean 40-hex commit")
        missing = [arch for arch in ARCHES if arch not in platforms]
        if missing:
            raise ValueError(f"images.{name} is missing {', '.join(missing)}")


def validate(manifest, require_arches=False):
    if not isinstance(manifest, dict) or manifest.get("schema") != SCHEMA:
        raise ValueError("unsupported schema")
    images = manifest.get("images")
    if not isinstance(images, dict) or "aios" not in images:
        raise ValueError("images.aios is missing")
    for name, image in images.items():
        validate_image(name, image, require_arches)


def head(url):
    request = urllib.request.Request(url, method="HEAD", headers={"Cache-Control": "no-cache", "User-Agent": "buckyos-paios-images/1"})
    with urllib.request.urlopen(request, timeout=60) as response:
        return response.status, int(response.headers.get("Content-Length", "-1"))


def check_urls(manifest, head=head):
    for name, image in manifest["images"].items():
        for arch, entry in image["platforms"].items():
            tar = entry["tar"]
            if not tar.get("url"):
                raise ValueError(f"images.{name}.platforms.{arch}: tar.url is missing")
            try:
                status, size = head(tar["url"])
            except urllib.error.HTTPError as error:
                status, size = error.code, -1
            if status != 200 or size != tar["size"]:
                raise ValueError(f"{tar['url']}: HTTP {status}, size {size}, expected {tar['size']}")


def check_tars(manifest, directory, arch):
    for name, image in manifest["images"].items():
        entry = image["platforms"].get(arch)
        if entry is None:
            raise ValueError(f"images.{name} has no {arch} entry")
        tar, path = entry["tar"], Path(directory) / entry["tar"]["file"]
        if not path.is_file():
            raise ValueError(f"missing {path}")
        if path.stat().st_size != tar["size"] or sha256_file(path) != tar["sha256"]:
            raise ValueError(f"{path} does not match the manifest")


def write_json(value, output):
    text = json.dumps(value, indent=2) + "\n"
    if output:
        Path(output).write_text(text)
    else:
        sys.stdout.write(text)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)
    command = commands.add_parser("platform")
    command.add_argument("--name", choices=NAMES, required=True)
    command.add_argument("--arch", choices=ARCHES, required=True)
    command.add_argument("--ref", required=True)
    command.add_argument("--inspect", required=True, help="docker image inspect JSON file")
    command.add_argument("--manifest-digest", required=True)
    command.add_argument("--tar", required=True)
    command.add_argument("--base-url", default=DEFAULT_BASE_URL)
    command.add_argument("--output")
    command = commands.add_parser("fragment")
    command.add_argument("--name", choices=NAMES, required=True)
    command.add_argument("--repo", required=True)
    command.add_argument("--metadata", required=True, help="metadata.env from build_aios/build_exttool --prepare-context")
    command.add_argument("--platform", action="append", required=True, metavar="ARCH=FILE")
    command.add_argument("--output")
    command = commands.add_parser("assemble")
    command.add_argument("--fragment", action="append", required=True, metavar="NAME=FILE")
    command.add_argument("--buckyos-revision", required=True)
    command.add_argument("--output")
    command = commands.add_parser("validate-fragment")
    command.add_argument("name", choices=NAMES)
    command.add_argument("fragment")
    command = commands.add_parser("validate")
    command.add_argument("manifest")
    command.add_argument("--check-urls", action="store_true")
    command.add_argument("--tar-dir")
    command.add_argument("--arch", choices=ARCHES)
    args = parser.parse_args(argv)

    if args.command == "platform":
        write_json(platform_entry(args.name, args.arch, args.ref, args.inspect, args.manifest_digest, args.tar, args.base_url), args.output)
    elif args.command == "fragment":
        platforms = {}
        for item in args.platform:
            arch, path = item.split("=", 1)
            platforms[arch] = json.loads(Path(path).read_text())
        write_json(fragment(args.name, args.repo, args.metadata, platforms), args.output)
    elif args.command == "assemble":
        fragments = {}
        for item in args.fragment:
            name, path = item.split("=", 1)
            if name not in NAMES:
                raise ValueError("unknown image " + name)
            fragments[name] = json.loads(Path(path).read_text())
        write_json(assemble(fragments, args.buckyos_revision), args.output)
    elif args.command == "validate-fragment":
        image = json.loads(Path(args.fragment).read_text())
        validate_image(args.name, image, require_arches=True)
        print(f"{args.name} fragment OK: revision {image['revision']}")
    else:
        manifest = json.loads(Path(args.manifest).read_text())
        validate(manifest)
        if args.check_urls:
            check_urls(manifest)
        if args.tar_dir:
            if not args.arch:
                parser.error("--tar-dir requires --arch")
            check_tars(manifest, args.tar_dir, args.arch)
        print("paios_images.json OK: " + ", ".join(f"{name}={image['revision'][:12]}" for name, image in manifest["images"].items()))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ValueError, KeyError, OSError) as error:
        print("paios_images: " + str(error), file=sys.stderr)
        raise SystemExit(1) from None
