import importlib.util
import io
import json
from pathlib import Path
import shutil
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch


HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import compliance
import release_manifest

# path -> (mode, contents): None is a directory, a str a symlink's target.
TREE = {"src": (0o755, None), "src/main.c": (0o644, b"int main(void) { return 0; }\n"),
        "src/run.sh": (0o755, b"#!/bin/sh\n"), "src/link": (0o777, "main.c")}
GITILES_URL = "https://chromium.googlesource.com/chromium/src/tools/example/+archive/" + "1" * 40 + ".tar.gz"


class ComplianceTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)

    def tearDown(self):
        self.temporary.cleanup()

    def tar_gz(self, name, members, mtime=1, reverse=False):
        path = self.root / name
        with tarfile.open(path, "w:gz") as handle:
            for member_name, (mode, contents) in sorted(members.items(), reverse=reverse):
                member = tarfile.TarInfo(member_name)
                member.mode, member.mtime = mode, mtime
                if contents is None:
                    member.type = tarfile.DIRTYPE
                elif isinstance(contents, str):
                    member.type, member.linkname = tarfile.SYMTYPE, contents
                else:
                    member.size = len(contents)
                handle.addfile(member, io.BytesIO(contents) if isinstance(contents, bytes) else None)
        return path

    def test_tree_sha256_is_the_content_not_the_bytes(self):
        pin = compliance.tree_sha256(self.tar_gz("original.tar.gz", TREE))
        # Another request of the same commit: other times, another order, no directory entries, a ./ prefix.
        repacked = self.tar_gz("repacked.tar.gz", {"./" + name: value for name, value in TREE.items() if value[1] is not None},
                               mtime=2, reverse=True)
        self.assertNotEqual(compliance.sha256(self.root / "original.tar.gz"), compliance.sha256(repacked))
        self.assertEqual(compliance.tree_sha256(repacked), pin)
        for changed in ({**TREE, "src/main.c": (0o644, b"int main(void) { return 1; }\n")},
                        {**TREE, "src/run.sh": (0o644, b"#!/bin/sh\n")},
                        {**TREE, "src/extra.h": (0o644, b"")},
                        {**TREE, "src/link": (0o777, "run.sh")}):
            self.assertNotEqual(compliance.tree_sha256(self.tar_gz("changed.tar.gz", changed)), pin)
        (self.root / "garbage.tar.gz").write_bytes(b"not an archive")
        with self.assertRaisesRegex(compliance.ComplianceError, "cannot read archive"):
            compliance.tree_sha256(self.root / "garbage.tar.gz")

    def test_download_checks_a_gitiles_archive_by_content_and_any_other_by_bytes(self):
        original = self.tar_gz("original.tar.gz", TREE)
        pin = compliance.tree_sha256(original)
        served = {}

        def urlopen(request, timeout):
            return io.BytesIO(served["bytes"])

        with patch.object(compliance, "urlopen", urlopen), patch.object(compliance.time, "sleep"):
            for mtime in (2, 3):
                served["bytes"] = self.tar_gz(f"request-{mtime}.tar.gz", TREE, mtime=mtime, reverse=mtime == 3).read_bytes()
                cache = self.root / f"cache-{mtime}"
                path = compliance.download([GITILES_URL], pin, cache, tree=True)
                self.assertEqual((path, path.read_bytes()), (cache / f"tree-{pin}", served["bytes"]))
                self.assertEqual(compliance.download([GITILES_URL], pin, cache, offline=True, tree=True), path)
            for changed in ({**TREE, "src/main.c": (0o644, b"changed\n")}, {**TREE, "src/run.sh": (0o644, b"#!/bin/sh\n")},
                            {**TREE, "src/extra.h": (0o644, b"")}):
                served["bytes"] = self.tar_gz("changed.tar.gz", changed).read_bytes()
                with self.assertRaisesRegex(compliance.ComplianceError, "tree_sha256 mismatch"):
                    compliance.download([GITILES_URL], pin, self.root / "cache-changed", tree=True)
            self.assertEqual(list((self.root / "cache-changed").iterdir()), [])
            (self.root / "cache-changed" / f"tree-{pin}").write_bytes(b"not an archive")
            with self.assertRaisesRegex(compliance.ComplianceError, "missing cached source"):
                compliance.download([GITILES_URL], pin, self.root / "cache-changed", offline=True, tree=True)
            # Anything else stays pinned by its bytes, and only a gitiles archive of a commit by its content.
            served["bytes"] = self.tar_gz("repacked.tar.gz", TREE, mtime=2).read_bytes()
            with self.assertRaisesRegex(compliance.ComplianceError, "SHA256 mismatch"):
                compliance.download(["https://example.invalid/source.tar.gz"], compliance.sha256(original), self.root / "cache-bytes")
            for url in ("https://example.invalid/source.tar.gz", GITILES_URL.replace("1" * 40, "main")):
                with self.assertRaisesRegex(compliance.ComplianceError, "only a gitiles"):
                    compliance.download([url], pin, self.root / "cache-bytes", tree=True)

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

    def test_a_package_built_from_its_own_recipe_directory_has_the_recipe_as_source(self):
        recipe = self.root / "info" / "recipe"
        recipe.mkdir(parents=True)
        (recipe / "meta.yaml").write_text(
            "# This file created by conda-build 26.7.1\n# meta.yaml template originally from:\n"
            "# /Users/runner/work/1/s/recipe, last modified Sat Aug 29 23:32:04 2026\n"
            "package:\n  name: _openmp_mutex\nsource:\n  path: /Users/runner/work/1/s/recipe\n")
        (recipe / "omp_hello.c").write_text("int main(void) { return 0; }\n")
        source = compliance.recipe_sources(recipe)[0]
        records = compliance.recipe_directory_sources(source, recipe, self.root, self.root)
        self.assertEqual([r["file"] for r in records], ["info/recipe/meta.yaml", "info/recipe/omp_hello.c"])
        self.assertTrue(all(r["kind"] == "recipe-source" and len(r["sha256"]) == 64 for r in records))
        # Any other absolute path is still refused.
        other = {"path": "/Users/runner/work/1/s/elsewhere"}
        self.assertIsNone(compliance.recipe_directory_sources(other, recipe, self.root, self.root))
        with self.assertRaisesRegex(compliance.ComplianceError, "unsafe relative path"):
            compliance.artifact(other, recipe, self.root, self.root)

    def test_a_cargo_git_source_is_included_only_when_its_commit_archive_is_pinned(self):
        origin = "git+https://github.com/example/crate?rev=" + "c" * 40 + "#" + "c" * 40
        lock = ('[[package]]\nname = "pinned"\nversion = "1.0.0"\nsource = "' + origin + '"\n\n'
                '[[package]]\nname = "unpinned"\nversion = "1.0.0"\nsource = "git+https://github.com/example/other?rev=1#1"\n')
        archive = self.root / "project.tar.gz"
        with tarfile.open(archive, "w:gz") as handle:
            data = lock.encode()
            info = tarfile.TarInfo("project-1.0/Cargo.lock")
            info.size = len(data)
            handle.addfile(info, io.BytesIO(data))
        pins = {origin: {"url": "https://codeload.github.com/example/crate/tar.gz/" + "c" * 40, "file": "crate.tar.gz", "sha256": "d" * 64}}
        (self.root / "cargo-git-sources.json").write_text(json.dumps(pins))
        with patch.object(compliance, "HERE", self.root):
            artifacts, unresolved = compliance.cargo_sources(archive)
        self.assertEqual([(a["file"], a["urls"]) for a in artifacts], [("upstream/" + "d" * 64 + "/crate.tar.gz", [pins[origin]["url"]])])
        self.assertEqual(len(unresolved), 1)
        self.assertIn("unpinned", unresolved[0])

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

    def make_release(self, incomplete=False, tree=None):
        payload = self.root / "payload"
        source = self.root / "source"
        dist = self.root / "dist"
        builder = self.root / "builder"
        (payload / "bin").mkdir(parents=True)
        (payload / "bin/tool").write_bytes(b"binary fixture\n")
        source.mkdir()
        builder.mkdir()
        for filename in ("build.sh", "compliance.py", "official_sources.py", "official-sources.json", "release_manifest.py",
                         "place_online.py", "conda-env.yml", "conda-lock.yml", "sources.sha256", "source-mirrors.json", "cargo-git-sources.json", "ext-manifest.json"):
            (builder / filename).write_text("build input fixture\n")
        for directory in ("windows", "smoke", "browser"):
            (builder / directory).mkdir()
        for kind in ("conda", "official"):
            artifact = source / (kind + "-source.txt")
            if tree and kind == "official":
                # (archive, the tree_sha256 its record has)
                artifact = source / "gitiles-source.tar.gz"
                shutil.copy2(tree[0], artifact)
            else:
                artifact.write_text("preferred form for modification\n")
            license_directory = payload / "store/ext-core/LICENSES" / kind
            license_directory.mkdir(parents=True)
            (license_directory / "LICENSE").write_text("third-party fixture license\n")
            component = {"id": kind + ":fixture", "license_directory": kind,
                         "sources": [{"file": artifact.name, "sha256": compliance.sha256(artifact), "kind": "upstream-source"}]}
            if tree and kind == "official":
                component["sources"][0]["tree_sha256"] = tree[1]
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

    def test_release_checks_a_gitiles_archive_by_its_content(self):
        archive = self.tar_gz("gitiles.tar.gz", TREE)
        other = self.tar_gz("other.tar.gz", {**TREE, "src/extra.h": (0o644, b"")})
        workspace = self.root
        for pin, passes in ((compliance.tree_sha256(archive), True), (compliance.tree_sha256(other), False)):
            with self.subTest(passes=passes):
                self.root = Path(tempfile.mkdtemp(dir=workspace))
                _, _, dist, _ = self.make_release(tree=(archive, pin))
                if passes:
                    release_manifest.build("1.2.0", dist, {}, None)
                else:
                    with self.assertRaisesRegex(compliance.ComplianceError, "content differs from its tree_sha256"):
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
