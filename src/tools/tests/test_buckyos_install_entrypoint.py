import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch


SCRIPT_PATH = Path(__file__).parents[2] / "buckyos-install.py"


def _load_install_script():
    spec = importlib.util.spec_from_file_location("buckyos_install_script", SCRIPT_PATH)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class BuckyosInstallEntrypointTests(unittest.TestCase):
    def test_main_delegates_arguments_to_devkit_install(self) -> None:
        module = _load_install_script()
        completed = module.subprocess.CompletedProcess([], 23)

        with (
            patch.object(module, "_find_command", return_value="/runtime/buckyos-install"),
            patch.object(module.subprocess, "run", return_value=completed) as run,
        ):
            result = module.main(["--app=buckyos", "--target-rootfs=/tmp/buckyos"])

        self.assertEqual(result, 23)
        run.assert_called_once_with(
            [
                "/runtime/buckyos-install",
                "--app=buckyos",
                "--target-rootfs=/tmp/buckyos",
            ],
            env=module.os.environ.copy(),
        )

    def test_main_reports_missing_devkit_install(self) -> None:
        module = _load_install_script()

        with patch.object(module, "_find_command", return_value=None):
            result = module.main([])

        self.assertEqual(result, 127)


if __name__ == "__main__":
    unittest.main()
