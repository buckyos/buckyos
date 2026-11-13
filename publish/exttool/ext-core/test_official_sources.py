import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import zipfile

import official_sources
from compliance import ComplianceError, sha256


class OfficialSourceTests(unittest.TestCase):
    def fixture(self, root, unresolved=False):
        cache = root / "cache"
        cache.mkdir()
        archive = root / "upstream.tar.gz"
        with tarfile.open(archive, "w:gz") as handle:
            for name, text in (("upstream/LICENSE", "MIT License\nCopyright upstream\n"),
                               ("upstream/source.c", "int main(void) { return 0; }\n")):
                contents = text.encode()
                member = tarfile.TarInfo(name)
                member.size = len(contents)
                handle.addfile(member, io.BytesIO(contents))
        digest = sha256(archive)
        (cache / digest).write_bytes(archive.read_bytes())
        (root / "build.sh").write_text("TOOL_VERSION=1.2.3\n")
        binary_cache = root / ".cache"
        binary_cache.mkdir()
        original = binary_cache / "tool-binary.zip"
        original.write_bytes(b"upstream executable bytes")
        binary_hash = sha256(original)
        installed = root / "payload/store/ext-core/tool/example"
        installed.parent.mkdir(parents=True)
        installed.write_bytes(original.read_bytes())
        (root / "sources.sha256").write_text(binary_hash + "  tool-binary.zip\n")
        (root / "official-sources.json").write_text(json.dumps({
            "schema": "buckyos.official-source-inputs/v1", "build_versions": {"TOOL_VERSION": "1.2.3"},
            "components": [{"name": "example", "version": "1.2.3", "license": "MIT", "license_scope": "project",
                            "binaries": {"linux-x86_64": {"file": "tool-binary.zip", "sha256": binary_hash, "archive_member": None, "payload_file": "tool/example",
                                                          "url": "https://upstream.invalid/binary"}},
                            "sources": [{"file": "source.tar.gz", "sha256": digest,
                                         "urls": ["https://upstream.invalid/source"], "kind": "upstream-source"}],
                            "unresolved_source_requirements": ["missing linked library source"] if unresolved else []}]}))
        return root / "payload", root / "sources", cache

    def test_collect_preserves_source_and_upstream_notice_with_hashes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            payload, sources, cache = self.fixture(root)
            with patch.object(official_sources, "HERE", root):
                result = official_sources.collect_official(payload, sources, "linux-x86_64", cache, offline=True)
            component = result["components"][0]
            item = component["sources"][0]
            self.assertEqual(sha256(sources / item["file"]), item["sha256"])
            notices = payload / "store/ext-core/LICENSES" / component["license_directory"]
            self.assertEqual((notices / "source.tar.gz/upstream/LICENSE").read_text(), "MIT License\nCopyright upstream\n")
            self.assertEqual(json.loads((sources / "official-components.json").read_text()), result)

    def test_incomplete_static_closure_stays_a_release_blocker(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            payload, sources, cache = self.fixture(root, unresolved=True)
            with patch.object(official_sources, "HERE", root):
                result = official_sources.collect_official(payload, sources, "linux-x86_64", cache, offline=True)
            self.assertEqual(result["components"][0]["source_coverage"], "incomplete")
            self.assertEqual(result["components"][0]["unresolved_source_requirements"], ["missing linked library source"])

    def test_version_and_binary_drift_rejected_before_publication(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            payload, sources, cache = self.fixture(root)
            with patch.object(official_sources, "HERE", root):
                (root / "build.sh").write_text("TOOL_VERSION=2.0.0\n")
                with self.assertRaisesRegex(ComplianceError, "TOOL_VERSION changed"):
                    official_sources.collect_official(payload, sources, "linux-x86_64", cache, offline=True)
                (root / "build.sh").write_text("TOOL_VERSION=1.2.3\n")
                (root / "sources.sha256").write_text("b" * 64 + "  tool-binary.zip\n")
                with self.assertRaisesRegex(ComplianceError, "binary pin changed"):
                    official_sources.collect_official(payload, sources, "linux-x86_64", cache, offline=True)
            self.assertFalse((sources / "official-components.json").exists())

    def test_missing_offline_source_cannot_generate_inventory(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            payload, sources, cache = self.fixture(root)
            sources.mkdir()
            (sources / "official-components.json").write_text("stale inventory")
            for file in cache.iterdir():
                file.unlink()
            with patch.object(official_sources, "HERE", root), self.assertRaises(ComplianceError):
                official_sources.collect_official(payload, sources, "linux-x86_64", cache, offline=True)
            self.assertFalse((sources / "official-components.json").exists())

    def test_missing_or_changed_payload_is_not_matched_to_build_inputs(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            payload, sources, cache = self.fixture(root)
            installed = payload / "store/ext-core/tool/example"
            with patch.object(official_sources, "HERE", root):
                installed.write_bytes(b"different old version")
                with self.assertRaisesRegex(ComplianceError, "payload executable differs"):
                    official_sources.collect_official(payload, sources, "linux-x86_64", cache, offline=True)
                installed.unlink()
                with self.assertRaisesRegex(ComplianceError, "missing from payload"):
                    official_sources.collect_official(payload, sources, "linux-x86_64", cache, offline=True)
            self.assertFalse((sources / "official-components.json").exists())

    def test_upstream_make_manifest_resolves_variables_and_checksum_pins(self):
        with tempfile.TemporaryDirectory() as temporary:
            archive = Path(temporary) / "recipe.tar.gz"
            content = ("LIB_MINOR := 2\nLIB_TARBALL := lib-1.$(LIB_MINOR).tar.xz\nLIB_SHA256SUM := " + "c" * 64 + "\n").encode()
            with tarfile.open(archive, "w:gz") as handle:
                member = tarfile.TarInfo("project/download.lst")
                member.size = len(content)
                handle.addfile(member, io.BytesIO(content))
            inputs = official_sources.dependency_inputs(archive, {"member": "project/download.lst", "url_base": "https://upstream.invalid/"})
            self.assertEqual(inputs[0]["file"], "lib-1.2.tar.xz")
            self.assertEqual(inputs[0]["sha256"], "c" * 64)

    def test_mingit_package_provenance_requires_exact_binary_packages_and_sources(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / ".cache").mkdir()
            archive = root / ".cache/mingit.zip"
            with zipfile.ZipFile(archive, "w") as handle:
                handle.writestr("etc/package-versions.txt", "git 1.2-1\nlibexample 3.0-2\n")
            binary = {"file": archive.name, "sha256": sha256(archive)}
            source = {"file": "git.src.tar.gz", "sha256": "a" * 64}
            proof = {"schema": "buckyos.mingit-package-sources/v1", "binary_sha256": binary["sha256"],
                     "binary_manifest_member": "etc/package-versions.txt",
                     "packages": [{"name": name, "version": version, "source_file": source["file"],
                                   "source_sha256": source["sha256"]}
                                  for name, version in (("git", "1.2-1"), ("libexample", "3.0-2"))]}
            path = root / "mingit-sources.json"
            path.write_text(json.dumps(proof))
            component = {"sources": [source], "package_provenance": {"file": path.name, "sha256": sha256(path)}}
            with patch.object(official_sources, "HERE", root):
                record = official_sources.package_provenance(component, binary, root / "sources")
                self.assertEqual(sha256(root / "sources" / record["file"]), record["sha256"])
                component["sources"] = []
                with self.assertRaisesRegex(ComplianceError, "no pinned source"):
                    official_sources.package_provenance(component, binary, root / "sources")
                component["sources"] = [source]
                proof["packages"].pop()
                path.write_text(json.dumps(proof))
                component["package_provenance"]["sha256"] = sha256(path)
                with self.assertRaisesRegex(ComplianceError, "package list differs"):
                    official_sources.package_provenance(component, binary, root / "sources")


if __name__ == "__main__":
    unittest.main()
