import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
import unittest


BUILD_SCRIPT = Path(__file__).resolve().parents[2] / "publish/exttool/ext-core/build.sh"
BODY = b"ext-core download fixture\n" * 100


class DownloadTest(unittest.TestCase):
    def setUp(self):
        git_bash = Path(os.environ.get("ProgramFiles", "C:/Program Files")) / "Git/bin/bash.exe"
        self.bash = str(git_bash) if os.name == "nt" and git_bash.is_file() else shutil.which("bash")
        if not self.bash:
            self.skipTest("bash is required")
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.cache = self.root / "cache"
        self.cache.mkdir()
        (self.root / "sources.sha256").write_text(
            f"{hashlib.sha256(BODY).hexdigest()}  archive.bin\n", encoding="utf-8"
        )
        self.requests = 0
        self.interruptions = 0
        self.body = BODY
        case = self

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                case.requests += 1
                self.send_response(200)
                self.send_header("Content-Length", str(len(case.body)))
                self.end_headers()
                interrupted = case.interruptions > 0
                case.interruptions -= int(interrupted)
                self.wfile.write(case.body[:10] if interrupted else case.body)
                self.wfile.flush()
                self.close_connection = True

            def log_message(self, *args):
                pass

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(
            target=self.server.serve_forever, kwargs={"poll_interval": 0.05}, daemon=True
        )
        self.thread.start()
        self.addCleanup(self.stop_server)

    def stop_server(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)

    def fetch(self):
        source = BUILD_SCRIPT.read_text(encoding="utf-8")
        functions = source[source.index("sha_of() {"):source.index("conda_platform() {")]
        script = (
            'set -euo pipefail\nhere="$PWD"\ncache="$here/cache"\n'
            + functions
            + '\nfile="$(fetch "$1")"\nprintf "%s\\n" "$file"\n'
        )
        env = dict(os.environ, NO_PROXY="127.0.0.1", no_proxy="127.0.0.1")
        return subprocess.run(
            [self.bash, "--noprofile", "--norc", "-s", "--",
             f"http://127.0.0.1:{self.server.server_port}/archive.bin"],
            input=script, text=True, capture_output=True, cwd=self.root, env=env, timeout=30,
        )

    def test_interrupted_download_is_retried_and_cached(self):
        self.interruptions = 1
        result = self.fetch()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.requests, 2)
        self.assertEqual((self.cache / "archive.bin").read_bytes(), BODY)
        self.assertFalse((self.cache / "archive.bin.part").exists())
        result = self.fetch()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.requests, 2, "a verified cache must not be downloaded again")

    def test_exhausted_retries_leave_no_cached_or_partial_file(self):
        self.interruptions = 100
        result = self.fetch()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.requests, 4)
        self.assertIn("curl: (18)", result.stderr)
        self.assertNotIn("sha256 differs", result.stderr)
        self.assertEqual(list(self.cache.iterdir()), [])

    def test_checksum_mismatch_does_not_enter_cache(self):
        self.body = b"complete but unexpected archive"
        result = self.fetch()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.requests, 1)
        self.assertIn("sha256 differs", result.stderr)
        self.assertEqual(list(self.cache.iterdir()), [])


if __name__ == "__main__":
    unittest.main()
