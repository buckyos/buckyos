import hashlib
import json
import os
from pathlib import Path
import stat
import sys
import tempfile
import unittest
import zipfile

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import place_online


def member(name, mode):
    info = zipfile.ZipInfo(name)
    info.create_system = 3
    info.external_attr = mode << 16
    return info


class PlaceOnlineTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.payload = self.root / "payload"
        self.tools = self.root / "tools"
        self.archive = self.root / "browser.zip"
        with zipfile.ZipFile(self.archive, "w") as handle:
            handle.writestr(member("browser-linux/browser", stat.S_IFREG | 0o755), "the binary")
            handle.writestr(member("browser-linux/data.pak", stat.S_IFREG | 0o644), "data")
            handle.writestr(member("browser-linux/current", stat.S_IFLNK | 0o777), "data.pak")
        layer = self.payload / "online/browser/browser-7"
        (layer / "browser-linux").mkdir(parents=True)
        (layer / "COMPLETE").write_text("")
        (layer / "browser-linux/browser").write_text("#!/bin/sh\n")
        (layer / "browser-linux/browser").chmod(0o755)
        self.entry = {
            "url": "https://upstream.invalid/browser.zip", "sha256": hashlib.sha256(self.archive.read_bytes()).hexdigest(),
            "size_bytes": self.archive.stat().st_size, "format": "zip", "into": "browser-7",
            "rename": [["browser-7/browser-linux/browser", "browser-7/browser-linux/browser-bin"]], "overlay": "online/browser",
        }
        self.write_manifest()

    def write_manifest(self):
        (self.payload / "ext-manifest.json").write_text(json.dumps({
            "id": "ext-test", "online": [{"id": "browser", "version": "7.0", "platforms": {"linux-x86_64": self.entry}}],
        }))

    def place(self, platform="linux-x86_64"):
        return place_online.place(self.payload, self.tools, platform, "browser", self.archive)

    def test_an_item_is_unpacked_renamed_and_overlaid_as_the_manifest_says(self):
        record = self.place()
        item = self.tools / "online/browser/browser-7"
        self.assertEqual((item / "browser-linux/browser-bin").read_text(), "the binary")
        self.assertEqual((item / "browser-linux/browser").read_text(), "#!/bin/sh\n")
        self.assertTrue((item / "COMPLETE").is_file())
        if os.name != "nt":
            self.assertTrue(os.access(item / "browser-linux/browser-bin", os.X_OK))
            self.assertTrue(os.access(item / "browser-linux/browser", os.X_OK))
            self.assertFalse(os.access(item / "browser-linux/data.pak", os.X_OK))
            self.assertEqual(os.readlink(item / "browser-linux/current"), "data.pak")
        self.assertEqual(json.loads((self.tools / "online/browser.json").read_text()), record)
        self.assertEqual((record["schema"], record["state"], record["version"], record["package"], record["sha256"]),
                         ("buckyos.ext_tools.online/1", "ready", "7.0", "ext-test", self.entry["sha256"]))
        self.assertEqual(sorted(p.name for p in (self.tools / "online").iterdir()), ["browser", "browser.json"])
        self.place()
        self.assertEqual((item / "browser-linux/browser-bin").read_text(), "the binary")

    def test_a_platform_without_an_entry_has_no_such_item(self):
        self.assertIsNone(self.place("windows-x86_64"))
        self.assertFalse((self.tools / "online/browser").exists())

    def test_an_archive_that_is_not_the_pinned_one_leaves_nothing(self):
        for field, value in (("sha256", "0" * 64), ("size_bytes", 1)):
            with self.subTest(field=field):
                pinned = self.entry[field]
                self.entry[field] = value
                self.write_manifest()
                with self.assertRaises(place_online.PlaceError):
                    self.place()
                self.assertEqual(list((self.tools / "online").iterdir()), [])
                self.entry[field] = pinned

    def test_nothing_is_written_outside_the_item(self):
        for name, mode, text in (("../outside", stat.S_IFREG | 0o644, "x"), ("/abs", stat.S_IFREG | 0o644, "x"),
                                 ("browser-linux/out", stat.S_IFLNK | 0o777, "../../../../outside")):
            with self.subTest(name=name):
                with zipfile.ZipFile(self.archive, "w") as handle:
                    handle.writestr(member(name, mode), text)
                self.entry.update(sha256=hashlib.sha256(self.archive.read_bytes()).hexdigest(), size_bytes=self.archive.stat().st_size)
                self.write_manifest()
                with self.assertRaises(place_online.PlaceError):
                    self.place()
                self.assertFalse((self.root / "outside").exists())
                self.assertEqual(list((self.tools / "online").iterdir()), [])
        for field, value in (("into", "../browser-7"), ("overlay", "/etc"), ("rename", [["browser-7/a", "../b"]])):
            with self.subTest(field=field):
                kept = self.entry[field]
                self.entry[field] = value
                self.write_manifest()
                with self.assertRaises(place_online.PlaceError):
                    self.place()
                self.entry[field] = kept

    def test_a_payload_unpacked_as_the_tools_directory_is_refused(self):
        with self.assertRaisesRegex(place_online.PlaceError, "unpacked as the tools directory"):
            place_online.place(self.payload, self.payload, "linux-x86_64", "browser", self.archive)
        self.assertTrue((self.payload / "online/browser/browser-7/COMPLETE").is_file())


if __name__ == "__main__":
    unittest.main()
