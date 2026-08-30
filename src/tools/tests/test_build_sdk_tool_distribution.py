import importlib.util
import tempfile
import unittest
from pathlib import Path, PurePosixPath


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

    def test_generated_launchers_match_committed_rootfs_files(self) -> None:
        committed_bin_dir = SCRIPT_PATH.parents[1] / "rootfs" / "bin"
        with tempfile.TemporaryDirectory() as temporary:
            generated_bin_dir = Path(temporary) / "bin"

            MODULE.write_launchers(generated_bin_dir, windows=True)

            self.assertEqual(
                (generated_bin_dir / "buckyos").read_bytes(),
                (committed_bin_dir / "buckyos").read_bytes(),
            )
            self.assertEqual(
                (generated_bin_dir / "buckyos.cmd").read_bytes(),
                (committed_bin_dir / "buckyos.cmd").read_bytes(),
            )


if __name__ == "__main__":
    unittest.main()
