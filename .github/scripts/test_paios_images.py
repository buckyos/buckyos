import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).with_name('paios_images.py')
FIXTURE = Path(__file__).resolve().parents[2] / 'src/kernel/node_daemon/src/testdata/paios_images.v1.json'
spec = importlib.util.spec_from_file_location('paios_images', SCRIPT)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def fixture():
    return json.loads(FIXTURE.read_text())


class PaiosImagesTests(unittest.TestCase):
    def test_fixture_shared_with_rust_is_valid(self):
        manifest = fixture()
        module.validate(manifest, require_arches=True)
        self.assertEqual(manifest['images']['aios']['platforms']['amd64']['ref'], 'paios/aios:0.7.0-835b5bd44125-amd64')

    def test_invalid_manifests_are_rejected(self):
        edits = {
            'schema': lambda m: m.__setitem__('schema', 'buckyos.paios-images/v2'),
            'no aios': lambda m: m['images'].pop('aios'),
            'arch': lambda m: m['images']['aios']['platforms'].__setitem__('arm64', m['images']['aios']['platforms']['amd64']),
            'tar path': lambda m: m['images']['aios']['platforms']['amd64']['tar'].__setitem__('file', '../x.tar.gz'),
            'tar dir': lambda m: m['images']['aios']['platforms']['amd64']['tar'].__setitem__('file', 'a/x.tar.gz'),
            'digest': lambda m: m['images']['aios']['platforms']['amd64'].__setitem__('image_id', '7329ae'),
            'size': lambda m: m['images']['aios']['platforms']['amd64']['tar'].__setitem__('size', 0),
            'url': lambda m: m['images']['aios']['platforms']['amd64']['tar'].__setitem__('url', 'https://example.com/other.tar.gz'),
            'ref digest': lambda m: m['images']['aios']['platforms']['amd64'].__setitem__('ref', 'paios/aios@sha256:' + '0' * 64),
        }
        for name, edit in edits.items():
            with self.subTest(name):
                manifest = fixture()
                edit(manifest)
                with self.assertRaises(ValueError):
                    module.validate(manifest)

    def test_release_manifest_requires_both_arches(self):
        manifest = fixture()
        manifest['images']['exttool']['platforms'].pop('aarch64')
        module.validate(manifest)
        with self.assertRaises(ValueError):
            module.validate(manifest, require_arches=True)

    def test_platform_fragment_assemble_round_trip(self):
        source = fixture()
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            tar = tmp / 'aios-amd64-7329ae7f4108.tar.gz'
            tar.write_bytes(b'not really a docker archive')
            layers = ['sha256:' + str(index) * 64 for index in range(3)]
            (tmp / 'inspect.json').write_text(json.dumps([{'Id': source['images']['aios']['platforms']['amd64']['image_id'],
                                                            'RootFS': {'Type': 'layers', 'Layers': layers}}]))
            amd64 = module.platform_entry('aios', 'amd64', 'paios/aios:0.7.0-835b5bd44125-amd64', tmp / 'inspect.json',
                                          source['images']['aios']['platforms']['amd64']['manifest_digest'], tar,
                                          'https://buckyos-installer.tbudr.top/')
            self.assertEqual(amd64['tar']['size'], tar.stat().st_size)
            self.assertEqual(amd64['tar']['path'], 'paios-images/aios/amd64/' + module.sha256_file(tar) + '.tar.gz')
            self.assertEqual(amd64['tar']['url'], 'https://buckyos-installer.tbudr.top/' + amd64['tar']['path'])
            self.assertEqual(amd64['rootfs_layers_sha256'], module.layers_sha256(layers))
            (tmp / 'metadata.env').write_text('version=0.7.0\nrevision=835b5bd44125cd923444206782fe34ff9e0c3225\ncreated=2026-10-19T17:20:11Z\namd64_sha256=x\n')
            aios = module.fragment('aios', 'paios/aios', tmp / 'metadata.env',
                                   {'aarch64': source['images']['aios']['platforms']['aarch64'], 'amd64': amd64})
            self.assertEqual(list(aios['platforms']), ['aarch64', 'amd64'])
            inherited = copy.deepcopy(source['images']['exttool'])
            manifest = module.assemble({'exttool': inherited, 'aios': aios}, 'a' * 40, '2026-10-20T00:00:00Z')
            self.assertEqual(list(manifest['images']), ['aios', 'exttool'])
            self.assertEqual(manifest['images']['exttool'], source['images']['exttool'])
            with self.assertRaises(ValueError):
                module.assemble({'aios': aios}, 'a' * 40 + '-dirty')
            dirty = copy.deepcopy(aios)
            dirty['revision'] += '-dirty'
            with self.assertRaises(ValueError):
                module.assemble({'aios': dirty}, 'a' * 40)

    def test_check_urls_and_tars(self):
        manifest = fixture()
        sizes = {entry['tar']['url']: entry['tar']['size'] for image in manifest['images'].values() for entry in image['platforms'].values()}
        module.check_urls(manifest, head=lambda url: (200, sizes[url]))
        with self.assertRaises(ValueError):
            module.check_urls(manifest, head=lambda url: (200, sizes[url] - 1))
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            for name in ('aios', 'exttool'):
                data = name.encode()
                tar = manifest['images'][name]['platforms']['amd64']['tar']
                (tmp / tar['file']).write_bytes(data)
                tar['size'], tar['sha256'] = len(data), module.hashlib.sha256(data).hexdigest()
            module.check_tars(manifest, tmp, 'amd64')
            (tmp / manifest['images']['exttool']['platforms']['amd64']['tar']['file']).write_bytes(b'exttooL')
            with self.assertRaises(ValueError):
                module.check_tars(manifest, tmp, 'amd64')

    def test_cli_validate_fragment(self):
        with tempfile.TemporaryDirectory() as tmp:
            fragment = Path(tmp) / 'pointer-aios.json'
            fragment.write_text(json.dumps(fixture()['images']['aios']))
            result = subprocess.run([sys.executable, str(SCRIPT), 'validate-fragment', 'aios', str(fragment)], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            fragment.write_text(json.dumps({'repo': 'paios/aios', 'revision': 'x', 'platforms': {}}))
            result = subprocess.run([sys.executable, str(SCRIPT), 'validate-fragment', 'aios', str(fragment)], capture_output=True, text=True)
            self.assertEqual(result.returncode, 1)

    def test_cli_validate(self):
        result = subprocess.run([sys.executable, str(SCRIPT), 'validate', str(FIXTURE)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('aios=835b5bd44125', result.stdout)


if __name__ == '__main__':
    unittest.main()
