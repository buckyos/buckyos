import importlib.util
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import patch


class WindowsContainerBootstrapTests(unittest.TestCase):
    def test_default_installs_wsl_and_override_installs_docker(self):
        spec = importlib.util.spec_from_file_location("buckyos_devenv_test", Path(__file__).parents[3] / "devenv.py")
        module = importlib.util.module_from_spec(spec)
        with patch.dict(sys.modules, {spec.name: module}):
            spec.loader.exec_module(module)
        for runtime, expected in ((None, "Microsoft.WSL"), ("docker", "Docker.DockerDesktop")):
            with self.subTest(runtime=runtime):
                bootstrapper = object.__new__(module.Bootstrapper)
                bootstrapper.args = SimpleNamespace(skip_docker=False, skip_msvc=True)
                bootstrapper.notes = []
                bootstrapper.warnings = []
                environment = {} if runtime is None else {"BUCKYOS_CONTAINER_RUNTIME": runtime}
                with (
                    patch.dict(module.os.environ, environment, clear=True),
                    patch.object(bootstrapper, "update_package_index"),
                    patch.object(bootstrapper, "ensure_windows_libclang"),
                    patch.object(bootstrapper, "ensure_rust_toolchain"),
                    patch.object(bootstrapper, "resolve_package_set", side_effect=lambda choices, kind: choices[0]),
                    patch.object(bootstrapper, "package_installed", side_effect=lambda package, kind: package != expected),
                    patch.object(bootstrapper, "install_winget_package") as install,
                ):
                    bootstrapper.install_windows_environment()
                install.assert_called_once_with(expected)
                if runtime is None:
                    self.assertTrue(any("wsl --install --no-distribution" in note for note in bootstrapper.notes))
