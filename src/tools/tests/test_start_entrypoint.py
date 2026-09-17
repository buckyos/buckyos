import contextlib
import importlib.util
import io
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


SCRIPT_PATH = Path(__file__).parents[2] / "start.py"


def _load_start_script():
    spec = importlib.util.spec_from_file_location("buckyos_start_script", SCRIPT_PATH)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class BuckyosStartEntrypointTests(unittest.TestCase):
    def test_invalid_arguments_stop_before_any_side_effects(self) -> None:
        for args in (
            ["--release"],
            ["release"],
            ["--reinstal", "release"],
            ["--reinstall", "release", "--skip-update"],
            ["--all", "--reinstall", "release"],
        ):
            with self.subTest(args=args):
                module = _load_start_script()
                with (
                    patch.object(module, "_sdk_tool_distribution_ready") as ready,
                    patch.object(module, "kill_all_processes") as kill_all,
                    patch.object(module, "update_files") as update_files,
                    patch.object(module, "start_system") as start_system,
                    contextlib.redirect_stderr(io.StringIO()) as stderr,
                ):
                    result = module.main(args)

                self.assertEqual(result, 2)
                self.assertIn("error:", stderr.getvalue())
                ready.assert_not_called()
                kill_all.assert_not_called()
                update_files.assert_not_called()
                start_system.assert_not_called()

    def test_startup_modes_select_expected_configuration(self) -> None:
        for args, expected_update in (
            ([], (False, None)),
            (["--all"], (True, "dev")),
            (["--reinstall"], (True, None)),
            (["--reinstall", "release"], (True, "release")),
            (["--skip-update"], None),
        ):
            with self.subTest(args=args):
                module = _load_start_script()
                with (
                    patch.object(module, "_sdk_tool_distribution_ready", return_value=True),
                    patch.object(module, "kill_all_processes") as kill_all,
                    patch.object(module, "update_files") as update_files,
                    patch.object(module, "start_system") as start_system,
                ):
                    result = module.main(args)

                self.assertEqual(result, 0)
                kill_all.assert_called_once_with()
                start_system.assert_called_once_with()
                if expected_update is None:
                    update_files.assert_not_called()
                else:
                    update_files.assert_called_once_with(*expected_update)

    def test_main_stops_before_install_when_sdk_tool_is_missing(self) -> None:
        module = _load_start_script()

        with (
            patch.object(module, "_sdk_tool_distribution_ready", return_value=False),
            patch.object(module, "kill_all_processes") as kill_all,
            patch.object(module, "update_files") as update_files,
            patch.object(module, "start_system") as start_system,
        ):
            result = module.main([])

        self.assertEqual(result, 2)
        kill_all.assert_not_called()
        update_files.assert_not_called()
        start_system.assert_not_called()

    def test_sdk_tool_distribution_requires_runtime_and_metadata(self) -> None:
        module = _load_start_script()
        runtime_name = "deno.exe" if module.os.name == "nt" else "deno"

        with tempfile.TemporaryDirectory() as temporary:
            tool_dir = Path(temporary) / "buckyos-tool"
            (tool_dir / "cli").mkdir(parents=True)
            (tool_dir / "runtime").mkdir()
            (tool_dir / "distribution.json").write_text("{}", encoding="utf-8")
            (tool_dir / "cli" / "system_bootstrap.ts").write_text("", encoding="utf-8")
            (tool_dir / "runtime" / runtime_name).write_bytes(b"runtime")

            with patch.object(module, "SDK_TOOL_DIR", tool_dir):
                self.assertTrue(module._sdk_tool_distribution_ready())


if __name__ == "__main__":
    unittest.main()
