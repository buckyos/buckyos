import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).parents[1] / "prepare_licenses.py"
spec = importlib.util.spec_from_file_location("prepare_licenses_tests", SCRIPT)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class PrepareLicensesTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.repository = self.root / "repository"
        self.repository.mkdir()
        (self.repository / "doc").mkdir()
        (self.repository / "LICENSE").write_bytes(b"AGPL license fixture\n")
        (self.repository / "NOTICE").write_bytes(b"Copyright notice fixture\n")
        (self.repository / "doc/licensing.md").write_bytes(b"Corresponding source instructions\n")
        hooks = self.root / "hooks"
        hooks.mkdir()
        self.git("init", "--quiet")
        self.git("add", ".")
        self.git(
            "-c", "user.name=License Test", "-c", "user.email=license-test@example.invalid",
            "-c", "commit.gpgsign=false", "-c", f"core.hooksPath={hooks}",
            "commit", "--quiet", "-m", "License materials",
        )
        self.commit = self.git("rev-parse", "HEAD").strip()

    def git(self, *args):
        return subprocess.check_output(
            ["git", *args], cwd=self.repository, text=True, stderr=subprocess.PIPE,
        )

    def test_clean_git_source_record_identifies_exact_commit(self):
        record = module.source_record(self.repository)
        self.assertEqual(record["schema_version"], 1)
        self.assertEqual(record["scope"], "packaging_checkout")
        self.assertFalse(record["binary_source_verified"])
        self.assertEqual(record["license"], "AGPL-3.0-only")
        self.assertEqual(record["repository"], "https://github.com/buckyos/buckyos")
        self.assertEqual(record["commit"], self.commit)
        self.assertFalse(record["working_tree_modified"])
        self.assertEqual(record["source_tree"], f"https://github.com/buckyos/buckyos/tree/{self.commit}")

    def test_modified_and_untracked_files_are_recorded(self):
        notice = self.repository / "NOTICE"
        notice.write_bytes(b"Changed copyright notice\n")
        self.assertTrue(module.source_record(self.repository)["working_tree_modified"])
        self.git("restore", "NOTICE")
        (self.repository / "untracked-source.rs").write_bytes(b"fn example() {}\n")
        record = module.source_record(self.repository)
        self.assertTrue(record["working_tree_modified"])
        self.assertEqual(record["commit"], self.commit)

    def test_source_archive_without_git_has_unknown_revision(self):
        archive = self.root / "source-archive"
        archive.mkdir()
        record = module.source_record(archive)
        self.assertIsNone(record["commit"])
        self.assertIsNone(record["working_tree_modified"])
        self.assertIsNone(record["source_tree"])
        self.assertEqual(record["scope"], "packaging_checkout")
        self.assertFalse(record["binary_source_verified"])
        self.assertEqual(record["license"], "AGPL-3.0-only")

    def test_prepare_copies_exact_materials_and_source_metadata(self):
        output = self.root / "payload/licenses/buckyos"
        record = module.prepare(output, self.repository)
        self.assertEqual(set(path.name for path in output.iterdir()), {"LICENSE", "NOTICE", "SOURCE.md", "source.json"})
        for output_name, source_name in (
            ("LICENSE", "LICENSE"), ("NOTICE", "NOTICE"), ("SOURCE.md", "doc/licensing.md"),
        ):
            self.assertEqual((output / output_name).read_bytes(), (self.repository / source_name).read_bytes())
        self.assertEqual(json.loads((output / "source.json").read_text()), record)
        self.assertEqual(record["commit"], self.commit)
        self.assertFalse(record["working_tree_modified"])

    def test_missing_material_fails_before_creating_payload(self):
        for name in ("LICENSE", "NOTICE", "doc/licensing.md"):
            with self.subTest(name=name):
                source = self.repository / name
                content = source.read_bytes()
                source.unlink()
                output = self.root / "payload"
                with self.assertRaisesRegex(FileNotFoundError, "missing distribution license material"):
                    module.prepare(output, self.repository)
                self.assertFalse(output.exists())
                source.write_bytes(content)


if __name__ == "__main__":
    unittest.main()
