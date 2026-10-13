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


    def test_all_desktop_platforms_use_the_same_schema(self):
        for platform in ('amd64-linux', 'arm64-linux', 'amd64-macos', 'arm64-macos', 'amd64-windows', 'arm64-windows'):
            for component in ('buckyos', 'cyfs-gateway'):
                with self.subTest(platform=platform, component=component), tempfile.TemporaryDirectory() as tmp:
                    root = Path(tmp)
                    (root / 'bin/service').mkdir(parents=True)
                    (root / 'data').mkdir()
                    project = {'apps': {component: {'modules': {'service': 'bin/service/'}, 'data_paths': ['data/']}}}
                    descriptor = module.create(project, component, '0.7.0+build261013', platform, root)
                    self.assertEqual(set(descriptor), {'schema', 'component', 'version', 'platform', 'modules', 'data_paths', 'clean_paths'})
                    self.assertEqual(descriptor['platform'], platform)
                    self.assertEqual(descriptor['modules'], {'service': 'bin/service/'})

    def test_windows_validates_suffixes_without_rewriting_project_paths(self):
        for suffix in ('.exe', '.ps1', '.cmd'):
            with self.subTest(suffix=suffix), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                (root / 'bin').mkdir()
                (root / ('bin/launcher' + suffix)).write_bytes(b'launcher')
                project = {'apps': {'buckyos': {'modules': {'launcher': 'bin/launcher'}, 'data_paths': []}}}
                descriptor = module.create(project, 'buckyos', '0.7.0+build261013', 'amd64-windows', root)
                self.assertEqual(descriptor['modules'], {'launcher': 'bin/launcher'})
                self.assertEqual(project['apps']['buckyos']['modules']['launcher'], 'bin/launcher')
                with self.assertRaisesRegex(ValueError, 'missing'):
                    module.create(project, 'buckyos', '0.7.0+build261013', 'amd64-linux', root)

    def test_staged_paths_cannot_escape_or_claim_a_file_is_a_directory(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / 'payload'
            root.mkdir()
            (root.parent / 'outside').write_bytes(b'outside')
            (root / 'escape').symlink_to(root.parent / 'outside')
            (root / 'file').write_bytes(b'file')
            for path in ('escape', 'file/'):
                with self.subTest(path=path), self.assertRaises(ValueError):
                    module.create({'apps': {'buckyos': {'modules': {'x': path}}}},
                                  'buckyos', '0.7.0+build261013', 'arm64-macos', root)


    def test_project_lists_are_preserved_without_sorting_or_deduplication(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'bin').mkdir()
            (root / 'data').mkdir()
            app = {'modules': {'second': 'bin/', 'first': 'data/'},
                   'data_paths': ['data/', 'data/'], 'clean_paths': ['cache/', 'cache/', 'local/']}
            descriptor = module.create({'apps': {'buckyos': app}}, 'buckyos', '0.7.0+build261013', 'amd64-linux', root)
            for field in ('modules', 'data_paths', 'clean_paths'):
                self.assertEqual(descriptor[field], app[field])
            self.assertEqual(list(descriptor['modules']), ['second', 'first'])
            for unsafe in ('../outside', '/etc', '.install-buckyos.json'):
                app['clean_paths'] = [unsafe]
                with self.subTest(unsafe=unsafe), self.assertRaises(ValueError):
                    module.create({'apps': {'buckyos': app}}, 'buckyos', '0.7.0+build261013', 'amd64-linux', root)


if __name__=='__main__':
    unittest.main()
