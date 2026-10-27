import hashlib
import importlib.util
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
import unittest.mock
import zipfile

SCRIPT = Path(__file__).with_name("agent_daemon_release.py")
spec = importlib.util.spec_from_file_location("agent_daemon_release", SCRIPT)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
packager = module.load_packager()
VERSION = packager.source_version() + "+build261026"
REPO = "https://github.com/buckyos/buckyos"


def write_archive(dist: Path, target: str, version: str = VERSION, manifest=None) -> Path:
    archive = dist / packager.archive_name(target, version)
    directory = archive.name.removesuffix(".zip").removesuffix(".tar.gz")
    payload = json.dumps(manifest if manifest is not None else {"version": version, "target": target}).encode()
    name = f"{directory}/manifest.json"
    if archive.suffix == ".zip":
        with zipfile.ZipFile(archive, "w") as zipped:
            zipped.writestr(name, payload)
    else:
        with tarfile.open(archive, "w:gz") as tar:
            member = tarfile.TarInfo(name)
            member.size = len(payload)
            tar.addfile(member, io.BytesIO(payload))
    archive.with_name(archive.name + ".sha256").write_text(
        f"{hashlib.sha256(archive.read_bytes()).hexdigest()}  {archive.name}\n")
    return archive


def fake_dist(root: Path, targets=None, version: str = VERSION) -> Path:
    dist = root / "dist"
    dist.mkdir()
    for target in targets or packager.TARGETS:
        write_archive(dist, target, version)
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
                name = packager.archive_name(target, VERSION)
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
            self.assertEqual(linux["url"], f"{REPO}/releases/download/agent-daemon-v{VERSION}/{packager.archive_name('x86_64-unknown-linux-musl', VERSION)}")
            self.assertEqual(linux["stable_url"], f"{REPO}/releases/download/agent-daemon-latest/buckyos-agent-daemon-x86_64-unknown-linux-musl.tar.gz")
            self.assertEqual(linux["sha256"], hashlib.sha256((dist / linux["file"]).read_bytes()).hexdigest())
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

    def test_tag_must_match_archive_version(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            dist = fake_dist(root)
            for tag in ("agent-daemon-v0.0.1", f"agent-daemon-v{packager.source_version()}"):
                with self.subTest(tag=tag), self.assertRaisesRegex(ValueError, "does not match archive version"):
                    module.assemble(dist, root / "out", repo_url=REPO, commit="abc", tag=tag, allow_missing=False)
            self.assertFalse((root / "out").exists())

    def test_mixed_build_dates_are_refused_even_when_missing_targets_are_allowed(self):
        for allow_missing in (False, True):
            with self.subTest(allow_missing=allow_missing), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                dist = fake_dist(root)
                target = "aarch64-apple-darwin"
                archive = dist / packager.archive_name(target, VERSION)
                archive.unlink()
                archive.with_name(archive.name + ".sha256").unlink()
                write_archive(dist, target, packager.source_version() + "+build261027")
                with self.assertRaisesRegex(ValueError, "different versions"):
                    module.assemble(dist, root / "out", repo_url=REPO, commit="abc", tag=None, allow_missing=allow_missing)
                self.assertFalse((root / "out").exists())

    def test_archive_base_version_must_match_source(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            dist = fake_dist(root, version="999.0.0+build261026")
            with self.assertRaisesRegex(ValueError, "does not match src/VERSION"):
                module.assemble(dist, root / "out", repo_url=REPO, commit="abc", tag=None, allow_missing=False)

    def test_manifest_must_match_archive_version_and_target(self):
        for target in ("x86_64-unknown-linux-musl", "x86_64-pc-windows-msvc"):
            for manifest in (
                {"version": packager.source_version() + "+build261027", "target": target},
                {"version": VERSION, "target": "aarch64-apple-darwin"},
                {"version": VERSION + ".main.abc (nightly)", "target": target},
                {"version": VERSION},
            ):
                with self.subTest(target=target, manifest=manifest), tempfile.TemporaryDirectory() as temporary:
                    root = Path(temporary)
                    dist = fake_dist(root)
                    write_archive(dist, target, manifest=manifest)
                    with self.assertRaisesRegex(ValueError, "manifest"):
                        module.assemble(dist, root / "out", repo_url=REPO, commit="abc", tag=None, allow_missing=False)

    def test_multiple_archives_for_one_target_are_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            dist = fake_dist(root)
            write_archive(dist, "aarch64-apple-darwin", packager.source_version() + "+build261027")
            with self.assertRaisesRegex(ValueError, "multiple release archives"):
                module.assemble(dist, root / "out", repo_url=REPO, commit="abc", tag=None, allow_missing=True)

    def test_publishing_an_older_build_preserves_its_version(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            version = packager.source_version() + "+build240101"
            dist = fake_dist(root, version=version)
            result = module.assemble(dist, root / "out", repo_url=REPO, commit="abc", tag=None, allow_missing=False)
            self.assertEqual(result["version"], version)
            self.assertEqual(result["tag"], f"agent-daemon-v{version}")
            manifest = json.loads((root / "out/latest/assets/latest.json").read_text())
            self.assertEqual(manifest["version"], version)
            for asset in manifest["assets"].values():
                self.assertIn(version, asset["url"])

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
            archive = dist / packager.archive_name("x86_64-pc-windows-msvc", VERSION)
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
