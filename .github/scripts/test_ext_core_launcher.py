import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


BUILD_SCRIPT = Path(__file__).resolve().parents[2] / "publish/exttool/ext-core/build.sh"


@unittest.skipIf(os.name == "nt", "a POSIX launcher")
class OnlineLauncherTest(unittest.TestCase):
    """bin/chromium finds <tools>/online/chromium from wherever a node has the package."""

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        source = BUILD_SCRIPT.read_text(encoding="utf-8")
        function = source[source.index("unix_online_launcher() {"):source.index("# A prefix's data directories")]
        payload = self.root / "payload"
        (payload / "bin").mkdir(parents=True)
        script = "set -euo pipefail\n" + function + '\nunix_online_launcher "$1" browse browser "rev-7/the browser-bin" "rev-7/the browser" "The Browser"\n'
        subprocess.run([shutil.which("bash"), "--noprofile", "--norc", "-s", "--", str(payload)], input=script, text=True, check=True, timeout=30)
        # What a payload carries of the item: its overlay, without the downloaded file.
        (payload / "online/browser/rev-7").mkdir(parents=True)
        self.payload = payload

    def install(self, root, *inside):
        tools = self.root / root
        package = tools.joinpath(*inside)
        package.parent.mkdir(parents=True, exist_ok=True)
        shutil.copytree(self.payload, package)
        return tools, package / "bin/browse"

    def download(self, tools):
        item = tools / "online/browser/rev-7"
        item.mkdir(parents=True, exist_ok=True)
        (item / "the browser-bin").write_text("")
        (item / "the browser").write_text('#!/bin/sh\nprintf "%s|" started "$@"\n')
        (item / "the browser").chmod(0o755)

    def run_launcher(self, launcher, *args):
        return subprocess.run([str(launcher), *args], text=True, capture_output=True, timeout=30)

    def test_the_item_is_found_from_the_tools_directory_a_version_and_a_staging_directory(self):
        # The last two: a tools directory that is itself in a directory called staging or versions.
        for root, *inside in (("a",), ("b", "versions", "ext-core", "1.0.0-0123456789ab"), ("c", "staging", "ext-core-0123456789ab-1"),
                              ("staging/d",), ("versions/ext-core/e",)):
            with self.subTest(root=root, inside=inside):
                tools, launcher = self.install(root, *inside)
                missing = self.run_launcher(launcher, "--version")
                self.assertEqual(missing.returncode, 127)
                self.assertEqual(missing.stdout, "")
                self.assertRegex(missing.stderr, r"^browse: The Browser is not downloaded on this node yet: .*buckyos ext-tools download ext-core browser\n$")
                self.download(tools)
                found = self.run_launcher(launcher, "--flag", "two words")
                self.assertEqual((found.returncode, found.stdout, found.stderr), (0, "started|--flag|two words|", ""))
                if inside:
                    # The launcher a node runs is a copy in <tools>/bin, also reached through a link.
                    (tools / "bin").mkdir()
                    shutil.copy2(launcher, tools / "bin/browse")
                    (tools / "browse-link").symlink_to("bin/browse")
                    for entry in (tools / "bin/browse", tools / "browse-link"):
                        self.assertEqual(self.run_launcher(entry).stdout, "started|")


if __name__ == "__main__":
    unittest.main()
