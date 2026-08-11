from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import package_common as common


class PackageCommonTests(unittest.TestCase):
    def test_parse_bool_rejects_ambiguous_strings(self) -> None:
        self.assertTrue(common.parse_bool("true", field_name="field"))
        self.assertFalse(common.parse_bool("false", field_name="field"))
        with self.assertRaises(ValueError):
            common.parse_bool("true,", field_name="field")

    def test_parse_component_type(self) -> None:
        self.assertEqual(common.parse_component_type("app", field_name="type"), "app")
        self.assertEqual(common.parse_component_type("bundle", field_name="type"), "bundle")
        with self.assertRaises(ValueError):
            common.parse_component_type("service", field_name="type")

    def test_arch_and_package_filename(self) -> None:
        self.assertEqual(common.canonical_arch("x86_64"), "amd64")
        self.assertEqual(common.canonical_arch("aarch64"), "arm64")
        self.assertEqual(
            common.package_filename(
                platform_key="macos",
                architecture="aarch64",
                version="0.6.0+build260528",
                package_format="pkg",
            ),
            "buckyos-macos-arm64-0.6.0+build260528.pkg",
        )

    def test_manifest_helpers(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            manifest = Path(td) / "manifest.json"
            manifest.write_text(
                json.dumps(
                    {
                        "platforms": {"linux": {"component_keys": ["buckyos"]}},
                        "install_projects": {
                            "buckyos": {
                                "module_items": [
                                    {
                                        "raw_path": "bin/node-daemon/",
                                        "source_path": "/stage/buckyos/bin/node-daemon",
                                    }
                                ],
                                "data_items": [],
                                "clean_items": [],
                            }
                        },
                    }
                ),
                encoding="utf-8",
            )
            project = common.manifest_install_project(manifest, "buckyos")
            self.assertEqual(common.manifest_component_keys(manifest, "linux"), ["buckyos"])
            self.assertEqual(
                common.item_paths(project, "module_items", project_key="buckyos"),
                ["bin/node-daemon/"],
            )
            self.assertEqual(
                common.item_source_paths(project, "module_items", project_key="buckyos"),
                {"bin/node-daemon/": "/stage/buckyos/bin/node-daemon"},
            )

    def test_discover_component_hook(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            scripts = Path(td)
            hook = scripts / "buckyos_preinstall.ps1"
            hook.write_text("exit 0\n", encoding="utf-8")
            self.assertEqual(
                common.discover_component_hook(
                    scripts_dirs=(scripts,),
                    component_key="buckyos",
                    step="preinstall",
                    extensions=(".ps1", ".cmd"),
                ),
                hook,
            )

    def test_source_path_for_prefers_manifest_mapping(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td) / "root"
            mapped = Path(td) / "merged" / "bin" / "cyfs-gateway"
            mapped.mkdir(parents=True)
            self.assertEqual(
                common.source_path_for(
                    source_rootfs=root,
                    rel="bin/cyfs-gateway/",
                    item_source_paths={"bin/cyfs-gateway/": str(mapped)},
                ),
                mapped.resolve(),
            )

    def test_source_path_for_windows_exe_fallback(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td) / "root"
            source_root_override = Path(td) / "stage" / "buckycli"
            exe = source_root_override / "buckycli.exe"
            exe.parent.mkdir(parents=True)
            exe.write_bytes(b"exe")

            self.assertEqual(
                common.source_path_for(
                    source_rootfs=root,
                    rel="buckycli",
                    item_source_paths={"buckycli": str(source_root_override / "buckycli")},
                    source_root_override=source_root_override,
                    windows=True,
                ),
                exe,
            )

    def test_source_path_for_does_not_add_exe_on_non_windows(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td) / "root"
            exe = root / "buckycli.exe"
            exe.parent.mkdir(parents=True)
            exe.write_bytes(b"exe")

            self.assertEqual(
                common.source_path_for(
                    source_rootfs=root,
                    rel="buckycli",
                    item_source_paths={},
                    windows=False,
                ),
                root / "buckycli",
            )

    def test_unexpected_payload_paths(self) -> None:
        unexpected = common.unexpected_payload_paths(
            [
                "./opt",
                "./opt/buckyos",
                "./opt/buckyos/bin/node-daemon/node_daemon",
                "./opt/buckyos/.buckyos_installer_defaults/etc/node_gateway.json",
                "./opt/buckyos/data/user.db",
                "./Applications/BuckyOS.app/Contents/Info.plist",
            ],
            allowed_prefixes=[
                "opt/buckyos/bin/node-daemon",
                "opt/buckyos/.buckyos_installer_defaults/etc/node_gateway.json",
            ],
            allowed_exact=["opt", "opt/buckyos", "opt/buckyos/.buckyos_installer_defaults"],
        )
        self.assertEqual(
            unexpected,
            [
                "Applications/BuckyOS.app/Contents/Info.plist",
                "opt/buckyos/data/user.db",
            ],
        )


if __name__ == "__main__":
    unittest.main()
