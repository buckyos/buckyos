import contextlib
import importlib.util
import io
from pathlib import Path
import subprocess
import unittest
from unittest.mock import patch


SCRIPT_PATH = Path(__file__).parents[2] / "rootfs" / "bin" / "stop.py"
IN_PROGRESS = "Error response from daemon: removal of container bbb is already in progress"


def load_stop():
    spec = importlib.util.spec_from_file_location("buckyos_rootfs_stop", SCRIPT_PATH)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class FakeDocker:
    """`docker ps -aq` lists aaa, bbb, ccc; `docker rm -f` answers with `rm`;
    each later `docker ps -a --format` lists the next entry of `states`."""

    def __init__(self, rm, states):
        self.rm = rm
        self.states = list(states)
        self.calls = []

    def __call__(self, args, **kwargs):
        self.calls.append(args)
        if args[:3] == ["docker", "ps", "-aq"]:
            return subprocess.CompletedProcess(args, 0, "aaa\nbbb\nccc\n", "")
        if args[:3] == ["docker", "rm", "-f"]:
            return self.rm
        if args[:3] == ["docker", "ps", "-a"]:
            state = self.states.pop(0)
            if state is None:
                return subprocess.CompletedProcess(args, 1, "", "Cannot connect to the Docker daemon")
            return subprocess.CompletedProcess(args, 0, "".join(f"{i} {s}\n" for i, s in state.items()), "")
        raise AssertionError(f"unexpected command {args}")


class RemoveContainersTests(unittest.TestCase):
    def run_stop(self, docker, clock=(0,)):
        module = load_stop()
        output = io.StringIO()
        with (
            patch.object(module, "system", "Linux"),
            patch.object(module, "_product_runs_containers", return_value=True),
            patch.object(module.shutil, "which", return_value="/usr/bin/docker"),
            patch.object(module.subprocess, "run", side_effect=docker),
            patch.object(module.time, "monotonic", side_effect=list(clock)),
            patch.object(module.time, "sleep") as sleep,
            contextlib.redirect_stdout(output),
        ):
            module.kill_buckyos_containers()
        return output.getvalue(), sleep

    def test_all_removed(self):
        docker = FakeDocker(subprocess.CompletedProcess([], 0, "aaa\nccc\n", ""), [])
        output, _ = self.run_stop(docker)
        self.assertEqual(output.splitlines(), [f"{i} container removed" for i in ("aaa", "bbb", "ccc")])
        self.assertEqual(len(docker.calls), 2)

    def test_removal_already_in_progress_that_finishes_is_success(self):
        rm = subprocess.CompletedProcess([], 1, "aaa\nccc\n", IN_PROGRESS + "\n")
        docker = FakeDocker(rm, [{"bbb": "removing"}, {"bbb": "removing"}, {}])
        output, sleep = self.run_stop(docker, clock=(0, 1, 2, 3))
        self.assertNotIn("Failed", output)
        self.assertEqual(output.splitlines(), [
            "aaa container removed",
            "ccc container removed",
            "bbb container removed (Docker was already removing it)",
        ])
        self.assertEqual(sleep.call_count, 2)

    def test_removal_still_in_progress_after_the_wait_is_failure(self):
        rm = subprocess.CompletedProcess([], 1, "aaa\nccc\n", IN_PROGRESS + "\n")
        docker = FakeDocker(rm, [{"bbb": "removing"}, {"bbb": "removing"}])
        output, _ = self.run_stop(docker, clock=(0, 10, 31))
        self.assertIn(f"Failed to remove buckyos docker containers: {IN_PROGRESS}", output)
        self.assertIn("bbb is still being removed after 30s", output)

    def test_other_errors_are_reported_without_waiting(self):
        rm = subprocess.CompletedProcess([], 1, "aaa\n", "Error response from daemon: cannot remove bbb\n")
        docker = FakeDocker(rm, [{"bbb": "dead", "ccc": "removing", "zzz": "running"}, {"bbb": "dead"}])
        output, sleep = self.run_stop(docker, clock=(0, 1, 2))
        self.assertIn("Failed to remove buckyos docker containers: Error response from daemon: cannot remove bbb", output)
        self.assertIn("bbb is still there (dead)", output)
        self.assertIn("ccc container removed (Docker was already removing it)", output)
        self.assertNotIn("zzz", output)
        self.assertEqual(sleep.call_count, 1)

    def test_unlisted_state_keeps_the_docker_error(self):
        rm = subprocess.CompletedProcess([], 1, "aaa\nccc\n", IN_PROGRESS + "\n")
        output, _ = self.run_stop(FakeDocker(rm, [None]), clock=(0,))
        self.assertEqual(output.splitlines()[-1], f"Failed to remove buckyos docker containers: {IN_PROGRESS}")


if __name__ == "__main__":
    unittest.main()
