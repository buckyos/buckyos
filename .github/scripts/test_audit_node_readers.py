import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("audit", Path(__file__).with_name("audit-node-readers.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

READERS = """
[review]
commit = "{commit}"
date = 2026-11-23
changed = 2026-11-20
conclusion = "nothing to do"

[audited_sources]
dep = "{dep}"
absent = "{dep}"

[[reader]]
id = "config"
owner = "kernel"
rust = "config::kernel"
schema = "1"
audit = ["kernel/config.rs"]

[[reader]]
id = "store"
owner = "dep"
rust = "sql::store"
schema = "2"
audit = ["dep:src/store.rs"]
"""


class AuditTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name) / "buckyos"
        self.dep = Path(temp.name) / "dep"
        for checkout in (self.root, self.dep):
            checkout.mkdir()
            module.git(checkout, "init", "-q")
        dep = self.commit(self.dep, "src/store.rs", "1")
        reviewed = self.commit(self.root, "src/kernel/config.rs", "1")
        self.commit(self.root, module.REGISTRY, READERS.format(commit=reviewed, dep=dep))

    def commit(self, checkout, name, text):
        path = checkout / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
        module.git(checkout, "add", "-A")
        module.git(checkout, "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "c")
        return module.git(checkout, "rev-parse", "HEAD").strip()

    def lines(self, **kwargs):
        return list(module.audit(self.root, **kwargs))

    def test_the_recorded_review_is_the_default_base_of_every_repository(self):
        lines = self.lines()
        self.assertEqual(lines[0], "Last review 2026-11-23, readers last changed 2026-11-20: nothing to do")
        self.assertTrue(lines[1].startswith("buckyos: 1 commits since "))
        self.assertTrue(any(line.startswith("dep: 0 commits since ") for line in lines))
        self.assertTrue(lines[-1].startswith("absent: not checked"))
        self.assertNotIn("    src/kernel/config.rs", lines)
        self.assertNotIn("    src/store.rs", lines)

        self.commit(self.root, "src/kernel/config.rs", "2")
        self.commit(self.dep, "src/store.rs", "2")
        lines = self.lines()
        config = lines.index("Desktop schema review — config (kernel), target 1")
        self.assertIn("    src/kernel/config.rs", lines[config:config + 3])
        store = lines.index("dep: 1 commits since " + module.git(self.dep, "rev-parse", "HEAD^")[:12])
        self.assertEqual(lines[store + 1:store + 3],
                         ["Desktop schema review — store (dep), target 2", "    src/store.rs"])

    def test_a_given_base_or_dependency_is_reported_alone(self):
        self.commit(self.root, "README.md", "unregistered")
        head = module.git(self.root, "rev-parse", "HEAD^").strip()
        self.assertEqual(self.lines(base=head), [f"buckyos: 1 commits since {head[:12]}"])
        self.commit(self.dep, "src/store.rs", "2")
        self.assertEqual(self.lines(dependency=self.dep)[1:],
                         ["Desktop schema review — store (dep), target 2", "    src/store.rs"])
        with self.assertRaisesRegex(ValueError, "unregistered format source"):
            self.lines(dependency=self.root)
        with self.assertRaises(subprocess.CalledProcessError):
            self.lines(base="0" * 40)


if __name__ == "__main__":
    unittest.main()
