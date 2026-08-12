from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import make_local_rpm as rpm


class MakeLocalRpmTests(unittest.TestCase):
    def test_rpm_arch_and_version_release(self) -> None:
        self.assertEqual(rpm._rpm_arch("amd64"), "x86_64")
        self.assertEqual(rpm._rpm_arch("arm64"), "aarch64")
        self.assertEqual(rpm._rpm_version_release("0.6.0+build260529"), ("0.6.0", "build260529"))
        self.assertEqual(rpm._rpm_version_release("0.6.0-dev+build-1"), ("0.6.0_dev", "build_1"))

    def test_render_spec_includes_linux_package_contract(self) -> None:
        layout = rpm.AppLayout(
            source_rootfs=Path("/stage/buckyos"),
            target_rootfs=Path("/opt/buckyos"),
            module_paths=["bin/node-daemon/"],
            data_paths=["etc/node_gateway.json", "data/"],
            clean_paths=[],
            module_source_paths={},
            data_source_paths={},
        )
        spec = rpm._render_spec(
            rpm_version="0.6.0",
            rpm_release="build260529",
            payload_tree=Path("/tmp/buckyos-payload"),
            layout=layout,
            component_keys=["buckyos"],
        )

        self.assertIn("Version: 0.6.0", spec)
        self.assertIn("Release: build260529", spec)
        self.assertNotIn("BuildArch:", spec)
        self.assertIn("Requires: (moby-engine or docker-ce or docker-engine)", spec)
        self.assertIn("BEGIN COMPONENT HOOK: buckyos_preinstall", spec)
        self.assertIn("BEGIN COMPONENT HOOK: buckyos_postinstall", spec)
        self.assertIn("# BEGIN AUTO-GENERATED: modules", spec)
        self.assertIn("# BEGIN AUTO-GENERATED: data_paths", spec)
        self.assertIn('rm -rf "$BUCKYOS_ROOT/bin/"', spec)
        self.assertIn('rm -rf "$BUCKYOS_ROOT/bin/node-daemon"', spec)
        self.assertIn('cp -p "$DEFAULTS_DIR/etc/node_gateway.json" "$BUCKYOS_ROOT/etc/node_gateway.json"', spec)
        self.assertIn('cp -a "$DEFAULTS_DIR/data/." "$BUCKYOS_ROOT/data/"', spec)
        self.assertIn("systemctl start buckyos.service", spec)
        self.assertIn(".buckyos_installer_defaults", spec)
        self.assertIn("%preun", spec)
        self.assertIn("%global __os_install_post %{nil}", spec)
        self.assertIn("%global _build_id_links none", spec)
        files_section = spec.split("%files", 1)[1]
        self.assertNotIn("/etc/systemd/system/buckyos.service", files_section)
        self.assertIn('cp -a /tmp/buckyos-payload/. "%{buildroot}/"', spec)

    def test_linux_hooks_are_discovered_from_rpm_pkg(self) -> None:
        hook = rpm._discover_linux_hook("buckyos", "preinstall")

        self.assertIsNotNone(hook)
        assert hook is not None
        self.assertEqual(hook.name, "buckyos_preinstall")
        self.assertEqual(hook.parent.name, "rpm_pkg")

    def test_verify_payload_contract_allows_declared_paths_only(self) -> None:
        layout = rpm.AppLayout(
            source_rootfs=Path("/stage/buckyos"),
            target_rootfs=Path("/opt/buckyos"),
            module_paths=["bin/node-daemon/"],
            data_paths=["etc/node_gateway.json"],
            clean_paths=[],
            module_source_paths={},
            data_source_paths={},
        )

        failures: list[str] = []
        rpm._verify_linux_payload_contract(
            payload_paths=[
                "/opt",
                "/opt/buckyos",
                "/opt/buckyos/bin",
                "/opt/buckyos/bin/node-daemon",
                "/opt/buckyos/bin/node-daemon/node_daemon",
                "/opt/buckyos/.buckyos_installer_defaults",
                "/opt/buckyos/.buckyos_installer_defaults/etc",
                "/opt/buckyos/.buckyos_installer_defaults/etc/node_gateway.json",
            ],
            layout=layout,
            failures=failures,
            package_kind="rpm",
            include_systemd_service=False,
        )
        self.assertEqual(failures, [])

        rpm._verify_linux_payload_contract(
            payload_paths=[
                "/etc/systemd/system/buckyos.service",
                "/opt/buckyos/data/user.db",
                "/Applications/BuckyOS.app/Contents/Info.plist",
            ],
            layout=layout,
            failures=failures,
            package_kind="rpm",
            include_systemd_service=False,
        )
        self.assertTrue(any("undeclared paths" in failure for failure in failures))
        self.assertTrue(any("BuckyOS.app" in failure for failure in failures))


if __name__ == "__main__":
    unittest.main()
