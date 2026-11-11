from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


BUILD_SCRIPT = Path(__file__).resolve().parents[2] / "publish/exttool/ext-core/build.sh"


class PruneTest(unittest.TestCase):
    def test_macos_firebird_removal_preserves_other_framework_links(self):
        source = BUILD_SCRIPT.read_text(encoding="utf-8")
        function = source.split("prune_soffice() {", 1)[1]
        code = function.split("<<'PY'\n", 1)[1].split("\nPY\n", 1)[0]
        for platform in ("darwin-arm64", "darwin-x86_64"):
            with self.subTest(platform=platform), tempfile.TemporaryDirectory() as temp:
                lo = Path(temp) / "libreoffice"
                contents = lo / "LibreOffice.app/Contents"
                firebird = contents / "Resources/firebird"
                intl = contents / "Frameworks/intl"
                macos = contents / "MacOS"
                for directory in (firebird, intl, macos):
                    directory.mkdir(parents=True)
                (firebird / "fbintl.conf").write_text("firebird config")
                (intl / "libfbintl.dylib").write_text("firebird intl library")
                core = contents / "Frameworks/libuno.dylib"
                core.write_text("required runtime library")
                config_link = intl / "fbintl.conf"
                alias = macos / "urelibs"
                core_link = contents / "Frameworks/libuno-current.dylib"
                unrelated = contents / "Frameworks/unrelated.conf"
                try:
                    config_link.symlink_to("../../Resources/firebird/fbintl.conf")
                    alias.symlink_to("../Frameworks", target_is_directory=True)
                    core_link.symlink_to("libuno.dylib")
                    unrelated.symlink_to("missing.conf")
                except OSError as exc:
                    if getattr(exc, "winerror", None) == 1314:
                        self.skipTest("creating symlinks requires Windows privileges")
                    raise
                self.assertTrue(config_link.exists())
                self.assertTrue((alias / "intl/fbintl.conf").exists())
                result = subprocess.run(
                    [sys.executable, "-c", code, str(lo), platform],
                    capture_output=True, text=True,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertFalse(firebird.exists())
                self.assertFalse(intl.exists())
                self.assertFalse(config_link.is_symlink())
                self.assertFalse((alias / "intl/fbintl.conf").is_symlink())
                self.assertTrue(alias.is_symlink())
                self.assertEqual((alias / "libuno.dylib").read_text(), "required runtime library")
                self.assertEqual(core_link.read_text(), "required runtime library")
                self.assertTrue(unrelated.is_symlink())


if __name__ == "__main__":
    unittest.main()
