import contextlib
import importlib.util
import io
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


REPO = Path(__file__).parents[3]
SRC = REPO / "src"

FAKE_DOCKER = """#!/bin/sh
printf 'docker sudoed=%s %s\\n' "${FAKE_SUDOED:-0}" "$*" >> "$FAKE_LOG"
case "$1" in
  version)
    if [ -z "$FAKE_SUDOED" ] && [ -n "$FAKE_NO_SOCKET" ]; then
      echo 'permission denied while trying to connect to the Docker daemon socket' >&2
      exit 1
    fi
    ;;
  buildx)
    [ "$2" = inspect ] && echo 'Platforms: linux/amd64, linux/arm64'
    ;;
esac
exit 0
"""

FAKE_SUDO = """#!/bin/sh
printf 'sudo %s\\n' "$*" >> "$FAKE_LOG"
[ -n "$FAKE_SUDO_DRY" ] && exit 0
[ -n "$FAKE_SUDO_GRANT" ] && chmod u+w "$FAKE_SUDO_GRANT"
FAKE_SUDOED=1 exec "$@"
"""

DOCKER_FUNCTIONS = "/^DOCKER_NEEDS_SUDO=0/p;/^docker() {/,/^}/p;/^resolve_docker_access() {/,/^}/p"


def load(path: Path, name: str):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


@unittest.skipIf(os.name == "nt", "bash scripts")
class ShellScriptTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.tmp = Path(temporary.name)
        fake_bin = self.tmp / "bin"
        fake_bin.mkdir()
        for name, body in (("docker", FAKE_DOCKER), ("sudo", FAKE_SUDO)):
            (fake_bin / name).write_text(body, encoding="utf-8")
            (fake_bin / name).chmod(0o755)
        self.log = self.tmp / "calls.log"
        self.env = {
            **os.environ,
            "PATH": f"{fake_bin}:{os.environ['PATH']}",
            "FAKE_LOG": str(self.log),
        }

    def calls(self) -> list[str]:
        return self.log.read_text(encoding="utf-8").splitlines() if self.log.exists() else []

    def bash(self, script: str, **extra_env: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["bash", "-c", f"set -euo pipefail\n{script}"],
            env={**self.env, **extra_env},
            capture_output=True,
            text=True,
        )

    def build_exttool(self, **extra_env: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [str(REPO / "build_exttool"), "--image", "local/test", "--tag", "t", "--arch", "amd64"],
            env={**self.env, **extra_env},
            capture_output=True,
            text=True,
        )

    def test_build_exttool_uses_sudo_only_for_docker_without_socket_access(self) -> None:
        result = self.build_exttool(FAKE_NO_SOCKET="1")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("run through sudo", result.stdout)
        calls = self.calls()
        self.assertEqual(calls[0], "docker sudoed=0 version")
        docker_calls = [call for call in calls[1:] if call.startswith("docker ")]
        self.assertTrue(any("buildx build" in call and "--load" in call for call in docker_calls))
        self.assertTrue(all(call.startswith("docker sudoed=1 ") for call in docker_calls))
        self.assertEqual(calls[1], "sudo true")
        sudo_calls = [call for call in calls[2:] if call.startswith("sudo ")]
        self.assertEqual(len(sudo_calls), len(docker_calls))
        self.assertTrue(all(call.split()[1].endswith("/docker") for call in sudo_calls))

    def test_refused_sudo_stops_before_any_build(self) -> None:
        fake = Path(self.env["PATH"].split(":")[0]) / "sudo"
        fake.write_text('#!/bin/sh\nprintf "sudo %s\\n" "$*" >> "$FAKE_LOG"\nexit 1\n', encoding="utf-8")
        result = self.build_exttool(FAKE_NO_SOCKET="1")
        self.assertEqual(result.returncode, 1)
        self.assertIn("sudo was refused", result.stderr)
        self.assertEqual(self.calls(), ["docker sudoed=0 version", "sudo true"])

    def test_build_exttool_never_uses_sudo_with_socket_access(self) -> None:
        result = self.build_exttool()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn("sudo", result.stdout)
        self.assertFalse([call for call in self.calls() if call.startswith("sudo ")])
        self.assertTrue(any("buildx build" in call for call in self.calls()))

    def test_every_script_routes_docker_the_same_way(self) -> None:
        for script in (REPO / "build_aios", REPO / "build_exttool", SRC / "debug_jarvis.sh"):
            with self.subTest(script=script.name):
                self.log.unlink(missing_ok=True)
                body = f"""
log() {{ :; }}
PUSH=0
eval "$(sed -n '{DOCKER_FUNCTIONS}' '{script}')"
resolve_docker_access
captured="$(docker image inspect 'a b')"
docker ps
"""
                result = self.bash(body, FAKE_NO_SOCKET="1")
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(
                    [call for call in self.calls() if call.startswith("docker ")],
                    ["docker sudoed=0 version", "docker sudoed=1 image inspect a b", "docker sudoed=1 ps"],
                )

                self.log.unlink()
                result = self.bash(body)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(
                    self.calls(),
                    ["docker sudoed=0 version", "docker sudoed=0 image inspect a b", "docker sudoed=0 ps"],
                )

    def test_daemon_being_down_does_not_trigger_sudo(self) -> None:
        fake = Path(self.env["PATH"].split(":")[0]) / "docker"
        fake.write_text("#!/bin/sh\necho 'Cannot connect to the Docker daemon' >&2\nexit 1\n", encoding="utf-8")
        result = self.bash(
            f"""
log() {{ :; }}
PUSH=0
eval "$(sed -n '{DOCKER_FUNCTIONS}' '{REPO / "build_aios"}')"
resolve_docker_access
echo "needs_sudo=$DOCKER_NEEDS_SUDO"
"""
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("needs_sudo=0", result.stdout)
        self.assertEqual(self.calls(), [])

    def run_use_image(self, root: Path, **extra_env: str) -> subprocess.CompletedProcess[str]:
        return self.bash(
            f"""
log() {{ :; }}
IMAGE_NAME=local/test
BUCKYOS_ROOT='{root}'
eval "$(sed -n '/^use_image_in_local_buckyos() {{/,/^}}/p' '{REPO / "build_aios"}')"
use_image_in_local_buckyos python3
""",
            **extra_env,
        )

    def test_devenv_update_stays_unprivileged_while_writable(self) -> None:
        etc = self.tmp / "root" / "etc"
        etc.mkdir(parents=True)
        result = self.run_use_image(etc.parent)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("local/test", (etc / "devenv.json").read_text(encoding="utf-8"))
        self.assertEqual(self.calls(), [])

    @unittest.skipIf(hasattr(os, "geteuid") and os.geteuid() == 0, "root ignores file modes")
    def test_devenv_update_uses_sudo_only_when_not_writable(self) -> None:
        etc = self.tmp / "root" / "etc"
        etc.mkdir(parents=True)
        config = etc / "devenv.json"
        config.write_text('{"exttool": "local/keep"}', encoding="utf-8")
        config.chmod(stat.S_IRUSR)
        result = self.run_use_image(etc.parent, FAKE_SUDO_GRANT=str(config))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(self.calls()), 1)
        self.assertTrue(self.calls()[0].startswith("sudo python3 - "))
        content = config.read_text(encoding="utf-8")
        self.assertIn('"aios": "local/test"', content)
        self.assertIn("local/keep", content)

    def run_take_back(self, root: Path, uid: str) -> subprocess.CompletedProcess[str]:
        return self.bash(
            f"""
BUCKYOS_ROOT='{root}'
OWNER_USER_ID=devtest
APP_ID=agent.example
id() {{
  case "$1" in
    -u) echo {uid} ;;
    -un) echo dev ;;
    -g) echo {uid} ;;
  esac
}}
eval "$(sed -n '/^take_back_agent_root() {{/,/^}}/p' '{SRC / "debug_jarvis.sh"}')"
take_back_agent_root
""",
            FAKE_SUDO_DRY="1",
        )

    def test_agent_root_is_taken_back_only_when_someone_else_owns_it(self) -> None:
        root = self.tmp / "root"
        agent_root = root / "data/home/devtest/agents/agent.example"

        result = self.run_take_back(root, str(os.getuid()))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(agent_root.is_dir())
        self.assertEqual(self.calls(), [])

        other_uid = str(os.getuid() + 1)
        result = self.run_take_back(root, other_uid)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.calls(), [f"sudo chown -R {other_uid}:{other_uid} {agent_root}"])

    def test_scripts_parse(self) -> None:
        for script in (
            REPO / "build_aios",
            REPO / "build_exttool",
            SRC / "debug_jarvis.sh",
            SRC / "remove_open_dan_data.sh",
        ):
            with self.subTest(script=script.name):
                result = subprocess.run(["bash", "-n", str(script)], capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stderr)


class PythonDockerCommandTests(unittest.TestCase):
    def check(self, command) -> None:
        for stderr, euid, expected, probes in (
            ("permission denied while trying to connect", 1000, ["sudo", "docker"], 2),
            ("Cannot connect to the Docker daemon", 1000, ["docker"], 1),
            ("", 1000, ["docker"], 1),
            ("permission denied", 0, ["docker"], 0),
        ):
            with self.subTest(stderr=stderr, euid=euid):
                def run_command(args, **kwargs):
                    if args == ["sudo", "true"]:
                        return subprocess.CompletedProcess(args, 0)
                    return subprocess.CompletedProcess(args, 1 if stderr else 0, "", stderr)

                with (
                    patch.object(os, "geteuid", return_value=euid, create=True),
                    patch.object(subprocess, "run", side_effect=run_command) as run,
                    contextlib.redirect_stdout(io.StringIO()),
                ):
                    self.assertEqual(command(), expected)
                self.assertEqual(run.call_count, probes)

    def check_refused_sudo(self, command) -> None:
        def run_command(args, **kwargs):
            if args == ["sudo", "true"]:
                return subprocess.CompletedProcess(args, 1)
            return subprocess.CompletedProcess(args, 1, "", "permission denied")

        with (
            patch.object(os, "geteuid", return_value=1000, create=True),
            patch.object(subprocess, "run", side_effect=run_command),
            contextlib.redirect_stdout(io.StringIO()),
            self.assertRaisesRegex(RuntimeError, "sudo was refused"),
        ):
            command()

    def test_refused_sudo_is_reported(self) -> None:
        self.check_refused_sudo(load(REPO / "build.py", "buckyos_root_build").docker_command)
        self.check_refused_sudo(load(REPO / "make_local_pkg.py", "buckyos_make_local_pkg")._docker_command)

    def test_build_py(self) -> None:
        self.check(load(REPO / "build.py", "buckyos_root_build").docker_command)

    def test_make_local_pkg(self) -> None:
        self.check(load(REPO / "make_local_pkg.py", "buckyos_make_local_pkg")._docker_command)


class SetHostTests(unittest.TestCase):
    def write(self, module, system: str, returncode: int = 0):
        with (
            patch("builtins.open", side_effect=PermissionError),
            patch.object(module.platform, "system", return_value=system),
            patch.object(module.subprocess, "run", return_value=subprocess.CompletedProcess([], returncode)) as run,
            contextlib.redirect_stdout(io.StringIO()),
        ):
            module.write_hosts_file(Path("/etc/hosts"), ["a\n", "b\n"])
        return run

    def test_only_the_write_goes_through_sudo(self) -> None:
        run = self.write(load(SRC / "set_host.py", "buckyos_set_host"), "Linux")
        self.assertEqual(run.call_args.args[0], ["sudo", "tee", str(Path("/etc/hosts"))])
        self.assertEqual(run.call_args.kwargs["input"], "a\nb\n")

    def test_refused_sudo_is_reported(self) -> None:
        with self.assertRaises(SystemExit) as error:
            self.write(load(SRC / "set_host.py", "buckyos_set_host"), "Linux", returncode=1)
        self.assertEqual(error.exception.code, 1)

    def test_windows_never_tries_sudo(self) -> None:
        with self.assertRaises(SystemExit):
            self.write(load(SRC / "set_host.py", "buckyos_set_host"), "Windows")


class UninstallTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load(SRC / "uninstall.py", "buckyos_uninstall")

    def identity(self, euid: int, system: str = "Linux"):
        return (
            patch.object(self.module.platform, "system", return_value=system),
            patch.object(os, "geteuid", return_value=euid, create=True),
        )

    def remove_denied(self, euid: int, sudo_returncode: int = 0):
        results = []
        system, geteuid = self.identity(euid)
        with (
            tempfile.TemporaryDirectory() as temporary,
            system,
            geteuid,
            patch.object(self.module.shutil, "rmtree", side_effect=PermissionError("denied")),
            patch.object(
                self.module.subprocess,
                "run",
                return_value=subprocess.CompletedProcess([], sudo_returncode),
            ) as run,
        ):
            self.module._remove_fs_path(Path(temporary), results)
            return results[0], run, temporary

    def test_root_owned_files_are_removed_through_sudo(self) -> None:
        result, run, path = self.remove_denied(1000)
        run.assert_called_once_with(["sudo", "rm", "-rf", "--", path])
        self.assertEqual(result.status, "ok")

    def test_refused_sudo_is_reported_as_failed(self) -> None:
        result, _, _ = self.remove_denied(1000, sudo_returncode=1)
        self.assertEqual(result.status, "failed")

    def test_root_does_not_retry_through_sudo(self) -> None:
        result, run, _ = self.remove_denied(0)
        run.assert_not_called()
        self.assertEqual(result.status, "failed")

    def stop_command(self, euid: int) -> tuple[list[str], str]:
        system, geteuid = self.identity(euid)
        with (
            tempfile.TemporaryDirectory() as temporary,
            system,
            geteuid,
            patch.object(
                self.module,
                "_run_command",
                return_value=subprocess.CompletedProcess([], 0, "", ""),
            ) as run,
        ):
            root = Path(temporary)
            (root / "bin").mkdir()
            (root / "bin" / "stop.py").write_text("", encoding="utf-8")
            self.module._run_installed_stop_script(root, [])
            return run.call_args.args[0], temporary

    def test_linux_developer_stops_the_root_daemon_through_sudo(self) -> None:
        command, root = self.stop_command(1000)
        self.assertEqual(command[:3], ["sudo", "env", f"BUCKYOS_ROOT={root}"])
        self.assertEqual(command[3], "python3")

    def test_root_stops_directly(self) -> None:
        command, _ = self.stop_command(0)
        self.assertEqual(command[0], "python3")


if __name__ == "__main__":
    unittest.main()
