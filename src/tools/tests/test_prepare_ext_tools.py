import hashlib
import importlib.util
import json
import tarfile
import tempfile
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location("prepare_ext_tools", Path(__file__).resolve().parent.parent / "prepare_ext_tools.py")
tool = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(tool)


class PrepareExtToolsTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.out = self.root / "out"

    def payload(self, version="1.2.3"):
        payload = self.root / "payload"
        (payload / "bin").mkdir(parents=True)
        (payload / "bin" / "tool").write_text("#!/bin/sh\n")
        (payload / "ext-manifest.json").write_text(json.dumps({"version": version, "platforms": ["linux-x86_64"]}))
        return payload

    def test_a_local_payload_becomes_what_node_daemon_seeds_from(self):
        tool.from_local(self.out, self.payload(), "linux-x86_64")
        pin = json.loads((self.out / "ext_tools.json").read_text())
        self.assertEqual(pin["schema"], "buckyos.ext-tools/v1")
        entry = pin["packages"]["ext-core"]["platforms"]["linux-x86_64"]
        tar = self.out / entry["file"]
        self.assertEqual(entry["sha256"], "sha256:" + hashlib.sha256(tar.read_bytes()).hexdigest())
        with tarfile.open(tar) as archive:
            self.assertEqual(sorted(archive.getnames()), ["bin", "bin/tool", "ext-manifest.json"])
        self.assertTrue(tool.ready(self.out, "1.2.3", "linux-x86_64"))

    def test_ready_means_the_right_version_and_the_whole_file(self):
        self.assertFalse(tool.ready(self.out, "1.2.3", "linux-x86_64"), "nothing there")
        tool.from_local(self.out, self.payload(), "linux-x86_64")
        self.assertFalse(tool.ready(self.out, "9.9.9", "linux-x86_64"), "another version is pinned")
        self.assertFalse(tool.ready(self.out, "1.2.3", "windows-x86_64"), "another platform")
        self.assertTrue(tool.ready(self.out, None, "linux-x86_64"))
        tar = next(self.out.glob("*.tar.gz"))
        tar.write_bytes(tar.read_bytes()[:10])
        self.assertFalse(tool.ready(self.out, "1.2.3", "linux-x86_64"), "cut short")

    def test_a_release_is_downloaded_once_and_checked(self):
        source = self.root / "ext-core-1.2.3-linux-x86_64.tar.gz"
        source.write_bytes(b"payload")
        sha = "sha256:" + hashlib.sha256(b"payload").hexdigest()
        release = {"schema": "buckyos.ext-tools/v1", "packages": {"ext-core": {"version": "1.2.3", "pikg_digest": "sha256:pikg", "platforms": {
            "linux-x86_64": {"file": source.name, "sha256": sha, "size": 7, "path": "x.tar.gz"},
            "darwin-arm64": {"file": "other", "sha256": "sha256:0", "size": 1, "path": "y"},
        }}}}
        tool.fetch_json = lambda url: release
        downloads = []

        def download(url, dest, size, sha256):
            downloads.append(url)
            dest.write_bytes(b"payload")

        tool.download = download
        tool.pinned_version = lambda: "1.2.3"
        self.assertEqual(tool.main(["--out", str(self.out), "--platform", "linux-x86_64"]), 0)
        self.assertEqual(tool.main(["--out", str(self.out), "--platform", "linux-x86_64"]), 0)
        self.assertEqual(len(downloads), 1, "no download when the file is there")
        self.assertEqual(tool.main(["--out", str(self.out), "--platform", "linux-x86_64", "--force"]), 0)
        self.assertEqual(len(downloads), 2)
        pin = json.loads((self.out / "ext_tools.json").read_text())["packages"]["ext-core"]
        self.assertEqual(list(pin["platforms"]), ["linux-x86_64"], "the pin only says what is in the directory")
        self.assertEqual(pin["pikg_digest"], "sha256:pikg")


if __name__ == "__main__":
    unittest.main()
