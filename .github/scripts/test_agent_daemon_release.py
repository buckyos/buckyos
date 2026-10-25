import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
import unittest.mock

SCRIPT = Path(__file__).with_name("agent_daemon_release.py")
spec = importlib.util.spec_from_file_location("agent_daemon_release", SCRIPT)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
packager = module.load_packager()
VERSION = packager.version()
REPO = "https://github.com/buckyos/buckyos"


def fake_dist(root: Path, targets=None) -> Path:
    dist = root / "dist"
    dist.mkdir()
    for target in targets or packager.TARGETS:
        archive = dist / packager.archive_name(target)
        archive.write_bytes(f"payload for {target}".encode())
        archive.with_name(archive.name + ".sha256").write_text(
            f"{hashlib.sha256(archive.read_bytes()).hexdigest()}  {archive.name}\n")
    (dist / module.LAUNCHER).write_text("#!/bin/sh\n")
    return dist


class ReleaseAssemblyTests(unittest.TestCase):
    def test_versioned_and_rolling_payloads_agree(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            dist = fake_dist(root)
            result = module.assemble(dist, root / "out", repo_url=REPO, commit="abc123", tag=f"agent-daemon-v{VERSION}", allow_missing=False)
            self.assertEqual(result["tag"], f"agent-daemon-v{VERSION}")
            self.assertEqual(result["targets"], sorted(packager.TARGETS))
            versioned = root / "out/versioned/assets"
            latest = root / "out/latest/assets"
            for target in packager.TARGETS:
                name = packager.archive_name(target)
                stable = name.replace(f"-{VERSION}-", "-", 1)
                self.assertNotIn(VERSION, stable)
                self.assertEqual((versioned / name).read_bytes(), (dist / name).read_bytes())
                self.assertEqual((latest / stable).read_bytes(), (dist / name).read_bytes())
                self.assertEqual((latest / (stable + ".sha256")).read_text().split()[1], stable)
            manifest = json.loads((latest / "latest.json").read_text())
            self.assertEqual(manifest["version"], VERSION)
            self.assertEqual(manifest["commit"], "abc123")
            self.assertEqual(manifest, json.loads((versioned / "latest.json").read_text()))
            linux = manifest["assets"]["x86_64-unknown-linux-musl"]
            self.assertEqual(linux["url"], f"{REPO}/releases/download/agent-daemon-v{VERSION}/{packager.archive_name('x86_64-unknown-linux-musl')}")
            self.assertEqual(linux["stable_url"], f"{REPO}/releases/download/agent-daemon-latest/buckyos-agent-daemon-x86_64-unknown-linux-musl.tar.gz")
            self.assertEqual(linux["sha256"], hashlib.sha256(b"payload for x86_64-unknown-linux-musl").hexdigest())
            # The launcher parses the version with sed from a single line.
            self.assertIn(f'  "version": "{VERSION}",', (latest / "latest.json").read_text().splitlines())
            for directory in (versioned, latest):
                sums = dict(line.split()[::-1] for line in (directory / "SHA256SUMS").read_text().splitlines())
                self.assertIn(module.LAUNCHER, sums)
                self.assertIn("latest.json", sums)
                for name, digest in sums.items():
                    self.assertEqual(hashlib.sha256((directory / name).read_bytes()).hexdigest(), digest)
                self.assertTrue((directory / module.LAUNCHER).stat().st_mode & 0o100)
            notes = (root / "out/versioned/notes.md").read_text()
            self.assertIn(f"# BuckyOS Agent Daemon {VERSION}", notes)
            self.assertIn("Windows ARM64", notes)
            self.assertIn("not code-signed", notes)
            self.assertIn("agent-daemon-latest", (root / "out/latest/notes.md").read_text())

    def test_tag_must_match_source_version(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            dist = fake_dist(root)
            with self.assertRaisesRegex(ValueError, "does not match src/VERSION"):
                module.assemble(dist, root / "out", repo_url=REPO, commit="abc", tag="agent-daemon-v0.0.1", allow_missing=False)
            self.assertFalse((root / "out").exists())

    def test_missing_target_fails_unless_allowed(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            dist = fake_dist(root, targets=["aarch64-apple-darwin"])
            with self.assertRaisesRegex(ValueError, "missing release archives"):
                module.assemble(dist, root / "out", repo_url=REPO, commit="abc", tag=None, allow_missing=False)
            result = module.assemble(dist, root / "out", repo_url=REPO, commit="abc", tag=None, allow_missing=True)
            self.assertEqual(result["targets"], ["aarch64-apple-darwin"])
            manifest = json.loads((root / "out/latest/assets/latest.json").read_text())
            self.assertEqual(list(manifest["assets"]), ["aarch64-apple-darwin"])

    def test_corrupt_checksum_or_stray_file_is_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            dist = fake_dist(root)
            archive = dist / packager.archive_name("x86_64-pc-windows-msvc")
            archive.write_bytes(b"tampered")
            with self.assertRaisesRegex(ValueError, "checksum mismatch"):
                module.assemble(dist, root / "out", repo_url=REPO, commit="abc", tag=None, allow_missing=False)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            dist = fake_dist(root)
            (dist / "unpacked.bin").write_bytes(b"x")
            with self.assertRaisesRegex(ValueError, "unexpected files"):
                module.assemble(dist, root / "out", repo_url=REPO, commit="abc", tag=None, allow_missing=False)

    def test_main_writes_github_outputs(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            dist = fake_dist(root)
            output_file = root / "github_output"
            with unittest.mock.patch.dict(module.os.environ, {"GITHUB_OUTPUT": str(output_file)}):
                self.assertEqual(module.main(["--dist", str(dist), "--output", str(root / "out"), "--commit", "abc"]), 0)
            self.assertIn(f"version={VERSION}\n", output_file.read_text())
            self.assertIn(f"tag=agent-daemon-v{VERSION}\n", output_file.read_text())
            self.assertIn("latest_tag=agent-daemon-latest\n", output_file.read_text())


if __name__ == "__main__":
    unittest.main()
