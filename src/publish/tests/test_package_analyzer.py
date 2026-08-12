from __future__ import annotations

import contextlib
import gzip
import io
import json
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import package_analyzer


class PackageAnalyzerTests(unittest.TestCase):
    def test_infer_old_and_new_package_names(self) -> None:
        self.assertEqual(
            package_analyzer.infer_from_filename(Path("buckyos-linux-arm64-0.6.0+build260529.deb")),
            {
                "project": "buckyos",
                "arch": "arm64",
                "version": "0.6.0+build260529",
                "format": "deb",
                "platform": "linux",
            },
        )
        self.assertEqual(
            package_analyzer.infer_from_filename(Path("buckyos-apple-aarch64-0.6.0+build260529.pkg"))["platform"],
            "macos",
        )
        self.assertEqual(
            package_analyzer.infer_from_filename(Path("buckyos-windows-amd64-0.6.0+build260529.exe"))["platform"],
            "windows",
        )

    def test_find_packages_is_not_recursive_by_default(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            direct = root / "buckyos-linux-amd64-0.6.0+build260529.deb"
            nested = root / "payload" / "buckycli.exe"
            direct.write_bytes(b"deb")
            nested.parent.mkdir()
            nested.write_bytes(b"exe")

            self.assertEqual(package_analyzer.find_packages(root, recursive=False), [direct])
            self.assertEqual(package_analyzer.find_packages(root, recursive=True), [direct, nested])

    def test_default_report_omits_runtime_environment_fields(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            package = root / "buckyos-linux-amd64-0.6.0+build260529.deb"
            package.write_bytes(b"package")

            original_analyze_package = package_analyzer.analyze_package

            def fake_analyze_package(path: Path, **kwargs) -> dict:
                return package_analyzer.base_package_record(path, include_hashes=True)

            try:
                package_analyzer.analyze_package = fake_analyze_package
                report = package_analyzer.build_report(
                    root,
                    include_hashes=True,
                    script_text_limit=1024,
                    recursive=False,
                )
            finally:
                package_analyzer.analyze_package = original_analyze_package

        self.assertEqual(set(report), {"schema_version", "package_count", "packages"})
        package_report = report["packages"][0]
        for key in (
            "path",
            "suffix",
            "mtime",
            "generated_at",
            "analyzer",
            "tools",
            "required_tools",
            "missing_tools",
            "warnings",
            "errors",
        ):
            self.assertNotIn(key, package_report)

    def test_tree_entries_omit_extraction_artifacts(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            (root / "opt" / "buckyos").mkdir(parents=True)
            (root / "opt" / "buckyos" / "service").write_bytes(b"binary")

            entries = package_analyzer.tree_entries(root, include_hashes=False)

        self.assertNotIn("", {entry["path"] for entry in entries})
        directory = next(entry for entry in entries if entry["path"] == "opt")
        self.assertEqual(directory["type"], "dir")
        self.assertNotIn("size_bytes", directory)
        file_entry = next(entry for entry in entries if entry["path"] == "opt/buckyos/service")
        self.assertEqual(file_entry["size_bytes"], 6)

    def test_collect_scripts_includes_extensionless_installer_hooks(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            (root / "postinst").write_text("#!/bin/sh\necho installed\n", encoding="utf-8")
            (root / "control").write_text("Package: buckyos\n", encoding="utf-8")

            scripts = package_analyzer.collect_scripts(root, limit_bytes=1024)

        self.assertIn("postinst", scripts)
        self.assertNotIn("control", scripts)

    def test_external_windows_nsis_script_is_recorded_without_7z(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            package = Path(td) / "buckyos-windows-amd64-0.6.0+build260529.exe"
            package.write_bytes(b"exe")
            script = package_analyzer.ExternalScript(
                name="installer.nsi",
                kind="windows-nsis-script",
                source="ssh:bucky@pve-windows-builder:C:/Users/bucky/buckyosci/win-installer/distbuild/installer.nsi",
                data=b'Name "BuckyOS"\nSection\nSectionEnd\n',
            )

            record = package_analyzer.analyze_package(
                package,
                tools={"7z": package_analyzer.ToolStatus(name="7z", path=None)},
                include_hashes=False,
                script_text_limit=1024,
                external_windows_scripts=[script],
            )

        scripts = record["analysis"]["installer_scripts"]
        self.assertEqual(scripts[0]["name"], "installer.nsi")
        self.assertEqual(scripts[0]["kind"], "windows-nsis-script")
        self.assertIn("SectionEnd", scripts[0]["text"])
        self.assertIn("exe payload inspection skipped", record["errors"][0])

    def test_required_tools_are_linux_only(self) -> None:
        tools = package_analyzer.required_tool_names()

        self.assertIn("xar", tools)
        self.assertIn("cpio", tools)
        self.assertIn("7z", tools)
        self.assertIn("gzip", tools)
        self.assertIn("xz", tools)
        self.assertIn("zstd", tools)
        self.assertNotIn("pkgutil", tools)

    def test_missing_tools_error_includes_apt_hint(self) -> None:
        message = package_analyzer.format_missing_tools_error(["xar", "7z"])

        self.assertIn("missing required analyzer tools", message)
        self.assertIn("xar (apt package: xar)", message)
        self.assertIn("7z (apt package: 7zip)", message)
        self.assertIn("sudo apt install -y", message)
        self.assertIn("p7zip-full", message)

    @unittest.skipIf(shutil.which("cpio") is None, "cpio is required")
    def test_extract_payload_cpio_handles_gzip_payload(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            src = root / "src"
            out = root / "out"
            src.mkdir()
            out.mkdir()
            (src / "hello.txt").write_text("hello\n", encoding="utf-8")
            archive = subprocess.run(
                ["cpio", "-o", "--format=newc"],
                cwd=src,
                input=b"hello.txt\n",
                capture_output=True,
                check=True,
            )
            payload = root / "Payload"
            payload.write_bytes(gzip.compress(archive.stdout))
            tools = {
                "cpio": package_analyzer.ToolStatus(name="cpio", path=shutil.which("cpio")),
                "gzip": package_analyzer.ToolStatus(name="gzip", path=shutil.which("gzip")),
            }

            rc, detail = package_analyzer.extract_payload_cpio(payload, out, tools=tools)

            self.assertEqual((rc, detail), (0, ""))
            self.assertEqual((out / "hello.txt").read_text(encoding="utf-8"), "hello\n")

    def test_pkg_xar_expands_nested_flat_component_package(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            package = root / "buckyos-apple-aarch64-0.6.0+build260603.pkg"
            package.write_bytes(b"outer pkg")
            original_run_capture = package_analyzer.run_capture
            original_extract_payload_cpio = package_analyzer.extract_payload_cpio

            def fake_run_capture(cmd: list[str], *, check: bool = True) -> subprocess.CompletedProcess[str]:
                out_dir = Path(cmd[cmd.index("-C") + 1])
                source = Path(cmd[2])
                if source == package:
                    (out_dir / "Distribution").write_text("<installer-gui-script/>", encoding="utf-8")
                    (out_dir / "buckyos.pkg").write_bytes(b"flat component pkg")
                else:
                    (out_dir / "PackageInfo").write_text(
                        (
                            '<pkg-info identifier="org.buckyos" version="0.6.0" '
                            'install-location="/opt/buckyos" auth="root"/>'
                        ),
                        encoding="utf-8",
                    )
                    scripts_dir = out_dir / "Scripts"
                    scripts_dir.mkdir()
                    (scripts_dir / "postinstall").write_text("#!/bin/sh\necho installed\n", encoding="utf-8")
                    (out_dir / "Payload").write_bytes(b"cpio payload")
                return subprocess.CompletedProcess(cmd, 0, "", "")

            def fake_extract_payload_cpio(
                payload_path: Path,
                out_dir: Path,
                *,
                tools: dict[str, package_analyzer.ToolStatus],
            ) -> tuple[int, str]:
                (out_dir / "opt" / "buckyos").mkdir(parents=True)
                (out_dir / "opt" / "buckyos" / "ood-daemon").write_bytes(b"binary")
                return 0, ""

            try:
                package_analyzer.run_capture = fake_run_capture
                package_analyzer.extract_payload_cpio = fake_extract_payload_cpio
                record = package_analyzer.base_package_record(package, include_hashes=False)
                package_analyzer.analyze_pkg_with_xar(
                    package,
                    record,
                    tools={"cpio": package_analyzer.ToolStatus(name="cpio", path="/usr/bin/cpio")},
                    include_hashes=False,
                    script_text_limit=1024,
                )
            finally:
                package_analyzer.run_capture = original_run_capture
                package_analyzer.extract_payload_cpio = original_extract_payload_cpio

        components = record["analysis"]["components"]
        self.assertEqual(components[0]["name"], "buckyos.pkg")
        self.assertEqual(components[0]["package_info"]["identifier"], "org.buckyos")
        package_files = {entry["path"] for entry in components[0]["package_files"]}
        self.assertIn("PackageInfo", package_files)
        self.assertIn("Payload", package_files)
        payload_files = {entry["path"] for entry in components[0]["payload"]["files"]}
        self.assertIn("opt/buckyos/ood-daemon", payload_files)
        installer_scripts = record["analysis"]["installer_scripts"]
        self.assertTrue(any(script["name"] == "postinstall" for script in installer_scripts))

    @unittest.skipIf(shutil.which("cpio") is None, "cpio is required")
    def test_pkg_xar_extracts_gzip_scripts_archive(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            package = root / "buckyos-apple-aarch64-0.6.0+build260603.pkg"
            package.write_bytes(b"outer pkg")
            script_src = root / "script-src"
            script_src.mkdir()
            (script_src / "postinstall").write_text("#!/bin/zsh\necho installed\n", encoding="utf-8")
            archive = subprocess.run(
                ["cpio", "-o", "--format=newc"],
                cwd=script_src,
                input=b"postinstall\n",
                capture_output=True,
                check=True,
            )
            scripts_archive = gzip.compress(archive.stdout)
            original_run_capture = package_analyzer.run_capture

            def fake_run_capture(cmd: list[str], *, check: bool = True) -> subprocess.CompletedProcess[str]:
                out_dir = Path(cmd[cmd.index("-C") + 1])
                comp_dir = out_dir / "buckycli.pkg"
                comp_dir.mkdir()
                (comp_dir / "PackageInfo").write_text(
                    '<pkg-info identifier="ai.buckyos.buckycli" version="0.6.0"/>',
                    encoding="utf-8",
                )
                (comp_dir / "Scripts").write_bytes(scripts_archive)
                return subprocess.CompletedProcess(cmd, 0, "", "")

            try:
                package_analyzer.run_capture = fake_run_capture
                record = package_analyzer.base_package_record(package, include_hashes=False)
                package_analyzer.analyze_pkg_with_xar(
                    package,
                    record,
                    tools={
                        "cpio": package_analyzer.ToolStatus(name="cpio", path=shutil.which("cpio")),
                        "gzip": package_analyzer.ToolStatus(name="gzip", path=shutil.which("gzip")),
                    },
                    include_hashes=False,
                    script_text_limit=1024,
                )
            finally:
                package_analyzer.run_capture = original_run_capture

        comp = record["analysis"]["components"][0]
        self.assertEqual(comp["scripts_compression"], "gzip")
        self.assertIn("postinstall", comp["scripts"])
        self.assertIn("echo installed", comp["scripts"]["postinstall"]["text"])
        installer_scripts = record["analysis"]["installer_scripts"]
        self.assertTrue(any(script["name"] == "postinstall" for script in installer_scripts))

    def test_separate_index_uses_relative_analysis_json_name(self) -> None:
        report = {
            "schema_version": 1,
            "package_count": 1,
            "packages": [
                {
                    "name": "buckyos-linux-amd64-0.6.0.deb",
                    "format": "deb",
                    "size_bytes": 7,
                    "sha256": "abc",
                    "inferred": {"platform": "linux"},
                    "analysis": {},
                }
            ],
        }
        with tempfile.TemporaryDirectory() as td:
            with contextlib.redirect_stdout(io.StringIO()):
                package_analyzer.write_separate_reports(report, Path(td), pretty=False)
            index = json.loads((Path(td) / "package_analysis_index.json").read_text(encoding="utf-8"))

        item = index["packages"][0]
        self.assertEqual(item["analysis_json"], "buckyos-linux-amd64-0.6.0.deb.analysis.json")
        self.assertNotIn("path", item)


if __name__ == "__main__":
    unittest.main()
