import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import stat
import tempfile
import unittest
from unittest.mock import patch


SCRIPT_PATH = Path(__file__).parents[2] / "rootfs" / "etc" / "backup.py"


def _load_backup_script():
    spec = importlib.util.spec_from_file_location("buckyos_identity_backup", SCRIPT_PATH)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class IdentityBackupTests(unittest.TestCase):
    def test_backup_and_restore_security_config_and_device_identity(self):
        module = _load_backup_script()
        for custom_roots in (False, True):
            with self.subTest(custom_roots=custom_roots), tempfile.TemporaryDirectory() as temporary:
                temporary = Path(temporary)
                root = temporary / "buckyos"
                security_root = temporary / "private" if custom_roots else root / "security"
                identity_root = temporary / "identities" if custom_roots else root / "local" / "identity"
                env = {"BUCKYOS_ROOT": str(root)}
                if custom_roots:
                    env["BUCKYOS_SECURITY_ROOT"] = str(security_root)
                    env["BUCKYOS_IDENTITY_ROOT"] = str(identity_root)

                config = {"zone_name": "test.buckyos.io", "admin_password_hash": "test-secret"}
                files = {
                    root / "etc" / "node_identity.json": {"device_did": "did:bns:ood1.test"},
                    root / "etc" / "machine.json": {"web3_bridge": {"bns": "web3.devtests.org"}},
                    security_root / "start_config.json": config,
                    identity_root / "device" / "did.json": {"id": "did:bns:ood1.test"},
                }
                for file, contents in files.items():
                    file.parent.mkdir(parents=True, exist_ok=True)
                    file.write_text(json.dumps(contents), encoding="utf-8")
                device_key = security_root / "device" / "authentication.pem"
                device_key.parent.mkdir(parents=True)
                device_key.write_text("test-private-key", encoding="utf-8")
                device_key.chmod(0o600)
                (security_root / "start_config.json").chmod(0o644)

                backup_root = temporary / "backup"
                with patch.dict(os.environ, env, clear=True), contextlib.redirect_stdout(io.StringIO()):
                    module.backup_identity_file(str(backup_root))
                    backup = backup_root / config["zone_name"]
                    saved_config = backup / "security" / "start_config.json"
                    self.assertEqual(json.loads(saved_config.read_text(encoding="utf-8")), config)
                    self.assertFalse((backup / "etc" / "start_config.json").exists())
                    if os.name != "nt":
                        self.assertEqual(stat.S_IMODE(saved_config.stat().st_mode), 0o600)

                    shutil.rmtree(root)
                    if custom_roots:
                        shutil.rmtree(security_root)
                        shutil.rmtree(identity_root)
                    saved_config.chmod(0o644)
                    module.restore_identity_file(str(backup_root), config["zone_name"])

                for file, contents in files.items():
                    self.assertEqual(json.loads(file.read_text(encoding="utf-8")), contents)
                self.assertEqual(device_key.read_text(encoding="utf-8"), "test-private-key")
                self.assertFalse((root / "etc" / "start_config.json").exists())
                if os.name != "nt":
                    self.assertEqual(stat.S_IMODE((security_root / "start_config.json").stat().st_mode), 0o600)
                    self.assertEqual(stat.S_IMODE(device_key.stat().st_mode), 0o600)

    def test_backup_requires_start_config_in_security_root(self):
        module = _load_backup_script()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            etc = root / "etc"
            etc.mkdir()
            (etc / "start_config.json").write_text('{"zone_name":"test.buckyos.io"}', encoding="utf-8")
            with (
                patch.dict(os.environ, {"BUCKYOS_ROOT": str(root)}, clear=True),
                contextlib.redirect_stdout(io.StringIO()),
                self.assertRaises(SystemExit) as error,
            ):
                module.backup_identity_file(str(root / "backup"))
            self.assertEqual(error.exception.code, 1)
            self.assertFalse((root / "backup").exists())


if __name__ == "__main__":
    unittest.main()
