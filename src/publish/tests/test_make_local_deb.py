import tempfile
import unittest
from pathlib import Path

import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import make_local_deb as debpkg  # noqa: E402


class DebianPackagerTests(unittest.TestCase):
    def _write_project(self, path: Path) -> None:
        path.write_text(
            """
name: buckyos
version: "0.6.0"
base_dir: "."
apps:
  buckyos:
    name: buckyos
    rootfs: rootfs/
    default_target_rootfs: "${BUCKYOS_ROOT}"
    modules:
      node_daemon: bin/node-daemon/
      stop.py: bin/stop.py
    data_paths:
      - etc/node_gateway.json
      - data/
    clean_paths: []
publish:
  linux_pkg:
    apps:
      buckyos:
        name: BuckyOS Service
        type: app
        optional: true
        default_selected: true
        default_target: "/opt/buckyos/"
""",
            encoding="utf-8",
        )

    def test_materialize_deb_control_dir_from_deb_pkg_sources(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            project = root / "bucky_project.yaml"
            self._write_project(project)
            debian_dir = root / "DEBIAN"

            debpkg.materialize_deb_control_dir(
                project,
                debian_dir,
                version="0.6.0+test",
                architecture="x86_64",
            )

            control = (debian_dir / "control").read_text(encoding="utf-8")
            preinst = (debian_dir / "preinst").read_text(encoding="utf-8")
            postinst = (debian_dir / "postinst").read_text(encoding="utf-8")

            self.assertIn("Version: 0.6.0+test", control)
            self.assertIn("Architecture: amd64", control)
            self.assertIn("# BEGIN AUTO-GENERATED: component_preinstall_hooks", preinst)
            self.assertIn("BEGIN COMPONENT HOOK: buckyos_preinstall", preinst)
            self.assertIn('rm -rf "$BUCKYOS_ROOT/bin/node-daemon"', preinst)
            self.assertIn('rm -rf "$BUCKYOS_ROOT/bin/stop.py"', preinst)
            self.assertIn("# BEGIN AUTO-GENERATED: component_postinstall_hooks", postinst)
            self.assertIn("BEGIN COMPONENT HOOK: buckyos_postinstall", postinst)
            self.assertIn('cp -p "$DEFAULTS_DIR/etc/node_gateway.json" "$BUCKYOS_ROOT/etc/node_gateway.json"', postinst)
            self.assertIn('cp -a "$DEFAULTS_DIR/data/." "$BUCKYOS_ROOT/data/"', postinst)
            self.assertIn("ExecStart=/opt/buckyos/bin/node-daemon/node_daemon --enable_active", postinst)

    def test_linux_hooks_are_discovered_from_deb_pkg(self) -> None:
        hook = debpkg._discover_linux_hook("buckyos", "preinstall")

        self.assertIsNotNone(hook)
        self.assertEqual(hook.name, "buckyos_preinstall")
        self.assertEqual(hook.parent.name, "deb_pkg")

    def test_render_control_command_writes_final_debian_control_files(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            project = root / "bucky_project.yaml"
            out_dir = root / "rendered"
            self._write_project(project)

            rc = debpkg.main(
                [
                    "make_local_deb.py",
                    "render-control",
                    "x86_64",
                    "0.6.0+test",
                    "--project",
                    str(project),
                    "--out-dir",
                    str(out_dir),
                ]
            )

            self.assertEqual(rc, 0)
            self.assertIn("Architecture: amd64", (out_dir / "control").read_text(encoding="utf-8"))
            self.assertTrue((out_dir / "preinst").stat().st_mode & 0o111)
            self.assertTrue((out_dir / "postinst").stat().st_mode & 0o111)


if __name__ == "__main__":
    unittest.main()
