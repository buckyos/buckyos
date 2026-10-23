import hashlib
import io
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest

LAUNCHER = Path(__file__).resolve().parents[2] / "src/publish/agent_daemon/start-buckyos-agent-daemon.sh"


class LauncherTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.archive = self.root / "release.tar.gz"
        with tarfile.open(self.archive, "w:gz") as archive:
            for name, value in {
                "buckyos_agent_daemon": b'#!/bin/sh\nprintf "%s\\n" "$$" "$1"\n',
                "manifest.json": b'{}',
                "service.sh": b'#!/bin/sh\nexit 0\n',
            }.items():
                item = tarfile.TarInfo(name)
                item.size = len(value)
                item.mode = 0o700
                archive.addfile(item, io.BytesIO(value))
        self.digest = hashlib.sha256(self.archive.read_bytes()).hexdigest()
        self.env = {**os.environ, "BUCKYOS_AGENT_CACHE_DIR": str(self.root / "cache")}

    def run_launcher(self, *args, digest=None):
        return subprocess.run(["sh", str(LAUNCHER), "--artifact", str(self.archive), "--sha256", digest or self.digest, *args],
                              env=self.env, capture_output=True, text=True)

    def test_predownload_and_foreground_url_do_not_print_secret(self):
        result = self.run_launcher("--download-only")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(Path(result.stdout.strip()).is_file())
        result = self.run_launcher("wss://zone.invalid/manage#top-secret")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.splitlines()[1], "run")
        self.assertNotIn("top-secret", result.stdout + result.stderr)

    def test_checksum_mismatch_prevents_execution(self):
        result = self.run_launcher("doctor", digest="0" * 64)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("checksum mismatch", result.stderr)
        self.assertEqual(result.stdout, "")

    def test_missing_url_does_not_download_or_start(self):
        result = subprocess.run(["sh", str(LAUNCHER)], env=self.env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 2)
        self.assertIn("Usage:", result.stdout)
        self.assertFalse((self.root / "cache").exists())


if __name__ == "__main__":
    unittest.main()
