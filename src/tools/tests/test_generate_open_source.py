import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


TOOLS = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(TOOLS))
SPEC = importlib.util.spec_from_file_location("generate_open_source", TOOLS / "generate_open_source.py")
inventory = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(inventory)


class OpenSourceInventoryTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.repo = Path(self.temporary.name)

    def npm(self, directory, name, version, **fields):
        directory.mkdir(parents=True, exist_ok=True)
        (directory / "package.json").write_text(json.dumps({"name": name, "version": version, **fields}))
        return directory

    def test_npm_uses_resolved_versions_and_transitive_dependencies(self):
        root = self.repo / "src/frame/desktop"
        self.npm(root, "app", "0.0.0", dependencies={"react": "^19.0.0"}, devDependencies={"test-only": "1.0.0"})
        package = self.npm(root / "node_modules/.pnpm/react@19.3.1/node_modules/react", "react", "19.3.1", license="MIT", dependencies={"shared": "^2"})
        (root / "node_modules/react").symlink_to(package, target_is_directory=True)
        self.npm(package.parent / "shared", "shared", "2.9.0", license={"type": "BSD-2-Clause"})
        self.npm(root / "node_modules/test-only", "test-only", "1.0.0", license="MIT")
        (package / "LICENSE").write_text("The license text supplied with this precise version.")
        packages, scope, _ = inventory.collect_web(self.repo, "desktop")
        self.assertEqual({(p["name"], p["version"]) for p in packages}, {("react", "19.3.1"), ("shared", "2.9.0")})
        self.assertEqual(scope["status"], "complete")
        self.assertEqual(packages[0]["files"][0]["name"], "LICENSE")
        self.assertNotIn(str(self.repo), json.dumps(packages))

    def test_cargo_excludes_dev_only_edges_and_keeps_git_revision(self):
        crate = self.repo / "crate"
        crate.mkdir()
        (crate / "LICENSE-MIT").write_text("Exact license")
        root = {"id": "app", "manifest_path": str(self.repo / "src/frame/app/Cargo.toml")}
        dependency = {"id": "lib", "name": "example", "version": "1.2.3", "license": "MIT OR Apache-2.0", "manifest_path": str(crate / "Cargo.toml"), "source": "git+https://github.com/example/lib?branch=main#abcd1234"}
        fixture = self.repo / "metadata.json"
        fixture.write_text(json.dumps({"workspace_members": ["app"], "packages": [root, dependency], "resolve": {"nodes": [
            {"id": "app", "deps": [{"pkg": "lib", "dep_kinds": [{"kind": None}]}, {"pkg": "dev-only", "dep_kinds": [{"kind": "dev"}]}]},
            {"id": "lib", "deps": []},
        ]}}))
        packages, _, _ = inventory.collect_cargo(self.repo, fixture)
        self.assertEqual(len(packages), 1)
        self.assertEqual(packages[0]["revision"], "abcd1234")
        self.assertEqual(packages[0]["files"], [{"name": "LICENSE-MIT", "text": "Exact license"}])

    def test_missing_metadata_is_not_invented(self):
        packages, scope, _ = inventory.collect_web(self.repo, "desktop")
        self.assertEqual(packages, [])
        self.assertEqual(scope["status"], "unavailable")
        self.assertIsNone(inventory.license_value(None))
        self.assertEqual(inventory.safe_url("file:///home/private"), "")
        self.assertEqual(inventory.safe_url("https://token@example.com/source"), "")

    def test_bundled_font_and_wasm_licenses_are_collected_with_relative_paths(self):
        package = self.repo / "pdfjs-dist"
        paths = ["LICENSE", "standard_fonts/LICENSE_FOXIT", "standard_fonts/LICENSE_LIBERATION", "wasm/LICENSE_OPENJPEG", "cmaps/LICENSE", "iccs/LICENSE"]
        for name in paths + ["node_modules/unrelated/LICENSE", ".git/LICENSE"]:
            path = package / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(f"Bundled license: {name}")
        outside = self.repo / "private.txt"
        outside.write_text("Must not be published")
        (package / "LICENSE-external").symlink_to(outside)
        files = inventory.license_files(package)
        self.assertEqual({entry["name"] for entry in files}, set(paths))
        self.assertNotIn("Must not be published", json.dumps(files))

    def test_notices_do_not_claim_a_license_grant_and_runtime_metadata_is_cached(self):
        unknown = {"name": "unknown", "version": "1", "ecosystem": "cargo", "license": None, "files": [{"name": "AUTHORS", "text": "An author"}, {"name": "NOTICE", "text": "A notice"}]}
        current = {"name": "deno", "version": "2.9.2", "ecosystem": "runtime", "license": None, "files": []}
        previous = {**current, "files": [{"name": "LICENSE.md", "text": "Exact tagged license"}]}
        inventory.complete_licenses([unknown, current], [previous], False)
        self.assertIsNone(unknown["license"])
        self.assertEqual(current["license"], "SEE LICENSE IN LICENSE.md")
        self.assertEqual(current["files"], previous["files"])

    def test_inventory_is_deterministic_and_preserves_multiple_versions(self):
        output = self.repo / "output"
        first = {"name": "example", "version": "1.0.0", "ecosystem": "npm", "license": "MIT", "scopes": ["desktop"], "files": [{"name": "LICENSE", "text": "license"}]}
        second = {**first, "scopes": ["opendan"]}
        third = {**first, "version": "2.0.0", "license": None}
        index = inventory.write_inventory(output, [first, second, third], [], [])
        self.assertEqual(len(index["packages"]), 2)
        package = next(p for p in index["packages"] if p["version"] == "1.0.0")
        self.assertEqual(package["scopes"], ["desktop", "opendan"])
        self.assertEqual(package["licenseFileCount"], 1)
        before = {p.relative_to(output).as_posix(): p.read_bytes() for p in output.rglob("*.json")}
        inventory.write_inventory(output, [third, second, first], [], [])
        after = {p.relative_to(output).as_posix(): p.read_bytes() for p in output.rglob("*.json")}
        self.assertEqual(before, after)
        loaded, _, _ = inventory.read_existing(output)
        self.assertEqual(loaded[0]["files"], first["files"])

    def test_partial_refresh_replaces_current_scope_and_marks_retained_snapshot(self):
        output = self.repo / "output"
        package = {"name": "old-web", "version": "1.0.0", "ecosystem": "npm", "license": None, "scopes": ["desktop"], "files": []}
        rust = {**package, "name": "rust-lib", "ecosystem": "cargo", "scopes": ["rust"]}
        inventory.write_inventory(output, [package, rust], [
            {"id": "desktop", "label": "Desktop", "status": "partial", "notices": ["stale desktop warning"]},
            {"id": "rust", "label": "Rust", "status": "complete", "notices": ["retained rust notice"]},
        ], [])
        with mock.patch.object(sys, "argv", ["generate", "--repo", str(self.repo), "--output", str(output), "--scope", "desktop"]):
            self.assertEqual(inventory.main(), 0)
        index = inventory.read_json(output / "index.json")
        self.assertEqual([p["name"] for p in index["packages"]], ["rust-lib"])
        scopes = {s["id"]: s for s in index["scopes"]}
        self.assertEqual(scopes["desktop"]["status"], "unavailable")
        self.assertFalse(scopes["desktop"]["isSnapshot"])
        self.assertTrue(scopes["rust"]["isSnapshot"])
        self.assertNotIn("stale desktop warning", index["notices"])
        self.assertIn("retained rust notice", index["notices"])


if __name__ == "__main__":
    unittest.main()
