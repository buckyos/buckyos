import json
from pathlib import Path
import unittest
from unittest import mock

import ext_tools


class ScriptEncodingTest(unittest.TestCase):
    def test_related_scripts_are_ascii(self):
        paths = [ext_tools.ROOT / name for name in (
            "build_exttool",
            "src/tools/prepare_ext_tools.py",
            ".github/scripts/ext_tools.py",
            ".github/scripts/test_ext_tools.py",
            ".github/workflows/ext-core.yml",
            ".github/workflows/build-exttool.yml",
            "publish/exttool/ext-core/conda-env.yml",
        )]
        for directory in (ext_tools.EXT_CORE, *(ext_tools.EXT_CORE / name for name in (
            "smoke", "windows", "browser", "browser/test",
        ))):
            paths.extend(path for path in directory.iterdir()
                         if path.is_file() and path.suffix in (".py", ".sh", ".ps1", ".mjs", ".cmd", ".bat"))
        for path in paths:
            with self.subTest(path=str(path.relative_to(ext_tools.ROOT))):
                self.assertTrue(path.read_bytes().isascii(),
                                "Use ASCII script source and escapes for Unicode test data")


def manifest(*commands):
    return {"provides": [{"command": c} for c in commands]}


TABLE = """intro
**\u8868 B\uff1aext-core**

| \u547d\u4ee4 | \u5de5\u5177 | \u573a\u666f |
| --- | --- | --- |
| `ffmpeg`\u3001`ffprobe` | ffmpeg | x |
| `tesseract`\uff08\u542b `eng`\u3001`osd` \u6570\u636e\uff09 | tesseract | y |

**\u8868 C\uff1a\u5176\u5b83**
| `soffice` | z | z |
"""


class TableTest(unittest.TestCase):
    def test_the_real_files_agree(self):
        errors = ext_tools.check_table(
            ext_tools.TABLE_DOC.read_text(encoding="utf-8"),
            json.loads((ext_tools.EXT_CORE / "ext-manifest.json").read_text()),
            (ext_tools.EXT_CORE / "conda-env.yml").read_text(),
            (ext_tools.EXT_CORE / "sources.sha256").read_text(),
        )
        self.assertEqual(errors, [])

    def test_the_table_reads_the_command_column_of_table_b_only(self):
        self.assertEqual(ext_tools.table_commands(TABLE), {"ffmpeg", "ffprobe", "tesseract"})

    def test_a_difference_in_either_direction_is_reported(self):
        env = "\n".join(f"  - {tool}" for tool in ext_tools.CONDA_TOOLS)
        sources = "\n".join(f"abc  {prefix}1.0" for prefix in ext_tools.OFFICIAL_TOOLS.values())
        table = TABLE
        self.assertEqual(ext_tools.check_table(table, manifest("ffmpeg", "ffprobe", "tesseract"), env, sources), [])
        errors = ext_tools.check_table(table, manifest("ffmpeg", "tesseract", "magick"), env, sources)
        self.assertEqual(errors, ["table B lists `ffprobe`, ext-manifest.json does not provide it", "ext-manifest.json provides `magick`, table B does not list it"])
        errors = ext_tools.check_table(table, manifest("ffmpeg", "ffprobe", "tesseract"), "  - curl", sources)
        self.assertIn("conda-env.yml does not ask for ffmpeg", errors)
        errors = ext_tools.check_table(table, manifest("ffmpeg", "ffprobe", "tesseract"), env, "")
        self.assertIn("sources.sha256 pins nothing for pandoc", errors)


def online(version="1.2.3.4", into="chromium-77", sha="a" * 64, platforms=ext_tools.PLATFORMS):
    return {"provides": [], "online": [{"id": "chromium", "version": version, "platforms": {
        p: {"url": f"https://upstream.invalid/{version}/{p}.zip", "sha256": sha, "into": into} for p in platforms
    }}]}


class OnlineTest(unittest.TestCase):
    BUILD = "CHROMIUM_VERSION=1.2.3.4\nCHROMIUM_REVISION=77\n"
    SOURCES = "a" * 64 + "  chrome-for-testing-1.2.3.4-linux64.zip\n"

    def test_the_real_files_agree(self):
        errors = ext_tools.check_online(
            json.loads((ext_tools.EXT_CORE / "ext-manifest.json").read_text()),
            (ext_tools.EXT_CORE / "build.sh").read_text(encoding="utf-8"),
            (ext_tools.EXT_CORE / "sources.sha256").read_text(),
        )
        self.assertEqual(errors, [])

    def test_the_manifest_declares_what_the_build_checked(self):
        self.assertEqual(ext_tools.check_online(online(), self.BUILD, self.SOURCES), [])
        self.assertEqual(ext_tools.check_online({"provides": []}, self.BUILD, self.SOURCES), ["ext-manifest.json declares no online item `chromium`"])
        for manifest, message in (
            (online(version="1.2.3.5"), "online chromium is 1.2.3.5, build.sh checks 1.2.3.4"),
            (online(into="chromium-78"), "online chromium on linux-x86_64: not unpacked into chromium-77, where Playwright looks"),
            (online(sha="b" * 64), "online chromium on linux-x86_64: sources.sha256 does not pin the archive"),
            (online(platforms=ext_tools.PLATFORMS[:4]), "online chromium has nothing for windows-x86_64"),
        ):
            self.assertIn(message, ext_tools.check_online(manifest, self.BUILD, self.SOURCES))
        moved = online()
        moved["online"][0]["platforms"]["darwin-arm64"]["url"] = "https://upstream.invalid/1.2.3.3/darwin-arm64.zip"
        self.assertEqual(ext_tools.check_online(moved, self.BUILD, self.SOURCES), ["online chromium on darwin-arm64: the address is not of 1.2.3.4"])


def release(version="1.1.0", platforms=ext_tools.PLATFORMS):
    return {"schema": ext_tools.SCHEMA, "packages": {"ext-core": {"version": version, "pikg_digest": "sha256:p", "platforms": {
        p: {
            "file": f"{p}.tar.gz", "sha256": "sha256:" + "0" * 64, "size": 10,
            "path": f"ext-tools/ext-core/{version}/{p}/0.tar.gz",
            "corresponding_source": {
                "file": f"{p}-corresponding-source.tar.gz", "sha256": "sha256:" + "1" * 64,
                "size": 20, "path": f"ext-tools/ext-core/{version}/{p}/1.source.tar.gz",
                "manifest_sha256": "sha256:" + "2" * 64,
            },
        } for p in platforms
    }}}}


class ResolveTest(unittest.TestCase):
    def test_a_release_gets_urls_and_every_tar_is_checked(self):
        heads = []

        def head(url):
            heads.append(url)
            return 20 if url.endswith(".source.tar.gz") else 10

        resolved = ext_tools.resolve("1.1.0", "https://r2", fetch=lambda url: release(), head=head)
        entry = resolved["packages"]["ext-core"]["platforms"]["darwin-arm64"]
        self.assertEqual(entry["url"], "https://r2/ext-tools/ext-core/1.1.0/darwin-arm64/0.tar.gz")
        self.assertEqual(entry["corresponding_source"]["url"], "https://r2/ext-tools/ext-core/1.1.0/darwin-arm64/1.source.tar.gz")
        self.assertEqual(len(heads), len(ext_tools.PLATFORMS) * 2)

    def test_a_release_that_is_not_all_there_fails_the_build(self):
        with self.assertRaisesRegex(ValueError, "no payload for windows-x86_64"):
            ext_tools.resolve("1.1.0", "https://r2", fetch=lambda url: release(platforms=ext_tools.PLATFORMS[:4]), head=lambda url: 10)
        with self.assertRaisesRegex(ValueError, "the release says"):
            ext_tools.resolve("1.1.0", "https://r2", fetch=lambda url: release(), head=lambda url: 9)
        with self.assertRaisesRegex(ValueError, "the release says 1.0.0"):
            ext_tools.resolve("1.1.0", "https://r2", fetch=lambda url: release(version="1.0.0"), head=lambda url: 10)

    def test_sources_are_required_and_checked_before_accepting_a_release(self):
        for field, value, message in (
            ("corresponding_source", None, "no corresponding source archive"),
            ("manifest_sha256", "sha256:bad", "no valid source manifest checksum"),
            ("sha256", "", "invalid artifact checksum"),
            ("path", "../source.tar.gz", "invalid artifact path"),
            ("size", 0, "invalid artifact size"),
        ):
            with self.subTest(field=field):
                data = release()
                entry = data["packages"]["ext-core"]["platforms"][ext_tools.PLATFORMS[0]]
                if field == "corresponding_source":
                    entry[field] = value
                else:
                    entry["corresponding_source"][field] = value
                with self.assertRaisesRegex(ValueError, message):
                    ext_tools.resolve("1.1.0", "https://r2", fetch=lambda url: data, head=lambda url: 10)
        with self.assertRaisesRegex(ValueError, "10 bytes, the release says 20"):
            ext_tools.resolve("1.1.0", "https://r2", fetch=lambda url: release(), head=lambda url: 10)


class RequestTest(unittest.TestCase):
    def test_no_request_carries_the_default_user_agent(self):
        seen = []

        def urlopen(request, timeout):
            seen.append(request)
            raise OSError("stop")

        with mock.patch("urllib.request.urlopen", urlopen):
            for call in (ext_tools.fetch_json, ext_tools.head_size):
                with self.assertRaises(OSError):
                    call("https://r2/x")
        self.assertEqual([r.get_header("User-agent") for r in seen], ["buckyos-ext-tools/1"] * 2)


if __name__ == "__main__":
    unittest.main()
