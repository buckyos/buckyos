import importlib.util
import os
import plistlib
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


SCRIPT_PATH = Path(__file__).parents[1] / "macos_runtime_bundle.py"


def load_tool():
    spec = importlib.util.spec_from_file_location("macos_runtime_bundle", SCRIPT_PATH)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


MACHO = bytes.fromhex("cffaedfe") + b"\0" * 60
ELF = b"\x7fELF" + b"\0" * 60


class RuntimeBundleTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tool = load_tool()
        self.temporary = tempfile.TemporaryDirectory(prefix="runtime bundle ")
        self.module_dir = Path(self.temporary.name) / "bin" / "node-daemon"
        self.module_dir.mkdir(parents=True)

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def _write_raw(self, content: bytes) -> Path:
        raw = self.module_dir / "node_daemon"
        raw.write_bytes(content)
        raw.chmod(0o755)
        return raw

    def test_assemble_moves_the_only_binary_into_a_versioned_bundle(self) -> None:
        self._write_raw(MACHO)
        with patch.object(self.tool, "_sign") as sign, patch.object(self.tool, "_signature_failures", return_value=[]):
            self.assertEqual(self.tool.assemble(self.module_dir, version="1.2.3"), "assembled")
        bundle = self.module_dir / "BuckyOS Runtime.app"
        sign.assert_called_once()
        self.assertEqual(sign.call_args.args[1], "-")
        self.assertFalse((self.module_dir / "node_daemon").exists())
        self.assertEqual((bundle / "Contents" / "MacOS" / "node_daemon").read_bytes(), MACHO)
        self.assertTrue(os.access(bundle / "Contents" / "MacOS" / "node_daemon", os.X_OK))
        with (bundle / "Contents" / "Info.plist").open("rb") as handle:
            info = plistlib.load(handle)
        self.assertEqual(info["CFBundleIdentifier"], "com.buckyos.runtime")
        self.assertEqual(info["CFBundleExecutable"], "node_daemon")
        self.assertEqual(info["CFBundleShortVersionString"], "1.2.3")
        self.assertEqual(info["LSMinimumSystemVersion"], "14.0")
        self.assertFalse((self.module_dir / ".runtime-bundle-staging").exists())

    def test_rebuild_replaces_the_bundle_and_identity_comes_from_env(self) -> None:
        self._write_raw(MACHO)
        with (
            patch.object(self.tool, "_sign") as sign,
            patch.object(self.tool, "_signature_failures", return_value=[]),
            patch.dict(os.environ, {"BUCKYOS_MACOS_CODESIGN_IDENTITY": "Developer ID Application: Test"}),
        ):
            self.tool.assemble(self.module_dir, version="1.0.0")
            stale = self.module_dir / "BuckyOS Runtime.app" / "Contents" / "MacOS" / "old_helper"
            stale.write_bytes(b"old")
            self._write_raw(MACHO + b"new")
            self.assertEqual(self.tool.assemble(self.module_dir, version="1.0.1"), "assembled")
        self.assertEqual(sign.call_args.args[1], "Developer ID Application: Test")
        self.assertFalse(stale.exists())
        self.assertTrue(
            (self.module_dir / "BuckyOS Runtime.app" / "Contents" / "MacOS" / "node_daemon").read_bytes().endswith(b"new")
        )

    def test_failed_signature_keeps_the_raw_binary_and_previous_bundle(self) -> None:
        self._write_raw(MACHO)
        with patch.object(self.tool, "_sign", side_effect=subprocess.CalledProcessError(1, ["codesign"])):
            with self.assertRaises(subprocess.CalledProcessError):
                self.tool.assemble(self.module_dir, version="1.0.0")
        self.assertTrue((self.module_dir / "node_daemon").is_file())
        self.assertFalse((self.module_dir / "BuckyOS Runtime.app").exists())
        self.assertFalse((self.module_dir / ".runtime-bundle-staging").exists())

    def test_cross_build_output_removes_a_stale_bundle(self) -> None:
        (self.module_dir / "BuckyOS Runtime.app" / "Contents").mkdir(parents=True)
        self._write_raw(ELF)
        self.assertEqual(self.tool.assemble(self.module_dir), "not-macos")
        self.assertFalse((self.module_dir / "BuckyOS Runtime.app").exists())
        self.assertTrue((self.module_dir / "node_daemon").is_file())

    def test_no_new_binary_leaves_the_bundle_alone(self) -> None:
        self.assertEqual(self.tool.assemble(self.module_dir), "absent")
        (self.module_dir / "BuckyOS Runtime.app").mkdir()
        self.assertEqual(self.tool.assemble(self.module_dir), "unchanged")

    def test_verify_reports_loose_binaries_and_extra_executables(self) -> None:
        self._write_raw(MACHO)
        with patch.object(self.tool, "_sign"), patch.object(self.tool, "_signature_failures", return_value=[]):
            self.tool.assemble(self.module_dir, version="1.0.0")
        bundle = self.module_dir / "BuckyOS Runtime.app"
        self.assertEqual(self.tool.verify(bundle, check_signature=False), [])
        self._write_raw(MACHO)
        (bundle / "Contents" / "MacOS" / "node_daemon_copy").write_bytes(MACHO)
        failures = self.tool.verify(bundle, check_signature=False)
        self.assertTrue(any("loose node_daemon" in failure for failure in failures))
        self.assertTrue(any("node_daemon_copy" in failure for failure in failures))

    @unittest.skipUnless(sys.platform == "darwin" and shutil.which("codesign"), "needs macOS codesign")
    def test_real_ad_hoc_signature_survives_a_copy(self) -> None:
        shutil.copyfile(Path("/usr/bin/true"), self.module_dir / "node_daemon")
        self.assertEqual(self.tool.assemble(self.module_dir, version="1.0.0"), "assembled")
        copied = Path(self.temporary.name) / "copied"
        shutil.copytree(self.module_dir, copied)
        self.assertEqual(self.tool.verify(copied / "BuckyOS Runtime.app"), [])
        (copied / "BuckyOS Runtime.app" / "Contents" / "Resources" / "BuckyOSRuntime.icns").write_bytes(b"x")
        self.assertTrue(self.tool.verify(copied / "BuckyOS Runtime.app"))


if __name__ == "__main__":
    unittest.main()
