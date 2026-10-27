import hashlib
import importlib.util
import io
import json
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("agent_daemon_verify.py")
spec = importlib.util.spec_from_file_location("agent_daemon_verify", SCRIPT)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
packager = module.load_packager()
VERSION = packager.source_version() + "+build261026"
RUNTIME_VERSION = VERSION + ".main.abc123 (nightly)"
TARGET = "x86_64-unknown-linux-musl"


def fake_dist(root: Path, cli_version=RUNTIME_VERSION, doctor_version=RUNTIME_VERSION) -> Path:
    dist = root / "dist"
    dist.mkdir()
    archive = dist / packager.archive_name(TARGET, VERSION)
    directory = archive.name.removesuffix(".tar.gz")
    doctor = json.dumps({"version": doctor_version, "identity": "current_user"})
    binary = (
        "#!/bin/sh\n"
        "if [ \"$1\" = '--version' ]; then\n"
        f"  printf '%s\\n' 'buckyos_agent_daemon {cli_version}'\n"
        "else\n"
        f"  printf '%s\\n' '{doctor}'\n"
        "fi\n"
    ).encode()
    manifest = {
        "version": VERSION,
        "target": TARGET,
        "platform": "linux",
        "binary": "buckyos_agent_daemon",
        "sha256": hashlib.sha256(binary).hexdigest(),
        "files": ["buckyos_agent_daemon", "manifest.json"],
    }
    with tarfile.open(archive, "w:gz") as tar:
        for name, content in {"buckyos_agent_daemon": binary, "manifest.json": json.dumps(manifest).encode()}.items():
            member = tarfile.TarInfo(f"{directory}/{name}")
            member.size = len(content)
            member.mode = 0o755 if name == "buckyos_agent_daemon" else 0o644
            tar.addfile(member, io.BytesIO(content))
    archive.with_name(archive.name + ".sha256").write_text(
        f"{hashlib.sha256(archive.read_bytes()).hexdigest()}  {archive.name}\n")
    (dist / "start-buckyos-agent-daemon.sh").write_text("#!/bin/sh\n")
    return dist


@unittest.skipUnless(sys.platform.startswith("linux"), "fixture uses a Linux shell executable")
class AgentDaemonVerifyTests(unittest.TestCase):
    def test_runtime_version_metadata_is_normalized_to_archive_version(self):
        with tempfile.TemporaryDirectory() as temporary:
            dist = fake_dist(Path(temporary))
            result = module.verify(dist, TARGET)
            self.assertEqual(result["archive"], packager.archive_name(TARGET, VERSION))
            self.assertEqual(result["doctor"]["identity"], "current_user")

    def test_binary_build_date_must_match_manifest(self):
        with tempfile.TemporaryDirectory() as temporary:
            dist = fake_dist(Path(temporary), cli_version=packager.source_version() + "+build261027 (nightly)")
            with self.assertRaisesRegex(ValueError, "binary version does not match archive version"):
                module.verify(dist, TARGET)

    def test_doctor_build_date_must_match_manifest(self):
        with tempfile.TemporaryDirectory() as temporary:
            dist = fake_dist(Path(temporary), doctor_version=packager.source_version() + "+build261027 (nightly)")
            with self.assertRaisesRegex(ValueError, "doctor reports version"):
                module.verify(dist, TARGET)

    def test_doctor_base_version_without_build_is_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            dist = fake_dist(Path(temporary), doctor_version=packager.source_version())
            with self.assertRaisesRegex(ValueError, "cannot parse BuckyOS build version"):
                module.verify(dist, TARGET)


if __name__ == "__main__":
    unittest.main()
