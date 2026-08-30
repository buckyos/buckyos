import importlib.util
import hashlib
import json
import tarfile
import tempfile
import unittest
from pathlib import Path, PurePosixPath
from types import SimpleNamespace
from unittest import mock


SCRIPT_PATH = Path(__file__).resolve().parents[1] / "build_sdk_tool_distribution.py"
SPEC = importlib.util.spec_from_file_location("build_sdk_tool_distribution", SCRIPT_PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class WindowsOrderedPath:
    def __init__(self, actual: Path, relative: str) -> None:
        self.actual = actual
        self.relative = relative

    def __lt__(self, other: "WindowsOrderedPath") -> bool:
        return self.relative.casefold() < other.relative.casefold()

    def is_file(self) -> bool:
        return self.actual.is_file()

    def relative_to(self, _root: object) -> PurePosixPath:
        return PurePosixPath(self.relative)

    def open(self, *args, **kwargs):
        return self.actual.open(*args, **kwargs)

    def stat(self):
        return self.actual.stat()


class PackageRoot:
    def __init__(self, files: list[WindowsOrderedPath]) -> None:
        self.files = files

    def rglob(self, _pattern: str) -> list[WindowsOrderedPath]:
        return self.files


class PackageFileManifestTests(unittest.TestCase):
    def test_manifest_ignores_native_windows_path_order(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            temporary_root = Path(temporary)
            source_files = []
            for index, relative in enumerate(("cli/main.ts", "LICENSE", "README.md")):
                actual = temporary_root / str(index)
                actual.write_text(relative, encoding="utf-8")
                source_files.append(WindowsOrderedPath(actual, relative))
            package_root = PackageRoot(source_files)

            manifest = MODULE.package_file_manifest(package_root)

        self.assertEqual(
            [item["path"] for item in manifest],
            ["LICENSE", "README.md", "cli/main.ts"],
        )

    def test_build_does_not_modify_committed_launchers(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            temporary_root = Path(temporary)
            package_root = temporary_root / "package"
            package_files = {
                "LICENSE": "license",
                "dist/node.mjs": "export {};",
                "cli/main.ts": "export {};",
                "cli/system_bootstrap.ts": "export {};",
                "cli/system_launcher.ts": "export {};",
                "package.json": json.dumps({"name": "buckyos", "version": "1.2.3"}),
            }
            for relative, content in package_files.items():
                path = package_root / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(content, encoding="utf-8")

            artifact = temporary_root / "buckyos.tgz"
            with tarfile.open(artifact, "w:gz") as archive:
                for path in sorted(package_root.rglob("*")):
                    if path.is_file():
                        archive.add(path, arcname=f"package/{path.relative_to(package_root).as_posix()}")

            npm_files = MODULE.package_file_manifest(package_root)
            npm_files_sha256 = hashlib.sha256(
                json.dumps(npm_files, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
            ).hexdigest()
            release = {
                "schema_version": 1,
                "buckyos_version": "0.6.0",
                "build_id": "test",
                "tool_version": "1.2.3",
                "sdk_version": "1.2.3",
                "npm_tarball_sha256": "test",
                "npm_integrity": "test",
                "npm_files_sha256": npm_files_sha256,
                "npm_files": npm_files,
                "deno_version": "2.9.6",
                "deno_sha256": "test",
                "sbom_sha256": "test",
                "protocol_version": "1",
                "capability_range": "buckyos.tool.v1",
            }
            release_manifest = temporary_root / "release.json"
            release_manifest.write_text(json.dumps(release), encoding="utf-8")
            deno = temporary_root / "deno.exe"
            deno.write_bytes(b"deno")
            sbom = temporary_root / "sbom.json"
            sbom.write_text("{}", encoding="utf-8")

            rootfs = temporary_root / "rootfs"
            bin_dir = rootfs / "bin"
            bin_dir.mkdir(parents=True)
            posix_launcher = bin_dir / "buckyos"
            windows_launcher = bin_dir / "buckyos.cmd"
            posix_launcher.write_bytes(b"committed posix launcher")
            windows_launcher.write_bytes(b"committed windows launcher")

            args = SimpleNamespace(
                artifact=artifact,
                deno=deno,
                sbom=sbom,
                release_manifest=release_manifest,
                deno_version="2.9.6",
                rootfs=rootfs,
                windows=True,
            )
            with mock.patch.object(MODULE, "verify_artifacts"):
                MODULE.build(args)

            self.assertEqual(posix_launcher.read_bytes(), b"committed posix launcher")
            self.assertEqual(windows_launcher.read_bytes(), b"committed windows launcher")
            self.assertTrue((rootfs / "libexec" / "buckyos-tool" / "distribution.json").is_file())


if __name__ == "__main__":
    unittest.main()
