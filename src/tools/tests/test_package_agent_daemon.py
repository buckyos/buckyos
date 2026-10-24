import hashlib
import importlib.util
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).parents[1] / "package_agent_daemon.py"
spec = importlib.util.spec_from_file_location("package_agent_daemon", SCRIPT)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class AgentDaemonPackageTests(unittest.TestCase):
    def test_macos_archive_contains_executable_and_matching_checksums(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / "daemon"
            binary.write_bytes(b"test executable payload")
            for target in ("aarch64-apple-darwin", "x86_64-apple-darwin"):
                archive = module.package(binary, target, root / "dist")
                checksum = archive.with_name(archive.name + ".sha256").read_text().split()[0]
                self.assertEqual(checksum, hashlib.sha256(archive.read_bytes()).hexdigest())
                with tarfile.open(archive) as tar:
                    members = tar.getmembers()
                    executable = next(member for member in members if member.name.endswith("/buckyos_agent_daemon"))
                    self.assertEqual(executable.mode, 0o755)
                    self.assertEqual(tar.extractfile(executable).read(), binary.read_bytes())
                    manifest_member = next(member for member in members if member.name.endswith("/manifest.json"))
                    manifest = json.load(tar.extractfile(manifest_member))
                    self.assertEqual(manifest["target"], target)
                    self.assertEqual(manifest["requirements"]["macos_minimum"], "14.0")
                    self.assertEqual(manifest["sha256"], hashlib.sha256(binary.read_bytes()).hexdigest())
                    self.assertTrue(all(not member.issym() and not Path(member.name).is_absolute() and ".." not in Path(member.name).parts for member in members))

    def test_missing_binary_does_not_create_an_archive(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with self.assertRaises(ValueError):
                module.package(root / "missing", "aarch64-apple-darwin", root / "dist")
            self.assertFalse((root / "dist").exists())

    def test_explicit_binary_does_not_build(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / "daemon"
            binary.write_bytes(b"test executable payload")
            with patch.object(module.subprocess, "run") as run:
                self.assertEqual(module.main(["--target", "aarch64-apple-darwin", "--binary", str(binary), "--output", str(root / "dist")]), 0)
                run.assert_not_called()

    def test_build_uses_the_configured_cargo_target_directory(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            target = "aarch64-apple-darwin"
            binary = root / "custom-target" / target / "release/buckyos_agent_daemon"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"test executable payload")
            results = [
                module.subprocess.CompletedProcess([], 0),
                module.subprocess.CompletedProcess([], 0, stdout=json.dumps({"target_directory": str(root / "custom-target")})),
            ]
            with patch.object(module.subprocess, "run", side_effect=results):
                self.assertEqual(module.main(["--target", target, "--output", str(root / "dist")]), 0)
                self.assertEqual(len(list((root / "dist").glob("*.tar.gz"))), 1)


if __name__ == "__main__":
    unittest.main()
