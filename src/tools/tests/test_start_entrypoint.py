import contextlib
import importlib.util
import io
from pathlib import Path
import shutil
import stat
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
                    tempfile.TemporaryDirectory() as temporary,
                    patch.object(module, "resolve_buckyos_root", return_value=Path(temporary)),
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

    def test_all_restores_devenv_before_start_and_reports_it_last(self) -> None:
        module = _load_start_script()
        original = (
            b'{\n  "aios": "local/aios-test",\n'
            b'  "exttool": "local/exttool-test", "other": {"enabled": true}\n}\n'
        )

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            config = root / "etc" / "devenv.json"
            config.parent.mkdir()
            config.write_bytes(original)
            config.chmod(0o640)
            original_mode = stat.S_IMODE(config.stat().st_mode)

            def install(command, args):
                self.assertEqual(command, "buckyos-update")
                self.assertEqual(args, ["--app=buckyos", "--all"])
                shutil.rmtree(config.parent)
                return 0

            def make_config(group, target):
                self.assertEqual((group, target), ("dev", root))
                config.parent.mkdir()
                config.write_text('{"aios": "paios/aios"}', encoding="utf-8")

            def start():
                self.assertEqual(config.read_bytes(), original)
                self.assertEqual(stat.S_IMODE(config.stat().st_mode), original_mode)

            with (
                patch.object(module, "resolve_buckyos_root", return_value=root),
                patch.object(module, "_sdk_tool_distribution_ready", return_value=True),
                patch.object(module, "kill_all_processes"),
                patch.object(module, "_run_command", side_effect=install) as run_command,
                patch.object(module, "_run_make_config", side_effect=make_config) as run_make_config,
                patch.object(module, "start_system", side_effect=start) as start_system,
                contextlib.redirect_stdout(io.StringIO()) as stdout,
            ):
                result = module.main(["--all"])

            self.assertEqual(result, 0)
            run_command.assert_called_once()
            run_make_config.assert_called_once()
            start_system.assert_called_once()
            reminder = stdout.getvalue().split("=== BuckyOS Startup Complete ===", 1)[1]
            self.assertIn(str(config), reminder)
            self.assertIn("aios", reminder)
            self.assertIn("exttool", reminder)
            self.assertIn("default", reminder.lower())

    def test_all_restores_devenv_when_install_or_config_generation_fails(self) -> None:
        for failure in ("install_returncode", "install_system_exit", "make_config"):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as temporary:
                module = _load_start_script()
                root = Path(temporary)
                config = root / "etc" / "devenv.json"
                config.parent.mkdir()
                original = b'{"aios": "local/aios-test"}\n'
                config.write_bytes(original)
                config.chmod(0o600)
                original_mode = stat.S_IMODE(config.stat().st_mode)

                def install(command, args):
                    shutil.rmtree(config.parent)
                    if failure == "install_system_exit":
                        raise SystemExit(7)
                    return 1 if failure == "install_returncode" else 0

                def make_config(group, target):
                    config.parent.mkdir()
                    config.write_text("{}", encoding="utf-8")
                    raise RuntimeError("configuration generation failed")

                with (
                    patch.object(module, "resolve_buckyos_root", return_value=root),
                    patch.object(module, "_sdk_tool_distribution_ready", return_value=True),
                    patch.object(module, "kill_all_processes"),
                    patch.object(module, "_run_command", side_effect=install),
                    patch.object(module, "_run_make_config", side_effect=make_config),
                    patch.object(module, "start_system") as start_system,
                    contextlib.redirect_stdout(io.StringIO()),
                    self.assertRaises(SystemExit) as exit_error,
                ):
                    module.main(["--all"])

                self.assertEqual(exit_error.exception.code, 7 if failure == "install_system_exit" else 1)
                self.assertEqual(config.read_bytes(), original)
                self.assertEqual(stat.S_IMODE(config.stat().st_mode), original_mode)
                start_system.assert_not_called()

    def test_all_without_devenv_does_not_create_it_or_print_a_reminder(self) -> None:
        module = _load_start_script()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            config = root / "etc" / "devenv.json"

            with (
                patch.object(module, "resolve_buckyos_root", return_value=root),
                patch.object(module, "_sdk_tool_distribution_ready", return_value=True),
                patch.object(module, "kill_all_processes"),
                patch.object(module, "_run_command", return_value=0),
                patch.object(module, "_run_make_config"),
                patch.object(module, "start_system"),
                contextlib.redirect_stdout(io.StringIO()) as stdout,
            ):
                result = module.main(["--all"])

            self.assertEqual(result, 0)
            self.assertFalse(config.exists())
            self.assertNotIn("devenv.json", stdout.getvalue())

    def test_reinstall_keeps_replacing_devenv(self) -> None:
        module = _load_start_script()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            config = root / "etc" / "devenv.json"
            config.parent.mkdir()
            config.write_text('{"aios": "local/aios-test"}', encoding="utf-8")
            generated = b'{"aios": "paios/aios"}\n'

            def install(command, args):
                self.assertEqual(args, ["--app=buckyos", "--all"])
                shutil.rmtree(config.parent)
                return 0

            def make_config(group, target):
                self.assertEqual((group, target), ("release", root))
                config.parent.mkdir()
                config.write_bytes(generated)

            with (
                patch.object(module, "resolve_buckyos_root", return_value=root),
                patch.object(module, "_sdk_tool_distribution_ready", return_value=True),
                patch.object(module, "kill_all_processes"),
                patch.object(module, "_run_command", side_effect=install),
                patch.object(module, "_run_make_config", side_effect=make_config),
                patch.object(module, "start_system") as start_system,
                contextlib.redirect_stdout(io.StringIO()) as stdout,
            ):
                result = module.main(["--reinstall", "release"])

            self.assertEqual(result, 0)
            self.assertEqual(config.read_bytes(), generated)
            start_system.assert_called_once()
            self.assertNotIn("devenv.json", stdout.getvalue())

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
