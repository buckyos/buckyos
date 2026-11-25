import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "finish_agent_tools_copy.py"


def _load():
    spec = importlib.util.spec_from_file_location("finish_agent_tools_copy", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class FinishAgentToolsCopyTest(unittest.TestCase):
    def _copy(self, temporary: str, imports: dict[str, str]) -> Path:
        directory = Path(temporary) / "buckyos-agent"
        (directory / "site-template" / "test").mkdir(parents=True)
        (directory / "site-template" / "test" / "site.spec.ts").write_text("// test\n", encoding="utf-8")
        (directory / "site-template" / "app.js").write_text("// page\n", encoding="utf-8")
        (directory / "deno.lock").write_text("{}", encoding="utf-8")
        (directory / "deno.json").write_text(json.dumps({"imports": imports}), encoding="utf-8")
        return directory

    def test_development_files_are_left_out_and_no_deno_is_needed_without_npm_imports(self) -> None:
        module = _load()
        with tempfile.TemporaryDirectory() as temporary:
            directory = self._copy(temporary, {"buckyos": "../buckyos-tool/dist/node.mjs"})
            self.assertEqual(module.finish(directory, "/nonexistent/deno"), 0)
            self.assertFalse((directory / "deno.lock").exists())
            self.assertFalse((directory / "site-template" / "test").exists())
            self.assertTrue((directory / "site-template" / "app.js").is_file())
            self.assertFalse((directory / "vendor").exists())

    def test_an_npm_specifier_in_the_source_is_refused(self) -> None:
        module = _load()
        with tempfile.TemporaryDirectory() as temporary:
            directory = self._copy(temporary, {})
            (directory / "lib.ts").write_text('const dom = await import("npm:linkedom@0.18.12");\n', encoding="utf-8")
            self.assertEqual(module.finish(directory, None), 2)

    def test_a_failed_bundle_fails_the_copy(self) -> None:
        module = _load()
        with tempfile.TemporaryDirectory() as temporary:
            directory = self._copy(temporary, {"linkedom": "npm:linkedom@0.18.12"})
            self.assertNotEqual(module.finish(directory, "/bin/false"), 0)
            self.assertEqual(json.loads((directory / "deno.json").read_text(encoding="utf-8"))["imports"]["linkedom"], "npm:linkedom@0.18.12")


if __name__ == "__main__":
    unittest.main()
