import hashlib
import importlib.util
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
import zipfile
from unittest.mock import patch


SCRIPT = Path(__file__).parents[1] / "package_agent_daemon.py"
spec = importlib.util.spec_from_file_location("package_agent_daemon", SCRIPT)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class AgentDaemonPackageTests(unittest.TestCase):
    def test_windows_archive_contains_executable_and_matching_checksums(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / "daemon.exe"
            binary.write_bytes(b"test Windows executable payload")
            for target in ("x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc"):
                with self.subTest(target=target):
                    archive = module.package(binary, target, root / "dist")
                    self.assertEqual(archive.suffix, ".zip")
                    checksum = archive.with_name(archive.name + ".sha256").read_text().split()[0]
                    self.assertEqual(checksum, hashlib.sha256(archive.read_bytes()).hexdigest())
                    with zipfile.ZipFile(archive) as zipped:
                        names = zipped.namelist()
                        executable = next(name for name in names if name.endswith("/buckyos_agent_daemon.exe"))
                        self.assertEqual(zipped.read(executable), binary.read_bytes())
                        manifest = json.loads(zipped.read(next(name for name in names if name.endswith("/manifest.json"))))
                        self.assertEqual(manifest["target"], target)
                        self.assertEqual(manifest["platform"], "windows")
                        self.assertEqual(manifest["binary"], "buckyos_agent_daemon.exe")
                        self.assertEqual(manifest["requirements"]["shell"], "powershell.exe")
                        self.assertEqual(manifest["requirements"]["windows_minimum"], "10.0.15063")
                        self.assertEqual(manifest["requirements"]["windows_crt"], "prebuilt_unverified")
                        self.assertIn("Visual C++ v14", manifest["requirements"]["visual_cpp_runtime"])
                        self.assertTrue(manifest["requirements"]["interactive_user_session"])
                        self.assertTrue(manifest["foreground"])
                        self.assertEqual(manifest["identity_mode"], "current_user")
                        self.assertEqual(manifest["sha256"], hashlib.sha256(binary.read_bytes()).hexdigest())
                        self.assertTrue(any(name.endswith("/README.md") for name in names))
                        self.assertTrue(all(not Path(name).is_absolute() and ".." not in Path(name).parts for name in names))

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
            for target in ("aarch64-apple-darwin", "x86_64-pc-windows-msvc"):
                with self.subTest(target=target), patch.object(module.subprocess, "run") as run:
                    self.assertEqual(module.main(["--target", target, "--binary", str(binary), "--output", str(root / "dist")]), 0)
                    run.assert_not_called()

    def test_windows_build_preserves_flags_and_requests_static_crt(self):
        encoded = "-C\x1flink-arg=/LIBPATH:C:\\SDK Path\\lib"
        raw = '-L C:\\SDK\\lib --cfg=feature="test"'
        for environment, expected in (
            ({}, ""),
            ({"RUSTFLAGS": raw}, "\x1f".join(raw.split())),
            ({"CARGO_ENCODED_RUSTFLAGS": encoded, "RUSTFLAGS": "--cfg=ignored"}, encoded),
            ({"CARGO_ENCODED_RUSTFLAGS": "", "RUSTFLAGS": "--cfg=ignored"}, ""),
        ):
            with self.subTest(environment=environment):
                results = [
                    module.subprocess.CompletedProcess([], 0),
                    module.subprocess.CompletedProcess([], 0, stdout=json.dumps({"target_directory": "custom-target"})),
                ]
                with (
                    patch.dict(module.os.environ, environment, clear=True),
                    patch.object(module.subprocess, "run", side_effect=results) as run,
                    patch.object(module, "package", return_value=Path("dist/daemon.zip")) as package,
                ):
                    self.assertEqual(module.main(["--target", "x86_64-pc-windows-msvc", "--output", "dist"]), 0)
                    build_env = run.call_args_list[0].kwargs["env"]
                    self.assertEqual(build_env["CARGO_ENCODED_RUSTFLAGS"], (expected + "\x1f" if expected else "") + "-C\x1ftarget-feature=+crt-static")
                    self.assertEqual(dict(module.os.environ), environment)
                    self.assertTrue(package.call_args.kwargs["static_crt"])

    def test_build_uses_the_configured_cargo_target_directory(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for target, filename, pattern in (
                ("aarch64-apple-darwin", "buckyos_agent_daemon", "*.tar.gz"),
                ("x86_64-pc-windows-msvc", "buckyos_agent_daemon.exe", "*.zip"),
                ("aarch64-pc-windows-msvc", "buckyos_agent_daemon.exe", "*.zip"),
            ):
                with self.subTest(target=target):
                    binary = root / "custom-target" / target / "release" / filename
                    binary.parent.mkdir(parents=True)
                    binary.write_bytes(b"test executable payload")
                    results = [
                        module.subprocess.CompletedProcess([], 0),
                        module.subprocess.CompletedProcess([], 0, stdout=json.dumps({"target_directory": str(root / "custom-target")})),
                    ]
                    output = root / "dist" / target
                    with patch.object(module.subprocess, "run", side_effect=results):
                        self.assertEqual(module.main(["--target", target, "--output", str(output)]), 0)
                        self.assertEqual(len(list(output.glob(pattern))), 1)
                        if filename.endswith(".exe"):
                            with zipfile.ZipFile(next(output.glob(pattern))) as zipped:
                                manifest = json.loads(zipped.read(next(name for name in zipped.namelist() if name.endswith("/manifest.json"))))
                                self.assertEqual(manifest["requirements"]["windows_crt"], "static")
                                self.assertIsNone(manifest["requirements"]["visual_cpp_runtime"])


if __name__ == "__main__":
    unittest.main()
