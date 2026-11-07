import json
from pathlib import Path
import unittest
from unittest import mock

import ext_tools


def manifest(*commands):
    return {"provides": [{"command": c} for c in commands]}


TABLE = """intro
**表 B：ext-core**

| 命令 | 工具 | 场景 |
| --- | --- | --- |
| `ffmpeg`、`ffprobe` | ffmpeg | x |
| `tesseract`（含 `eng`、`osd` 数据） | tesseract | y |

**表 C：其它**
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


def release(version="1.1.0", platforms=ext_tools.PLATFORMS):
    return {"schema": ext_tools.SCHEMA, "packages": {"ext-core": {"version": version, "pikg_digest": "sha256:p", "platforms": {
        p: {"file": f"{p}.tar.gz", "sha256": "sha256:0", "size": 10, "path": f"ext-tools/ext-core/{version}/{p}/0.tar.gz"} for p in platforms
    }}}}


class ResolveTest(unittest.TestCase):
    def test_a_release_gets_urls_and_every_tar_is_checked(self):
        heads = []

        def head(url):
            heads.append(url)
            return 10

        resolved = ext_tools.resolve("1.1.0", "https://r2", fetch=lambda url: release(), head=head)
        entry = resolved["packages"]["ext-core"]["platforms"]["darwin-arm64"]
        self.assertEqual(entry["url"], "https://r2/ext-tools/ext-core/1.1.0/darwin-arm64/0.tar.gz")
        self.assertEqual(len(heads), len(ext_tools.PLATFORMS))

    def test_a_release_that_is_not_all_there_fails_the_build(self):
        with self.assertRaisesRegex(ValueError, "no payload for windows-x86_64"):
            ext_tools.resolve("1.1.0", "https://r2", fetch=lambda url: release(platforms=ext_tools.PLATFORMS[:4]), head=lambda url: 10)
        with self.assertRaisesRegex(ValueError, "the release says"):
            ext_tools.resolve("1.1.0", "https://r2", fetch=lambda url: release(), head=lambda url: 9)
        with self.assertRaisesRegex(ValueError, "the release says 1.0.0"):
            ext_tools.resolve("1.1.0", "https://r2", fetch=lambda url: release(version="1.0.0"), head=lambda url: 10)


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
