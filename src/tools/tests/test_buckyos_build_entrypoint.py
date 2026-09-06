import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


SCRIPT_PATH = Path(__file__).parents[2] / "buckyos-build.py"


def _load_build_script():
    spec = importlib.util.spec_from_file_location("buckyos_build_script", SCRIPT_PATH)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class BuckyosBuildEntrypointTests(unittest.TestCase):
    def test_main_delegates_arguments_to_devkit_build(self) -> None:
        module = _load_build_script()
        completed = module.subprocess.CompletedProcess([], 23)

        with (
            patch.object(module, "_find_command", return_value="/runtime/buckyos-build"),
            patch.object(module, "_prepare_sdk_tool_distribution", return_value=0),
            patch.object(module.subprocess, "run", return_value=completed) as run,
        ):
            result = module.main(["--skip-web", "-s", "scheduler"])

        self.assertEqual(result, 23)
        run.assert_called_once_with(
            ["/runtime/buckyos-build", "--skip-web", "-s", "scheduler"],
            env=module.os.environ.copy(),
        )

    def test_main_prepares_sdk_tool_distribution_before_build(self) -> None:
        module = _load_build_script()
        env = {
            "BUCKYOS_SDK_TOOL_ARTIFACT": "/inputs/buckyos.tgz",
            "BUCKYOS_SDK_TOOL_RELEASE_MANIFEST": "/inputs/release.json",
            "BUCKYOS_SDK_TOOL_DENO": "/inputs/deno",
            "BUCKYOS_SDK_TOOL_SBOM": "/inputs/sbom.json",
        }
        completed = module.subprocess.CompletedProcess([], 0)

        with patch.object(module.subprocess, "run", return_value=completed) as run:
            result = module._prepare_sdk_tool_distribution(env)

        self.assertEqual(result, 0)
        run.assert_called_once_with(
            [
                module.sys.executable,
                str(
                    module.Path(module.__file__).parent
                    / "tools"
                    / "build_sdk_tool_distribution.py"
                ),
                "--artifact",
                "/inputs/buckyos.tgz",
                "--release-manifest",
                "/inputs/release.json",
                "--deno",
                "/inputs/deno",
                "--sbom",
                "/inputs/sbom.json",
            ],
            env=env,
        )

    def test_prepare_sdk_tool_distribution_reports_missing_inputs(self) -> None:
        module = _load_build_script()

        with patch.object(module.subprocess, "run") as run:
            result = module._prepare_sdk_tool_distribution({
                "BUCKYOS_SDK_TOOL_ARTIFACT": "/inputs/buckyos.tgz",
            })

        self.assertEqual(result, 2)
        run.assert_not_called()

    def test_default_build_uses_local_source_even_with_deno_override(self) -> None:
        module = _load_build_script()
        for env in ({}, {"BUCKYOS_SDK_TOOL_DENO": "/inputs/deno"}):
            with patch.object(module, "_build_local_sdk_tool_distribution", return_value=17) as build:
                self.assertEqual(module._prepare_sdk_tool_distribution(env), 17)
                build.assert_called_once_with(env)

    def test_main_does_not_build_modules_when_sdk_build_fails(self) -> None:
        module = _load_build_script()
        with (
            patch.object(module, "_find_command", return_value="/runtime/buckyos-build"),
            patch.object(module, "_prepare_sdk_tool_distribution", return_value=17),
            patch.object(module.subprocess, "run") as run,
        ):
            self.assertEqual(module.main([]), 17)
        run.assert_not_called()

    def test_local_build_failure_does_not_replace_distribution(self) -> None:
        module = _load_build_script()
        with tempfile.TemporaryDirectory() as temporary:
            (Path(temporary) / "package.json").write_text("{}", encoding="utf-8")
            env = {"BUCKYOS_SDK_TOOL_SOURCE": temporary}
            with (
                patch.object(module, "_find_command", side_effect=lambda name: f"/runtime/{name}"),
                patch.object(module.subprocess, "run", side_effect=[
                    module.subprocess.CompletedProcess([], 0),
                    module.subprocess.CompletedProcess([], 19),
                ]) as run,
                patch.object(module, "_build_sdk_tool_distribution") as install,
            ):
                self.assertEqual(module._prepare_sdk_tool_distribution(env), 19)
                self.assertEqual(run.call_count, 2)
                install.assert_not_called()

    def test_missing_local_source_does_not_fall_back_to_old_distribution(self) -> None:
        module = _load_build_script()
        with tempfile.TemporaryDirectory() as temporary:
            with patch.object(module.subprocess, "run") as run:
                self.assertEqual(module._prepare_sdk_tool_distribution({
                    "BUCKYOS_SDK_TOOL_SOURCE": temporary,
                }), 2)
                run.assert_not_called()

    def test_main_reports_missing_devkit_build(self) -> None:
        module = _load_build_script()

        with patch.object(module, "_find_command", return_value=None):
            result = module.main([])

        self.assertEqual(result, 127)


if __name__ == "__main__":
    unittest.main()
