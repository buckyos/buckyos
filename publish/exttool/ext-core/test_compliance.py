import importlib.util
import io
import json
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch


HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import compliance
import release_manifest


class ComplianceTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)

    def tearDown(self):
        self.temporary.cleanup()

    def test_rattler_staging_sources_are_included(self):
        recipe = self.root / "recipe"
        recipe.mkdir()
        (recipe / "rendered_recipe.yaml").write_text(
            "recipe:\n  staging_caches:\n  - name: compiled-library\n    source:\n"
            "    - url: https://example.invalid/library.tar.gz\n      sha256: " + "a" * 64 + "\n"
        )
        self.assertEqual(compliance.recipe_sources(recipe)[0]["url"], "https://example.invalid/library.tar.gz")

    def test_archive_paths_use_portable_separators(self):
        self.assertEqual(compliance.safe_relative(r"licenses\library\COPYING").as_posix(), "licenses/library/COPYING")
        with self.assertRaises(compliance.ComplianceError):
            compliance.safe_relative(r"C:\outside\LICENSE")

    def test_unrendered_or_vcs_recipes_fail(self):
        recipe = self.root / "recipe"
        recipe.mkdir()
        for body in ('source:\n  git_url: https://example.invalid/repo\n  git_rev: main\n',
                     'source:\n  url: "https://example.invalid/{{ version }}.tar.gz"\n'):
            (recipe / "meta.yaml").write_text(body)
            with self.assertRaises(compliance.ComplianceError):
                compliance.recipe_sources(recipe)

    def test_missing_patch_and_unpinned_source_fail(self):
        source = {"url": "https://example.invalid/source.tar.gz", "sha256": "a" * 64, "patches": ["missing.patch"]}
        with self.assertRaisesRegex(compliance.ComplianceError, "missing recipe patch"):
            compliance.artifact(source, self.root, self.root, self.root)
        source.pop("patches")
        source.pop("sha256")
        with self.assertRaisesRegex(compliance.ComplianceError, "SHA256 pin"):
            compliance.artifact(source, self.root, self.root, self.root)

    def test_corrupt_cached_source_is_not_accepted_offline(self):
        cache = self.root / "cache"
        cache.mkdir()
        (cache / ("a" * 64)).write_text("untrusted bytes")
        with self.assertRaises(compliance.ComplianceError):
            compliance.download(["https://example.invalid/source"], "a" * 64, cache, offline=True)

    def test_copy_readonly_license_can_be_repeated(self):
        source = self.root / "input"
        source.mkdir()
        (source / "COPYING").write_text("license text")
        (source / "COPYING").chmod(0o444)
        compliance.copy_tree(source, self.root / "copy")
        compliance.copy_tree(source, self.root / "copy")
        self.assertEqual((self.root / "copy/COPYING").read_text(), "license text")

    def test_archive_notice_path_never_escapes_destination(self):
        archive = self.root / "source.tar.gz"
        with tarfile.open(archive, "w:gz") as handle:
            member = tarfile.TarInfo("../AUTHORS.txt")
            member.size = len(b"upstream credit")
            handle.addfile(member, io.BytesIO(b"upstream credit"))
        destination = self.root / "notices"
        paths = compliance.extract_notices(archive, destination)
        self.assertEqual(len(paths), 1)
        self.assertEqual((destination / paths[0]).read_text(), "upstream credit")
        self.assertFalse((self.root / "AUTHORS.txt").exists())
        self.assertTrue(compliance.license_name("../OFL.txt"))
        self.assertTrue(compliance.license_name("font/UNLICENSE"))

    def make_release(self, incomplete=False):
        payload = self.root / "payload"
        source = self.root / "source"
        dist = self.root / "dist"
        builder = self.root / "builder"
        (payload / "bin").mkdir(parents=True)
        (payload / "bin/tool").write_bytes(b"binary fixture\n")
        source.mkdir()
        builder.mkdir()
        for filename in ("build.sh", "compliance.py", "official_sources.py", "official-sources.json", "release_manifest.py",
                         "conda-env.yml", "conda-lock.yml", "sources.sha256", "source-mirrors.json", "ext-manifest.json"):
            (builder / filename).write_text("build input fixture\n")
        for directory in ("windows", "smoke"):
            (builder / directory).mkdir()
        for kind in ("conda", "official"):
            artifact = source / (kind + "-source.txt")
            artifact.write_text("preferred form for modification\n")
            license_directory = payload / "store/ext-core/LICENSES" / kind
            license_directory.mkdir(parents=True)
            (license_directory / "LICENSE").write_text("third-party fixture license\n")
            component = {"id": kind + ":fixture", "license_directory": kind,
                         "sources": [{"file": artifact.name, "sha256": compliance.sha256(artifact), "kind": "upstream-source"}]}
            if incomplete and kind == "official":
                component["unresolved_source_requirements"] = ["missing statically linked library source"]
            compliance.write_json(source / (kind + "-components.json"), {
                "schema": compliance.SCHEMA, "platform": "linux-x86_64", "components": [component],
            })
        with patch.object(compliance, "HERE", builder):
            reference = compliance.finalize(payload, source, "linux-x86_64", "1.2.0", dist)
        tarpath = dist / "ext-core-1.2.0-linux-x86_64.tar.gz"
        with tarfile.open(tarpath, "w:gz") as archive:
            archive.add(payload, arcname=".")
        return payload, source, dist, reference

    def test_release_requires_matching_source_and_binary(self):
        _, _, dist, reference = self.make_release()
        result = release_manifest.build("1.2.0", dist, {}, None)
        self.assertEqual(result["packages"]["ext-core"]["platforms"]["linux-x86_64"]["corresponding_source"], reference)
        (dist / reference["file"]).unlink()
        with self.assertRaisesRegex(compliance.ComplianceError, "must accompany"):
            release_manifest.build("1.2.0", dist, {}, None)

    def test_incomplete_official_closure_prevents_archive(self):
        with self.assertRaisesRegex(compliance.ComplianceError, "missing statically linked"):
            self.make_release(incomplete=True)
        self.assertFalse((self.root / "dist").exists())

    def test_changed_payload_prevents_release(self):
        payload, _, dist, _ = self.make_release()
        (payload / "bin/tool").write_text("unmatched changed executable")
        with tarfile.open(dist / "ext-core-1.2.0-linux-x86_64.tar.gz", "w:gz") as archive:
            archive.add(payload, arcname=".")
        with self.assertRaisesRegex(compliance.ComplianceError, "hash/size changed"):
            release_manifest.build("1.2.0", dist, {}, None)

    def test_changed_source_archive_prevents_release(self):
        _, _, dist, reference = self.make_release()
        with (dist / reference["file"]).open("ab") as handle:
            handle.write(b"changed")
        with self.assertRaisesRegex(compliance.ComplianceError, "archive digest/size"):
            release_manifest.build("1.2.0", dist, {}, None)

    def test_changed_or_removed_payload_license_prevents_release(self):
        payload, _, dist, _ = self.make_release()
        license_file = payload / "store/ext-core/LICENSES/conda/LICENSE"
        for delete in (False, True):
            if delete:
                license_file.unlink()
            else:
                license_file.write_text("altered notice")
            with tarfile.open(dist / "ext-core-1.2.0-linux-x86_64.tar.gz", "w:gz") as archive:
                archive.add(payload, arcname=".")
            with self.assertRaises(compliance.ComplianceError):
                release_manifest.build("1.2.0", dist, {}, None)


if __name__ == "__main__":
    unittest.main()
