import hashlib
import io
import json
import os
from pathlib import Path
import platform
import subprocess
import tarfile
import tempfile
import unittest

LAUNCHER = Path(__file__).resolve().parents[2] / "src/rootfs/bin/buckyos-agent-daemon/start-buckyos-agent-daemon.sh"
# The launcher picks the target from uname -m; the test archive must be named for this host.
TARGET = {"x86_64": "x86_64-unknown-linux-musl", "aarch64": "aarch64-unknown-linux-gnu"}[platform.machine()]


def release_archive(path: Path, version: str = "9.9.9") -> str:
    """Write a release-shaped archive: one directory holding the daemon, manifest and service script."""
    root = f"buckyos-agent-daemon-{version}-{TARGET}"
    with tarfile.open(path, "w:gz") as archive:
        directory = tarfile.TarInfo(root)
        directory.type = tarfile.DIRTYPE
        directory.mode = 0o755
        archive.addfile(directory)
        for name, value in {
            "buckyos_agent_daemon": b'#!/bin/sh\nprintf "%s\\n" "$$" "$1"\n',
            "manifest.json": json.dumps({"version": version, "target": TARGET}, indent=2).encode(),
            "service.sh": b'#!/bin/sh\nexit 0\n',
            "README.md": b"# test\n",
        }.items():
            item = tarfile.TarInfo(f"{root}/{name}")
            item.size = len(value)
            item.mode = 0o700
            archive.addfile(item, io.BytesIO(value))
    return hashlib.sha256(path.read_bytes()).hexdigest()


class LauncherTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.archive = self.root / "release.tar.gz"
        self.digest = release_archive(self.archive)
        self.env = {**os.environ, "BUCKYOS_AGENT_CACHE_DIR": str(self.root / "cache")}
        self.env.pop("BUCKYOS_AGENT_VERSION", None)

    def run_launcher(self, *args, digest=None, env=None):
        return subprocess.run(["sh", str(LAUNCHER), "--artifact", str(self.archive), "--sha256", digest or self.digest, *args],
                              env=env or self.env, capture_output=True, text=True)

    def test_predownload_and_foreground_url_do_not_print_secret(self):
        result = self.run_launcher("--download-only")
        self.assertEqual(result.returncode, 0, result.stderr)
        binary = Path(result.stdout.strip())
        self.assertTrue(binary.is_file())
        self.assertEqual(binary.parent.name, f"9.9.9-{TARGET}-{self.digest}")
        self.assertTrue((binary.parent / "service.sh").is_file())
        result = self.run_launcher("wss://zone.invalid/manage#top-secret")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.splitlines()[1], "run")
        self.assertNotIn("top-secret", result.stdout + result.stderr)

    def test_checksum_mismatch_prevents_execution(self):
        result = self.run_launcher("doctor", digest="0" * 64)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("checksum mismatch", result.stderr)
        self.assertEqual(result.stdout, "")

    def test_requested_version_must_match_the_archive(self):
        result = self.run_launcher("--download-only", env={**self.env, "BUCKYOS_AGENT_VERSION": "1.0.0"})
        self.assertEqual(result.returncode, 1)
        self.assertIn("does not match", result.stderr)
        self.assertFalse(list((self.root / "cache").glob("9.9.9-*")))

    def test_build_version_is_preserved_in_the_download_cache(self):
        version = "0.7.0+build261026"
        digest = release_archive(self.archive, version=version)
        result = self.run_launcher("--download-only", digest=digest,
                                   env={**self.env, "BUCKYOS_AGENT_VERSION": version})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(Path(result.stdout.strip()).parent.name, f"{version}-{TARGET}-{digest}")

    def test_archive_without_daemon_is_rejected(self):
        with tarfile.open(self.archive, "w:gz") as archive:
            item = tarfile.TarInfo(f"buckyos-agent-daemon-9.9.9-{TARGET}/manifest.json")
            item.size = 2
            archive.addfile(item, io.BytesIO(b"{}"))
        result = self.run_launcher("--download-only", digest=hashlib.sha256(self.archive.read_bytes()).hexdigest())
        self.assertEqual(result.returncode, 1)
        self.assertIn("lacks buckyos_agent_daemon", result.stderr)

    def test_missing_url_does_not_download_or_start(self):
        result = subprocess.run(["sh", str(LAUNCHER)], env=self.env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 2)
        self.assertIn("Usage:", result.stdout)
        self.assertFalse((self.root / "cache").exists())


if __name__ == "__main__":
    unittest.main()
