import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import unittest


REPOSITORY = Path(__file__).parents[3]
SCRIPT = REPOSITORY / "build_aios"


@unittest.skipUnless(shutil.which("bash"), "build_aios requires Bash")
class BuildAiosLicenseTests(unittest.TestCase):
    def stage_context(self, repository, output):
        source = SCRIPT.read_text(encoding="utf-8")
        functions = []
        for name in ("prepare_license_materials", "render_dockerfile"):
            match = re.search(rf"^{name}\(\) \{{\n.*?^\}}$", source, re.MULTILINE | re.DOTALL)
            self.assertIsNotNone(match, name)
            functions.append(match.group())
        body = "\n".join([
            "set -euo pipefail",
            'SRC_DIR="$1/src"',
            "IMAGE_NAME=fixture/aios",
            "RUNTIME_PACKAGES=(bash)",
            "EXTRA_APT_PACKAGES=()",
            *functions,
            'prepare_license_materials "$2" "$3"',
            'render_dockerfile "$2/Dockerfile"',
        ])
        return subprocess.run(
            ["bash", "-c", body, "build-aios-license-test", str(repository), str(output), sys.executable],
            capture_output=True, text=True, env=os.environ.copy(),
        )

    def test_context_includes_exact_license_materials_and_image_copy(self):
        with tempfile.TemporaryDirectory() as temporary:
            context = Path(temporary) / "context with spaces"
            result = self.stage_context(REPOSITORY, context)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            materials = context / "licenses/buckyos"
            self.assertEqual(set(p.name for p in materials.iterdir()), {"LICENSE", "NOTICE", "SOURCE.md", "source.json"})
            for name, source in (("LICENSE", "LICENSE"), ("NOTICE", "NOTICE"), ("SOURCE.md", "doc/licensing.md")):
                self.assertEqual((materials / name).read_bytes(), (REPOSITORY / source).read_bytes())
            record = json.loads((materials / "source.json").read_text())
            self.assertEqual(record["license"], "AGPL-3.0-only")
            self.assertEqual(record["scope"], "packaging_checkout")
            self.assertFalse(record["binary_source_verified"])
            self.assertIn("COPY licenses /opt/buckyos/licenses\n", (context / "Dockerfile").read_text())

    def test_missing_material_stops_before_rendering_image(self):
        with tempfile.TemporaryDirectory() as temporary:
            repository = Path(temporary) / "incomplete checkout"
            tools = repository / "src/tools"
            tools.mkdir(parents=True)
            shutil.copyfile(REPOSITORY / "src/tools/prepare_licenses.py", tools / "prepare_licenses.py")
            context = Path(temporary) / "context"
            result = self.stage_context(repository, context)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("missing distribution license material", result.stderr)
            self.assertFalse((context / "Dockerfile").exists())


if __name__ == "__main__":
    unittest.main()
