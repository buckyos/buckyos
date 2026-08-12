from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[3]))

import make_local_pkg


class MakeLocalPkgPrepareTests(unittest.TestCase):
    def test_installs_buckyos_before_dependency_components(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            cyfs_src = root / "cyfs-gateway" / "src"
            cyfs_src.mkdir(parents=True)
            target = make_local_pkg.TargetScript(
                platform_key="linux",
                script_path=Path("make_local_deb.py"),
                architecture="amd64",
                build_root=root / "stage",
            )
            calls: list[tuple[list[str], Path | None]] = []

            def fake_run_checked(
                cmd: list[str], *, cwd: Path | None = None, dry_run: bool = False
            ) -> None:
                calls.append((cmd, cwd))

            with (
                patch.object(make_local_pkg, "CYFS_SRC_DIR", cyfs_src),
                patch.object(make_local_pkg, "_run_checked", side_effect=fake_run_checked),
                patch.object(make_local_pkg, "_stage_desktop_app"),
            ):
                make_local_pkg._prepare_common_build_root(
                    target=target,
                    dry_run=True,
                    skip_cargo_update=True,
                    skip_cyfs_gateway=False,
                    desktop_app=None,
                    skip_desktop_app_build=True,
                    rust_target="x86_64-unknown-linux-musl",
                )

        commands = [cmd for cmd, _ in calls]
        buckyos_install = [
            "buckyos-install",
            "--all",
            f"--target-rootfs={target.build_root / 'buckyos'}",
            "--app=buckyos",
        ]
        dependency_install = [
            "buckyos-install",
            "--all",
            f"--target-rootfs={target.build_root / 'buckyos'}",
            "--app=cyfs-gateway",
        ]
        make_config_index = next(i for i, cmd in enumerate(commands) if "make_config.py" in cmd)

        self.assertLess(commands.index(buckyos_install), commands.index(dependency_install))
        self.assertLess(commands.index(dependency_install), make_config_index)


if __name__ == "__main__":
    unittest.main()
