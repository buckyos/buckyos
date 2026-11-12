import importlib.util
import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock


SPEC = importlib.util.spec_from_file_location("open_source_gateway", Path(__file__).resolve().parent.parent / "open_source_gateway.py")
gateway = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(gateway)


class OpenSourceGatewayTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / "buckyos"
        self.repo.mkdir()
        self.source = self.root / "cyfs-gateway/src"

    def checkout(self):
        self.source.mkdir(parents=True)
        (self.source / "Cargo.toml").write_text("[workspace]\n", encoding="utf-8")
        (self.source / "Cargo.lock").write_text("version = 4\n", encoding="utf-8")

    def test_independent_gateway_versions_are_scoped_as_a_snapshot(self):
        self.checkout()
        original = {"name": "tokio", "version": "1.49.0", "license": "MIT", "scopes": ["rust"]}
        collect = Mock(return_value=([original], {"status": "complete"}, []))
        packages, scope, notices = gateway.collect_gateway(self.repo, collect, offline=True)
        collect.assert_called_once_with(self.source.parent, None, True)
        self.assertEqual(packages[0]["version"], "1.49.0")
        self.assertEqual(packages[0]["scopes"], ["gateway"])
        self.assertEqual(original["scopes"], ["rust"])
        self.assertEqual(scope, {"id": "gateway", "label": "CYFS Gateway", "status": "complete", "isSnapshot": True})
        self.assertIn("built separately", notices[0])
        self.assertNotIn(str(self.root), json.dumps([packages, scope, notices]))

    def test_missing_source_or_lock_never_resolves_new_versions(self):
        collect = Mock()
        _, scope, notices = gateway.collect_gateway(self.repo, collect)
        self.assertEqual(scope["status"], "unavailable")
        self.assertIn("source checkout", notices[-1])
        self.checkout()
        (self.source / "Cargo.lock").unlink()
        _, scope, notices = gateway.collect_gateway(self.repo, collect)
        self.assertEqual(scope["status"], "unavailable")
        self.assertIn("Cargo.lock", notices[-1])
        collect.assert_not_called()

    def test_metadata_failure_does_not_block_buckyos_or_expose_paths(self):
        self.checkout()
        for error in (subprocess.CalledProcessError(101, ["cargo", "metadata"]), OSError(str(self.root)), ValueError("invalid metadata")):
            with self.subTest(error=type(error).__name__):
                packages, scope, notices = gateway.collect_gateway(self.repo, Mock(side_effect=error))
                self.assertEqual(packages, [])
                self.assertEqual(scope["status"], "unavailable")
                self.assertNotIn(str(self.root), json.dumps(notices))


if __name__ == "__main__":
    unittest.main()
