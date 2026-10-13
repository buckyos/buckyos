import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

spec=importlib.util.spec_from_file_location('descriptor',Path(__file__).with_name('write-install-descriptor.py'))
module=importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class DescriptorTests(unittest.TestCase):
    def test_adds_one_file_without_moving_existing_payload(self):
        for component in ('buckyos','cyfs-gateway'):
            with self.subTest(component=component),tempfile.TemporaryDirectory() as tmp:
                root=Path(tmp)
                (root/'bin/service').mkdir(parents=True)
                (root/'bin/service/service').write_bytes(b'binary')
                (root/'data').mkdir()
                project=root.parent/(root.name+'.yaml')
                project.write_text('apps:\n  '+component+':\n    modules:\n      service: bin/service/\n    data_paths:\n      - data/\n')
                before={p.relative_to(root).as_posix():p.read_bytes() for p in root.rglob('*') if p.is_file()}
                try:
                    subprocess.run([sys.executable,str(Path(__file__).with_name('write-install-descriptor.py')),
                        '--project',str(project),'--component',component,'--root',str(root),
                        '--version','0.7.0+build261013','--platform','arm64-linux'],check=True)
                finally:
                    project.unlink()
                for name,data in before.items(): self.assertEqual((root/name).read_bytes(),data)
                descriptor=json.loads((root/('.install-'+component+'.json')).read_text())
                self.assertEqual(descriptor['component'],component)
                self.assertEqual(descriptor['modules'],{'service':'bin/service/'})
                self.assertEqual(descriptor['platform'],'arm64-linux')
                self.assertEqual({p.relative_to(root).as_posix() for p in root.rglob('*') if p.is_file()}-set(before),{'.install-'+component+'.json'})

    def test_unsafe_and_missing_paths_are_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp)
            for path in ('../bin','/bin','data/$(id)','.install-other.json','missing/'):
                project={'apps':{'buckyos':{'modules':{'x':path},'data_paths':[]}}}
                with self.subTest(path=path),self.assertRaises(ValueError):
                    module.create(project,'buckyos','0.7.0+build261013','amd64-linux',root)


if __name__=='__main__':
    unittest.main()
