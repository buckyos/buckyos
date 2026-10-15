import contextlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import unittest
from unittest.mock import patch


SCRIPT_PATH = Path(__file__).parents[2] / "stop.py"


def load_stop():
    spec = importlib.util.spec_from_file_location("buckyos_stop", SCRIPT_PATH)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class StopContainersTests(unittest.TestCase):
    def test_wslc_removes_only_buckyos_apps_from_ndjson_and_array(self):
        rows = [{"Names": "unrelated"}, {"Names": "buckyos-app-one"}, {"Names": "buckyos-app-two"}]
        for output in ("\n".join(map(json.dumps, rows)), json.dumps(rows), "[]", ""):
            with self.subTest(output=output):
                module = load_stop()
                results = [subprocess.CompletedProcess([], 0, output, ""), subprocess.CompletedProcess([], 0, "", "")]
                with (
                    patch.object(module, "_container_program", return_value=("wslc.exe", True)),
                    patch.object(module.subprocess, "run", side_effect=results) as run,
                    contextlib.redirect_stdout(io.StringIO()),
                ):
                    module.kill_buckyos_containers()
                self.assertEqual(run.call_args_list[0].args[0], ["wslc.exe", "container", "ls", "-a", "--format", "json"])
                if output in ("[]", ""):
                    self.assertEqual(run.call_count, 1)
                else:
                    self.assertEqual(run.call_args_list[1].args[0], ["wslc.exe", "rm", "-f", "buckyos-app-one", "buckyos-app-two"])

    def test_failed_list_does_not_remove_containers(self):
        module = load_stop()
        with (
            patch.object(module, "_container_program", return_value=("wslc.exe", True)),
            patch.object(module.subprocess, "run", return_value=subprocess.CompletedProcess([], 1, "", "session unavailable")) as run,
            contextlib.redirect_stdout(io.StringIO()),
        ):
            module.kill_buckyos_containers()
        self.assertEqual(run.call_count, 1)

    def test_windows_selection_and_explicit_override(self):
        module = load_stop()
        with (
            patch.object(module, "system", "Windows"),
            patch.object(module.shutil, "which", return_value=r"C:\Program Files\WSL\wslc.exe"),
            patch.dict(module.os.environ, {}, clear=True),
        ):
            self.assertEqual(module._container_program(), (r"C:\Program Files\WSL\wslc.exe", True))
            module.os.environ["BUCKYOS_CONTAINER_RUNTIME"] = "docker"
            self.assertEqual(module._container_program(), ("docker", False))
            module.os.environ["BUCKYOS_CONTAINER_RUNTIME"] = "invalid"
            with self.assertRaises(ValueError):
                module._container_program()
