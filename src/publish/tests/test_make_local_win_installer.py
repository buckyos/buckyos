import tempfile
import unittest
from pathlib import Path

import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import make_local_win_installer as winpkg  # noqa: E402


class WindowsPackagerTests(unittest.TestCase):
    def test_windows_cmd_fallback_keeps_cmd_filename(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            src = root / "src"
            dst = root / "dst"
            src.mkdir()
            (src / "buckyos.cmd").write_bytes(b"@echo off")

            layout = winpkg.AppLayout(
                source_rootfs=src,
                target_rootfs=Path("C:/BuckyOS"),
                module_paths=["buckyos"],
                data_paths=[],
                clean_paths=[],
                module_source_paths={"buckyos": str(src / "buckyos")},
                data_source_paths={},
            )

            winpkg._stage_buckyos_app_root(src_root=src, dst_root=dst, layout=layout)

            self.assertTrue((dst / "buckyos.cmd").is_file())
            self.assertFalse((dst / "buckyos").exists())

    def test_windows_exe_fallback_keeps_exe_filename(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            src = root / "src"
            dst = root / "dst"
            src.mkdir()
            (src / "tool.exe").write_bytes(b"fake exe")

            layout = winpkg.AppLayout(
                source_rootfs=src,
                target_rootfs=Path("C:/BuckyOS/tool"),
                module_paths=["tool"],
                data_paths=[],
                clean_paths=[],
                module_source_paths={"tool": str(src / "tool")},
                data_source_paths={},
            )

            winpkg._stage_buckyos_app_root(src_root=src, dst_root=dst, layout=layout)

            self.assertTrue((dst / "tool.exe").is_file())
            self.assertFalse((dst / "tool").exists())

    def test_buckyosapp_is_stopped_before_install_reinstall_and_uninstall(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            payload = root / "payload"
            app_payload = payload / "BuckyOSApp"
            app_payload.mkdir(parents=True)
            (app_payload / "buckyosapp.exe").write_bytes(b"fake exe")
            out_path = root / "installer.nsi"

            winpkg.generate_nsis_script(
                title="BuckyOS",
                version="0.7.0+test",
                architecture="amd64",
                components=[
                    winpkg.PublishComponent(
                        key="BuckyOSApp",
                        name="BuckyOS App",
                        kind="bundle",
                        optional=True,
                        default_selected=True,
                        src="buckyosapp.exe",
                        default_target="C:\\BuckyOS\\BuckyOSApp",
                        system_service=False,
                    )
                ],
                payload_dir=payload,
                out_path=out_path,
            )

            script = out_path.read_text(encoding="utf-8-sig")
            self.assertIn("Function StopBuckyOSApp", script)
            self.assertIn("Function un.StopBuckyOSApp", script)
            self.assertEqual(script.count("taskkill /F /T /IM buckyosapp.exe >nul 2>&1"), 2)

            install_section = script.index('Section "BuckyOS App" SEC_BUCKYOSAPP')
            install_stop = script.index("Call StopBuckyOSApp", install_section)
            install_extract = script.index("File /r", install_stop)
            self.assertLess(install_stop, install_extract)

            uninstall_section = script.index("Section Uninstall")
            uninstall_stop = script.index("Call un.StopBuckyOSApp", uninstall_section)
            uninstall_remove = script.index('RMDir /r "$InstDir_buckyosapp"', uninstall_stop)
            self.assertLess(uninstall_stop, uninstall_remove)

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

    def test_service_prerequisites_are_silent_and_non_listening(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            payload = root / "payload"
            service_bin = payload / "buckyos" / "bin"
            service_bin.mkdir(parents=True)
            (service_bin / "stop.ps1").write_text("exit 0\n", encoding="utf-8")
            bundled_vcredist = root / "vcredist_x64.exe"
            bundled_vcredist.write_bytes(b"fake vcredist")
            out_path = root / "installer.nsi"

            winpkg.generate_nsis_script(
                title="BuckyOS",
                version="0.7.0+test",
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
                bundled_vcredist=bundled_vcredist,
            )

            script = out_path.read_text(encoding="utf-8-sig")
            self.assertIn("SetCompressor lzma", script)
            self.assertNotIn("SetCompressor /SOLID", script)
            self.assertIn('/install /quiet /norestart', script)
            self.assertIn('StrCmp $VCRedistInstallCode 1641 vcredist_install_reboot', script)
            self.assertIn('StrCmp $VCRedistInstallCode 3010 vcredist_install_reboot', script)
            self.assertIn('Call InstallBundledVCRedist', script)
            self.assertNotIn('Install or repair it now?', script)
            self.assertNotIn('Please re-run setup.', script)
            self.assertIn("File /oname=stop.ps1", script)
            self.assertIn("$PLUGINSDIR\\stop.ps1", script)
            self.assertNotIn("buckyos_stop.ps1", script)
            self.assertIn("schtasks /Query /TN BuckyOSNodeDaemonKeepAlive", script)
            self.assertIn('&& schtasks /Delete', script)
            self.assertNotIn("nsExec::ExecToLog 'schtasks /Delete", script)
            self.assertIn("GetActiveTcpListeners()", script)
            self.assertNotIn("TcpListener]::new", script)
            self.assertNotIn("IPAddress]::Any", script)

    def test_buckyos_postinstall_does_not_delete_missing_task(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            payload = Path(td) / "buckyos"

            winpkg._stage_windows_hooks("buckyos", payload)

            hook = payload / "scripts" / "hooks" / "postinstall.ps1"
            text = hook.read_text(encoding="utf-8")

            self.assertNotIn("schtasks.exe /Delete", text)
            self.assertIn("schtasks.exe /Create", text)
            self.assertIn("/F /TR $RunCommand", text)

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


if __name__ == "__main__":
    unittest.main()
