from __future__ import annotations

import unittest
from pathlib import Path

import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import make_local_osx_pkg


class MakeLocalOsxPkgTests(unittest.TestCase):
    def test_buckycli_pkg_target_uses_publish_config(self) -> None:
        component = make_local_osx_pkg.PublishComponent(
            key="buckycli",
            name="BuckyOS CLI",
            kind="app",
            optional=True,
            default_selected=True,
            src=None,
            default_target="/usr/local/bin/",
        )

        self.assertEqual(make_local_osx_pkg._resolve_component_pkg_target(component), Path("/usr/local/bin"))


if __name__ == "__main__":
    unittest.main()
