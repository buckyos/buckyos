import hashlib
import json
import os
from pathlib import Path
import re
import sys
import tarfile
import tempfile
import unittest
import xml.etree.ElementTree as ElementTree

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
sys.path.insert(0, str(HERE))
import build

PLATFORMS = ["linux-x86_64", "linux-aarch64", "darwin-arm64", "darwin-x86_64", "windows-x86_64"]
FONTS_TS = ROOT / "src" / "frame" / "aiworkspace" / "print" / "fonts.ts"
EXT_CORE_BUILD = ROOT / "publish" / "exttool" / "ext-core" / "build.sh"
SETTINGS_I18N = ROOT / "src" / "frame" / "desktop" / "src" / "i18n" / "settings.ts"


def pack_fonts():
    """item -> family of PACK_FONTS in the print module."""
    text = FONTS_TS.read_text(encoding="utf-8")
    block = text[text.index("export const PACK_FONTS"):]
    block = block[: block.index("]\n")]
    return {item: family for family, item in re.findall(r"family: '([^']+)',[^}]*item: '([a-z0-9-]+)'", block)}


class ManifestTests(unittest.TestCase):
    def setUp(self):
        self.manifest = build.manifest()

    def test_a_package_of_online_items_that_are_fonts(self):
        m = self.manifest
        self.assertEqual(m["schema"], "buckyos.ext_tools/1")
        self.assertEqual(m["id"], "ext-fonts")
        self.assertEqual(m["provides"], [], "no command: what it gives is fonts")
        self.assertEqual(m["platforms"], PLATFORMS)
        self.assertTrue(m["online"])
        app = json.loads((HERE / "dapp_meta" / "app.json").read_text())
        pikg = json.loads((HERE / "dapp_meta" / "pikg.json").read_text())
        self.assertEqual(app["version"], m["version"])
        self.assertEqual(pikg["pikg_file"], f"ext-fonts-{m['version']}.pikg")

    def test_every_font_comes_from_its_upstream_release_on_github_the_same_on_every_platform(self):
        for item in self.manifest["online"]:
            with self.subTest(item=item["id"]):
                self.assertFalse(item["auto"], "an administrator asks for a font")
                self.assertEqual(sorted(item["platforms"]), sorted(PLATFORMS))
                sources = list(item["platforms"].values())
                self.assertTrue(all(source == sources[0] for source in sources))
                source = sources[0]
                self.assertRegex(source["url"], r"^https://github\.com/[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+/releases/download/[^/]+/[^/]+\.zip$")
                self.assertEqual(source["format"], "zip")
                self.assertRegex(source["sha256"], r"^[0-9a-f]{64}$")
                self.assertGreater(source["size_bytes"], 0)
                self.assertEqual(set(source), {"url", "sha256", "size_bytes", "format"})
                self.assertIn("SIL Open Font License 1.1", item["license"])
                self.assertIn("embedding", item["license"])
                self.assertIn("neither carries nor redistributes", item["license"])

    def test_the_print_module_and_the_settings_know_the_same_fonts(self):
        items = {item["id"]: item for item in self.manifest["online"]}
        families = pack_fonts()
        self.assertEqual(sorted(families), sorted(items))
        for item, family in families.items():
            self.assertTrue(items[item]["desc"].startswith(f"{family}:"), f"{item}: desc names {family}")
        i18n = SETTINGS_I18N.read_text(encoding="utf-8")
        for item in items:
            self.assertEqual(i18n.count(f"'settings.extTools.online.name.{item}'"), 2, f"{item}: English and Chinese names")


class FontsConfTests(unittest.TestCase):
    def test_fonts_conf_names_one_directory_in_every_item_and_nothing_else(self):
        text = build.FONTS_CONF.read_text(encoding="utf-8")
        root = ElementTree.fromstring(text)
        self.assertEqual(root.tag, "fontconfig")
        self.assertEqual({child.tag for child in root}, {"dir"})
        dirs = build.font_dirs(text)
        self.assertEqual(len(dirs), len(list(root)))
        self.assertEqual(sorted(dirs), sorted(item["id"] for item in build.manifest()["online"]))
        for path in dirs.values():
            self.assertFalse(path.startswith("/") or ".." in path.split("/") or path.endswith("/"), path)

    def test_relative_paths_reach_the_items_from_where_the_files_really_are(self):
        """fontconfig resolves a relative <dir> from the real path of the file that has it, and the kernel resolves the
        `..` of an <include> under FONTCONFIG_PATH from where that directory really is: a symlink in the way does not
        change the count. These are the layouts of buckyos-api's ext_tools_install.rs."""
        with tempfile.TemporaryDirectory() as temporary:
            tools = Path(temporary) / "tools"
            fonts_store = tools / "versions/ext-fonts/1.0.0-abc/store/ext-fonts"
            fonts_store.mkdir(parents=True)
            (tools / "store").mkdir()
            os.symlink("../versions/ext-fonts/1.0.0-abc/store/ext-fonts", tools / "store/ext-fonts")
            (fonts_store / "fonts.conf").write_text("")
            for item, path in build.font_dirs(build.FONTS_CONF.read_text(encoding="utf-8")).items():
                (tools / "online" / item / path).mkdir(parents=True)
                found = os.path.realpath(os.path.join(os.path.realpath(tools / "store/ext-fonts"), f"../../../../../online/{item}/{path}"))
                self.assertEqual(found, os.path.realpath(tools / "online" / item / path))
            includes = re.findall(r'<include ignore_missing="yes">([^<]+)</include>', EXT_CORE_BUILD.read_text(encoding="utf-8"))
            self.assertEqual(len(includes), 3)
            layouts = {
                "version directory": tools / "versions/ext-core/1.3.3-abc/store/ext-core/conda/etc/fonts",
                "staging": tools / "staging/ext-core-abc-1/store/ext-core/conda/etc/fonts",
            }
            for name, fonts in layouts.items():
                fonts.mkdir(parents=True)
                reached = [include for include in includes if os.path.isfile(os.path.join(fonts, include))]
                self.assertEqual(len(reached), 1, name)
                self.assertEqual(os.path.realpath(os.path.join(fonts, reached[0])), str((fonts_store / "fonts.conf").resolve()))
            unpacked = Path(temporary) / "unpacked"
            fonts = unpacked / "store/ext-core/conda/etc/fonts"
            fonts.mkdir(parents=True)
            (unpacked / "store/ext-fonts").mkdir(parents=True)
            (unpacked / "store/ext-fonts/fonts.conf").write_text("")
            self.assertEqual([include for include in includes if os.path.isfile(os.path.join(fonts, include))], ["../../../../../store/ext-fonts/fonts.conf"])


class BuildTests(unittest.TestCase):
    def test_every_platform_has_the_same_payload_and_a_build_the_same_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            out, dist = Path(temporary) / "payload", Path(temporary) / "dist"
            build.main(["--archive", "--out", str(out), "--dist", str(dist)])
            version = build.manifest()["version"]
            for platform in PLATFORMS:
                files = sorted(p.relative_to(out / platform).as_posix() for p in (out / platform).rglob("*") if p.is_file())
                self.assertEqual(files, ["ext-manifest.json", "store/ext-fonts/NOTICE.md", "store/ext-fonts/fonts.conf"])
            first = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in dist.glob("*.tar.gz")}
            self.assertEqual(len(first), len(PLATFORMS))
            self.assertEqual(len(set(first.values())), len(PLATFORMS), "a PIKG takes no two sub-packages with one digest")
            for platform in PLATFORMS:
                self.assertEqual((out / platform / "ext-manifest.json").read_bytes(), build.MANIFEST.read_bytes())
                self.assertEqual((out / platform / "store/ext-fonts/fonts.conf").read_bytes(), build.FONTS_CONF.read_bytes())
            build.main(["--archive", "--out", str(out), "--dist", str(dist)])
            self.assertEqual(first, {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in dist.glob("*.tar.gz")})
            with tarfile.open(dist / build.tar_name(version, "linux-x86_64")) as tar:
                names = tar.getnames()
                self.assertIn("ext-manifest.json", names)
                self.assertTrue(all(not name.startswith("/") and ".." not in name.split("/") for name in names))
                self.assertTrue(all(member.uid == 0 and member.mtime == 0 for member in tar.getmembers()))
            notice = (out / "linux-x86_64/store/ext-fonts/NOTICE.md").read_text()
            self.assertIn("for linux-x86_64", notice.splitlines()[0])
            for item in build.manifest()["online"]:
                self.assertIn(next(iter(item["platforms"].values()))["url"], notice)

            pikg = Path(temporary) / f"ext-fonts-{version}.pikg"
            pikg.write_bytes(b"pikg")
            build.main(["release", "--dist", str(dist), "--pikg", str(pikg)])
            release = json.loads((dist / "manifest.json").read_text())
            package = release["packages"]["ext-fonts"]
            self.assertEqual(release["schema"], "buckyos.ext-tools/v1")
            self.assertEqual(package["version"], version)
            self.assertEqual(package["manifest"], build.manifest())
            entry = package["platforms"]["linux-x86_64"]
            self.assertEqual(entry["path"], f"ext-tools/ext-fonts/{version}/linux-x86_64/{entry['sha256'].removeprefix('sha256:')}.tar.gz")
            self.assertEqual(package["pikg"]["path"], f"ext-tools/ext-fonts/{version}/{pikg.name}")


if __name__ == "__main__":
    unittest.main()
