import importlib.util
import hashlib
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

    def source_manifest(self, store):
        directory = store / "LICENSES/official/pandoc"
        directory.mkdir(parents=True, exist_ok=True)
        text = b"The complete upstream license text\n"
        (directory / "COPYING").write_bytes(text)
        manifest = {
            "schema": "buckyos.ext-tools-sources/v1", "platform": "windows-x86_64",
            "components": [{"id": "official:pandoc", "name": "pandoc", "version": "3.12.1",
                            "license": "GPL-2.0-or-later", "license_directory": "official/pandoc"}],
            "files": [{"path": "LICENSES/official/pandoc/COPYING", "sha256": hashlib.sha256(text).hexdigest()}],
        }
        self.write_source_manifest(store, manifest)
        return manifest

    def write_source_manifest(self, store, manifest):
        data = json.dumps(manifest).encode()
        (store / "LICENSES/source-manifest.json").write_bytes(data)
        (store / "LICENSES/corresponding-source.json").write_text(json.dumps({
            "manifest_sha256": "sha256:" + hashlib.sha256(data).hexdigest(),
            "path": "ext-tools/ext-core/1.2.0/windows-x86_64/source.tar.gz", "sha256": "sha256:" + "1" * 64,
        }))

    def test_verified_inventory_uses_actual_manifest_without_running_foreign_binaries(self):
        store = self.store("windows-x86_64")
        self.source_manifest(store)
        with mock.patch.object(runtime, "probe_version") as probe:
            packages, scopes, notices = runtime.collect_ext_core(self.repo)
        probe.assert_not_called()
        self.assertEqual(len(packages), 1)
        self.assertEqual(packages[0]["version"], "3.12.1")
        self.assertEqual(packages[0]["license"], "GPL-2.0-or-later")
        self.assertEqual(packages[0]["files"], [{"name": "COPYING", "text": "The complete upstream license text\n"}])
        self.assertEqual(scopes[-1]["status"], "complete")
        self.assertTrue(any("https://buckyos-installer.tbudr.top/ext-tools/" in note for note in notices))

    def test_changed_license_or_mismatched_source_archive_is_rejected(self):
        store = self.store("windows-x86_64")
        self.source_manifest(store)
        (store / "LICENSES/official/pandoc/COPYING").write_text("Changed text")
        with self.assertRaisesRegex(ValueError, "license text differs"):
            runtime.collect_ext_core(self.repo)
        self.source_manifest(store)
        (store / "LICENSES/source-manifest.json").write_text((store / "LICENSES/source-manifest.json").read_text() + "\n")
        with self.assertRaisesRegex(ValueError, "manifest checksum differs"):
            runtime.collect_ext_core(self.repo)

    def test_incomplete_sources_are_visible_and_unsafe_license_paths_are_rejected(self):
        store = self.store("windows-x86_64")
        manifest = self.source_manifest(store)
        manifest["components"][0]["unresolved_source_requirements"] = ["Missing exact dependency sources"]
        self.write_source_manifest(store, manifest)
        _, scopes, notices = runtime.collect_ext_core(self.repo)
        self.assertEqual(scopes[-1]["status"], "partial")
        self.assertTrue(any("Missing exact dependency sources" in note for note in notices))
        manifest["components"][0]["license_directory"] = "../../outside"
        self.write_source_manifest(store, manifest)
        with self.assertRaisesRegex(ValueError, "invalid component license directory"):
            runtime.collect_ext_core(self.repo)


if __name__ == "__main__":
    unittest.main()
