import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock


SPEC = importlib.util.spec_from_file_location("open_source_runtime", Path(__file__).resolve().parent.parent / "open_source_runtime.py")
runtime = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runtime)


class OpenSourceRuntimeTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.repo = Path(self.temp.name)
        self.source = self.repo / "publish/exttool/ext-core"
        self.source.mkdir(parents=True)
        (self.source / "ext-manifest.json").write_text(json.dumps({"platforms": ["linux-x86_64", "windows-x86_64"]}))

    def store(self, target):
        store = self.source / "payload" / target / "store/ext-core"
        (store / "LICENSES").mkdir(parents=True)
        return store

    def test_actual_conda_versions_and_licenses_are_preserved(self):
        store = self.store("windows-x86_64")
        (store / "LICENSES/conda-forge.tsv").write_text("name\tversion\tlicense\tsource\nimagemagick\t7.1.2_31\tAGPL-3.0-only AND ImageMagick\thttps://example.org/package\n")
        (self.source / "build.sh").write_text("IMAGEMAGICK_VERSION=99.0.0\n")
        with mock.patch.object(runtime, "native_platform", return_value="linux-x86_64"), mock.patch.object(runtime, "probe_version") as probe:
            packages, scopes, notices = runtime.collect_ext_core(self.repo)
        self.assertEqual(packages[0]["version"], "7.1.2_31")
        self.assertEqual(packages[0]["license"], "AGPL-3.0-only AND ImageMagick")
        probe.assert_not_called()
        self.assertEqual(scopes[0]["status"], "unavailable")
        self.assertTrue(any("could not be verified" in notice for notice in notices))

    def test_libreoffice_version_and_full_notice_come_from_payload(self):
        store = self.store("windows-x86_64")
        registry = store / "libreoffice/share/registry/main.xcd"
        registry.parent.mkdir(parents=True)
        registry.write_text('<data xmlns:oor="http://openoffice.org/2001/registry"><prop oor:name="ooSetupVersionAboutBox"><value/></prop><prop oor:name="ooSetupVersionAboutBox"><value>26.8.2.2</value></prop></data>')
        notice = "This product is made available subject to the Mozilla Public License, v. 2.0.\nThird-party notices follow."
        (store / "LICENSES/LibreOffice-LICENSE").write_text(notice)
        packages, _, _ = runtime.collect_ext_core(self.repo)
        office = next(p for p in packages if p["name"] == "libreoffice")
        self.assertEqual(office["version"], "26.8.2.2")
        self.assertEqual(office["license"], "SEE LICENSE IN LibreOffice-LICENSE")
        self.assertEqual(office["files"][0]["text"], notice)

    def test_native_version_probe_does_not_leak_output_into_inventory(self):
        store = self.store("linux-x86_64")
        (store / "node").mkdir()
        (store / "node/LICENSE").write_text("Bundled license text")
        def probe(path, pattern, arguments=None):
            return "24.21.0" if path.name == "node" else None
        with mock.patch.object(runtime, "native_platform", return_value="linux-x86_64"), mock.patch.object(runtime, "probe_version", side_effect=probe):
            packages, _, _ = runtime.collect_ext_core(self.repo)
        node = next(p for p in packages if p["name"] == "node")
        self.assertEqual(node["version"], "24.21.0")
        self.assertEqual(node["files"], [{"name": "LICENSE", "text": "Bundled license text"}])
        self.assertNotIn(str(self.repo), json.dumps(packages))


if __name__ == "__main__":
    unittest.main()
