import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import unittest


SRC = Path(__file__).parents[2]


@unittest.skipUnless(os.name == "nt", "Windows console creation")
class WindowsProcessTests(unittest.TestCase):
    def test_background_launchers_have_no_console_and_preserve_output(self):
        for relative in (
            "start.py",
            "rootfs/bin/util.py",
            "rootfs/bin/buckyos_filebrowser-bin/util.py",
        ):
            spec = importlib.util.spec_from_file_location("windows_launcher", SRC / relative)
            module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(module)
            for detached in (False, True):
                with self.subTest(launcher=relative, detached=detached):
                    kwargs = module._windows_subprocess_kwargs(detached=detached)
                    flags = kwargs["creationflags"]
                    self.assertTrue(flags & subprocess.CREATE_NO_WINDOW)
                    self.assertFalse(flags & (subprocess.DETACHED_PROCESS | subprocess.CREATE_NEW_CONSOLE))
                    if detached:
                        self.assertTrue(flags & subprocess.CREATE_NEW_PROCESS_GROUP)
                    result = subprocess.run(
                        [sys.executable, "-c", "import ctypes; print(ctypes.windll.kernel32.GetConsoleWindow())"],
                        capture_output=True,
                        text=True,
                        check=True,
                        **kwargs,
                    )
                    self.assertEqual(result.stdout.strip(), "0")
                    self.assertEqual(result.stderr, "")


if __name__ == "__main__":
    unittest.main()
