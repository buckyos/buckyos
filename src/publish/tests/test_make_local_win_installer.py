import tempfile
import unittest
from pathlib import Path

import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import make_local_win_installer as winpkg  # noqa: E402


class WindowsPackagerTests(unittest.TestCase):
    def test_windows_exe_fallback_keeps_exe_filename(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            src = root / "src"
            dst = root / "dst"
            src.mkdir()
            (src / "buckycli.exe").write_bytes(b"fake exe")

            layout = winpkg.AppLayout(
                source_rootfs=src,
                target_rootfs=Path("C:/BuckyOS/buckycli"),
                module_paths=["buckycli"],
                data_paths=[],
                clean_paths=[],
                module_source_paths={"buckycli": str(src / "buckycli")},
                data_source_paths={},
            )

            winpkg._stage_buckyos_app_root(src_root=src, dst_root=dst, layout=layout)

            self.assertTrue((dst / "buckycli.exe").is_file())
            self.assertFalse((dst / "buckycli").exists())

    def test_buckycli_hooks_are_called_for_path_lifecycle(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            payload = root / "payload"
            component_payload = payload / "buckycli"
            hooks_dir = component_payload / "scripts" / "hooks"
            hooks_dir.mkdir(parents=True)
            (component_payload / "buckycli.exe").write_bytes(b"fake exe")
            (hooks_dir / "postinstall.ps1").write_text("exit 0\n", encoding="utf-8")
            (hooks_dir / "preuninstall.ps1").write_text("exit 0\n", encoding="utf-8")

            out_path = root / "installer.nsi"
            winpkg.generate_nsis_script(
                title="BuckyOS",
                version="0.6.0+test",
                architecture="amd64",
                components=[
                    winpkg.PublishComponent(
                        key="buckycli",
                        name="BuckyOS CLI",
                        kind="app",
                        optional=True,
                        default_selected=True,
                        src=None,
                        default_target="C:\\BuckyOS\\buckycli\\",
                        system_service=False,
                    )
                ],
                payload_dir=payload,
                out_path=out_path,
            )

            script = out_path.read_text(encoding="utf-8-sig")
            self.assertIn("; Run buckycli postinstall hook", script)
            self.assertIn("; Run buckycli preuninstall hook", script)
            self.assertIn('powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$InstDir_buckycli\\scripts\\hooks\\postinstall.ps1"', script)
            self.assertIn('powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$InstDir_buckycli\\scripts\\hooks\\preuninstall.ps1"', script)
            self.assertIn('SendMessage ${HWND_BROADCAST} ${WM_WININICHANGE} 0 "STR:Environment" /TIMEOUT=5000', script)
            self.assertIn('StrCpy $InstallerLogPath "$TEMP\\buckyos-windows-${PRODUCT_ARCH}-${PRODUCT_VERSION}-uninstall.log"', script)

    def test_stage_windows_hooks_includes_buckycli_preuninstall(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            payload = Path(td) / "buckycli"

            winpkg._stage_windows_hooks("buckycli", payload)

            self.assertTrue((payload / "scripts" / "hooks" / "postinstall.ps1").is_file())
            self.assertTrue((payload / "scripts" / "hooks" / "preuninstall.ps1").is_file())

    def test_buckyos_task_lifecycle_uses_component_hooks(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            payload = root / "payload"
            component_payload = payload / "buckyos"
            hooks_dir = component_payload / "scripts" / "hooks"
            hooks_dir.mkdir(parents=True)
            (hooks_dir / "postinstall.ps1").write_text("exit 0\n", encoding="utf-8")
            (hooks_dir / "preuninstall.ps1").write_text("exit 0\n", encoding="utf-8")

            out_path = root / "installer.nsi"
            winpkg.generate_nsis_script(
                title="BuckyOS",
                version="0.6.0+test",
                architecture="amd64",
                components=[
                    winpkg.PublishComponent(
                        key="buckyos",
                        name="BuckyOS Service",
                        kind="app",
                        optional=True,
                        default_selected=True,
                        src=None,
                        default_target="C:\\BuckyOS\\",
                        system_service=True,
                    )
                ],
                payload_dir=payload,
                out_path=out_path,
            )

            script = out_path.read_text(encoding="utf-8-sig")
            self.assertIn("; Run buckyos postinstall hook", script)
            self.assertIn("; Run buckyos preuninstall hook", script)
            self.assertNotIn('schtasks /Create /TN "BuckyOSNodeDaemonKeepAlive"', script)
            self.assertNotIn('WriteRegStr HKCU "Software\\Microsoft\\Windows\\CurrentVersion\\Run" "BuckyOSDaemon"', script)

    def test_node_daemon_loader_vbs_no_longer_wraps_powershell(self) -> None:
        loader = Path(winpkg.__file__).resolve().parent / "win_pkg" / "scripts" / "node_daemon_loader.vbs"

        text = loader.read_text(encoding="utf-8")

        self.assertNotIn("powershell.exe", text.lower())
        self.assertIn("shell.CurrentDirectory", text)
        self.assertIn("Win32_Process", text)
        self.assertIn("--enable_active", text)

    def test_service_script_copy_excludes_hook_sources(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            dst = Path(td) / "scripts"

            winpkg._copy_windows_service_scripts(Path(winpkg.__file__).resolve().parent / "win_pkg" / "scripts", dst)

            self.assertTrue((dst / "node_daemon_loader.vbs").is_file())
            self.assertFalse((dst / "node_daemon_loader.ps1").exists())
            self.assertFalse((dst / "buckyos_postinstall.ps1").exists())
            self.assertFalse((dst / "buckycli_postinstall.ps1").exists())


if __name__ == "__main__":
    unittest.main()
