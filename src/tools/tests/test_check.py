import importlib.util
from pathlib import Path
import subprocess
import sys
import unittest
from unittest.mock import patch


SCRIPT_PATH = Path(__file__).parents[2] / "check.py"
SPEC = importlib.util.spec_from_file_location("buckyos_runtime_check", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
check = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = check
SPEC.loader.exec_module(check)


class RuntimeProcessTests(unittest.TestCase):
    def test_scheduler_excludes_proxmox_and_unrelated_arguments(self):
        processes = [
            check.ProcessInfo(1, "pvescheduler", "pvescheduler"),
            check.ProcessInfo(2, "bash", "bash -c /opt/buckyos/bin/scheduler/scheduler"),
            check.ProcessInfo(3, "scheduler", "/opt/buckyos/bin/scheduler/scheduler --boot"),
        ]
        self.assertEqual(
            [p.pid for p in check.find_processes(processes, check.PROCESS_ALIASES["scheduler"])],
            [3],
        )

    def test_collect_unix_processes_excludes_zombies(self):
        comm = "\n".join([
            "185050 Zs cyfs_gateway",
            "185084 Ssl system_config",
            "185121 Sl scheduler",
        ])
        args = "\n".join([
            "185050 [cyfs_gateway] <defunct>",
            "185084 /opt/buckyos/bin/system-config/system_config",
            "185121 /opt/buckyos/bin/scheduler/scheduler --boot",
        ])
        with (
            patch.object(check.platform, "system", return_value="Linux"),
            patch.object(check, "run_command", side_effect=[
                subprocess.CompletedProcess([], 0, comm),
                subprocess.CompletedProcess([], 0, args),
            ]) as run,
        ):
            processes = check.collect_processes()
        self.assertEqual(run.call_args_list[0].args[0], ["ps", "-axo", "pid=,stat=,comm="])
        self.assertEqual(run.call_args_list[1].args[0], ["ps", "-axo", "pid=,args="])
        self.assertEqual([p.pid for p in processes], [185084, 185121])
        self.assertEqual(processes[0].command, "system_config")
        self.assertEqual(processes[1].args, "/opt/buckyos/bin/scheduler/scheduler --boot")

    def test_macos_runtime_path_with_spaces_is_one_command(self):
        runtime = "/opt/buckyos/bin/node-daemon/BuckyOS Runtime.app/Contents/MacOS/node_daemon"
        with (
            patch.object(check.platform, "system", return_value="Darwin"),
            patch.object(check, "run_command", side_effect=[
                subprocess.CompletedProcess([], 0, f"501 Ss {runtime}"),
                subprocess.CompletedProcess([], 0, f"501 {runtime} --enable_active --desktop_daemon"),
            ]),
        ):
            processes = check.collect_processes()
        self.assertEqual(processes[0].command, runtime)
        self.assertEqual(
            [p.pid for p in check.find_processes(processes, check.PROCESS_ALIASES["node_daemon"])],
            [501],
        )

    def test_aliases_match_executable_names(self):
        for executable in ["cyfs_gateway", "cyfs-gateway", "cyfs_gateway.exe"]:
            with self.subTest(executable=executable):
                proc = check.ProcessInfo(1, executable, executable)
                self.assertTrue(check.process_matches(proc, check.PROCESS_ALIASES["cyfs_gateway"]))

    def test_argv_executable_matches_when_process_name_is_truncated(self):
        proc = check.ProcessInfo(1, "long-service-na", "/opt/buckyos/bin/long-service-name --serve")
        self.assertTrue(check.process_matches(proc, ["long-service-name"]))
