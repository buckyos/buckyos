import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch


SCRIPT_PATH = Path(__file__).parents[2] / "stop.py"


def load_stop():
    spec = importlib.util.spec_from_file_location("buckyos_stop", SCRIPT_PATH)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class StopPrivilegeTests(unittest.TestCase):
    def test_linux_developer_stops_the_root_daemon_through_sudo(self):
        module = load_stop()
        with (
            patch.object(module, "system", "Linux"),
            patch.object(module.os, "geteuid", return_value=1000, create=True),
            patch.object(module, "kill_all") as kill_all,
            patch.object(module.subprocess, "run") as run,
            patch.dict(module.os.environ, {"BUCKYOS_ROOT": "/r", "BUCKYOS_THIS_DEVICE": "d", "PATH": "/home/dev/bin"}),
        ):
            module.main()
        kill_all.assert_not_called()
        command = run.call_args.args[0]
        self.assertEqual(command[:2], ["sudo", "env"])
        self.assertIn("BUCKYOS_ROOT=/r", command)
        self.assertIn("BUCKYOS_THIS_DEVICE=d", command)
        self.assertFalse(any(part.startswith("PATH=") for part in command))
        self.assertEqual(command[-3:], [module.sys.executable, "-B", str(SCRIPT_PATH.resolve())])

    def test_root_and_other_platforms_stop_directly(self):
        for platform_name, euid in (("Linux", 0), ("Darwin", 501), ("Windows", 1000)):
            with self.subTest(platform=platform_name, euid=euid):
                module = load_stop()
                with (
                    patch.object(module, "system", platform_name),
                    patch.object(module.os, "geteuid", return_value=euid, create=True),
                    patch.object(module, "kill_all") as kill_all,
                    patch.object(module.subprocess, "run") as run,
                ):
                    module.main()
                kill_all.assert_called_once_with()
                run.assert_not_called()

    @unittest.skipIf(os.name == "nt", "Linux kevent ring")
    def test_only_another_users_kevent_ring_is_removed(self):
        module = load_stop()
        with tempfile.TemporaryDirectory() as temporary:
            ring = Path(temporary) / "buckyos_kevent_ringbuffer_v2.shm"
            unrelated = Path(temporary) / "other.shm"
            for path in (ring, unrelated):
                path.write_bytes(b"x")
            owner = ring.stat().st_uid
            with (
                patch.object(module, "system", "Linux"),
                patch.object(module, "KEVENT_RING_DIR", Path(temporary)),
                contextlib.redirect_stdout(io.StringIO()),
            ):
                with patch.object(module.os, "geteuid", return_value=owner):
                    module.remove_foreign_kevent_ring()
                self.assertTrue(ring.exists())
                with patch.object(module.os, "geteuid", return_value=owner + 1):
                    module.remove_foreign_kevent_ring()
                self.assertFalse(ring.exists())
                self.assertTrue(unrelated.exists())


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
