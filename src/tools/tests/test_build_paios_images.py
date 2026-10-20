import importlib.util
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


SCRIPT_PATH = Path(__file__).parents[3] / "build.py"


def _load_build_script():
    spec = importlib.util.spec_from_file_location("buckyos_root_build_script", SCRIPT_PATH)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class BuildPaiosImagesTests(unittest.TestCase):
    def test_pulls_the_arch_tags_node_daemon_uses_unless_overridden_locally(self) -> None:
        module = _load_build_script()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with (
                patch.dict(os.environ, {"BUCKYOS_ROOT": str(root)}),
                patch.object(module, "normalized_arch", return_value="aarch64"),
                patch.object(module, "resolve_filebrowser_image", return_value="filebrowser:arm"),
            ):
                self.assertEqual(
                    module.default_docker_images(),
                    ["paios/aios:latest-aarch64", "paios/exttool:latest-aarch64", "filebrowser:arm"],
                )
                (root / "etc").mkdir()
                (root / "etc" / "devenv.json").write_text('{"aios": "local/aios-test", "exttool": " "}', encoding="utf-8")
                self.assertEqual(
                    module.default_docker_images(),
                    ["paios/exttool:latest-aarch64", "filebrowser:arm"],
                )
                (root / "etc" / "devenv.json").write_text("not json", encoding="utf-8")
                self.assertEqual(len(module.default_docker_images()), 3)


if __name__ == "__main__":
    unittest.main()
