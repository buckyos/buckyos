import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
import importlib.util
import importlib.machinery


REPO_ROOT = Path(__file__).resolve().parents[2]
APP_LOADER_DIR = REPO_ROOT / "src" / "rootfs" / "bin" / "app-loader"


def _import_app_loader_module(module_name: str):
    """
    Import a module from `src/rootfs/bin/app-loader`.

    Note:
    - The directory name contains '-', so it cannot be imported as a Python package.
    - Some files are executable scripts without `.py` extension (e.g. `start`).
      For those, we load by file path.
    """
    py_path = APP_LOADER_DIR / f"{module_name}.py"
    plain_path = APP_LOADER_DIR / module_name

    if py_path.exists():
        sys.path.insert(0, str(APP_LOADER_DIR))
        return __import__(module_name)

    if plain_path.exists():
        loader = importlib.machinery.SourceFileLoader(f"app_loader_{module_name}", str(plain_path))
        spec = importlib.util.spec_from_loader(f"app_loader_{module_name}", loader)
        if spec is None or spec.loader is None:
            raise ImportError(f"Cannot load module from {plain_path}")
        mod = importlib.util.module_from_spec(spec)
        # Ensure dependencies like `util` can be imported.
        sys.path.insert(0, str(APP_LOADER_DIR))
        spec.loader.exec_module(mod)
        return mod

    raise ModuleNotFoundError(f"No module or script found for {module_name!r} in {APP_LOADER_DIR}")


class TestAppDocParsing(unittest.TestCase):
    def test_parse_filebrowser_did_doc_json(self):
        app_doc_mod = _import_app_loader_module("app_doc")
        AppDoc = app_doc_mod.AppDoc

        did_doc_path = REPO_ROOT / "src" / "rootfs" / "local" / "did_docs" / "filebrowser.buckyos.bns.did.doc.json"
        raw = json.loads(did_doc_path.read_text(encoding="utf-8"))

        doc = AppDoc.from_dict(raw)
        self.assertEqual(doc.name, "buckyos_filebrowser")
        self.assertEqual(doc.version, "0.5.1")
        self.assertEqual(doc.show_name, "BuckyOS File Browser")

        # Ensure schema tolerance: pkg_list.web may be null in this sample.
        self.assertTrue(hasattr(doc.pkg_list, "web"))

    def test_install_config_tips_service_ports(self):
        app_doc_mod = _import_app_loader_module("app_doc")
        AppDoc = app_doc_mod.AppDoc

        did_doc_path = REPO_ROOT / "src" / "rootfs" / "local" / "did_docs" / "filebrowser.buckyos.bns.did.doc.json"
        raw = json.loads(did_doc_path.read_text(encoding="utf-8"))
        doc = AppDoc.from_dict(raw)

        self.assertIn("www", doc.install_config_tips.service_ports)
        self.assertEqual(doc.install_config_tips.service_ports["www"], 80)

    def test_select_docker_image_by_arch(self):
        app_doc_mod = _import_app_loader_module("app_doc")
        AppDoc = app_doc_mod.AppDoc

        did_doc_path = REPO_ROOT / "src" / "rootfs" / "local" / "did_docs" / "filebrowser.buckyos.bns.did.doc.json"
        raw = json.loads(did_doc_path.read_text(encoding="utf-8"))
        doc = AppDoc.from_dict(raw)

        image_amd64, pkg_amd64 = doc.get_docker_image_for_host("x86_64")
        self.assertIn("amd64", image_amd64)
        self.assertIn("amd64", pkg_amd64)

        image_arm, pkg_arm = doc.get_docker_image_for_host("aarch64")
        self.assertIn("aarch64", image_arm)
        self.assertIn("aarch64", pkg_arm)

    def test_parse_docker_digest_field(self):
        app_doc_mod = _import_app_loader_module("app_doc")
        AppDoc = app_doc_mod.AppDoc

        raw = {
            "name": "demo",
            "version": "0.1.0",
            "show_name": "Demo",
            "selector_type": "single",
            "install_config_tips": {"service_ports": {"www": 80}},
            "pkg_list": {
                "amd64_docker_image": {
                    "pkg_id": "demo-img#0.1.0",
                    "docker_image_name": "demo/demo:0.1.0-amd64",
                    "docker_image_digest": "sha256:deadbeef",
                }
            },
        }
        doc = AppDoc.from_dict(raw)
        desc = doc.pkg_list.get_docker_image_for_host("x86_64")
        self.assertEqual(desc.docker_image_digest, "sha256:deadbeef")


class TestStartHelpers(unittest.TestCase):
    def test_ensure_directory_accessible_returns_path(self):
        start_mod = _import_app_loader_module("start")

        with tempfile.TemporaryDirectory() as td:
            target = os.path.join(td, "nested", "dir")
            returned = start_mod.ensure_directory_accessible(target)
            self.assertEqual(returned, target)
            self.assertTrue(os.path.isdir(target))


if __name__ == "__main__":
    unittest.main(verbosity=2)

