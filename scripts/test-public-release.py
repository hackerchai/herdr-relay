#!/usr/bin/env python3
"""Focused checks for the public publication boundary; no network needed."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('publish', Path(__file__).with_name('publish-public-release.py'))
publish = importlib.util.module_from_spec(spec)
spec.loader.exec_module(publish)


class PublicationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        artifacts = []
        for target in sorted(publish.TARGETS):
            name = f'herdr-relay-1.2.3-{target}.tar.gz'
            data = target.encode()
            digest = hashlib.sha256(data).hexdigest()
            (self.directory / name).write_bytes(data)
            (self.directory / f'{name}.sha256').write_text(f'{digest}  {name}\n')
            artifacts.append({'target': target, 'asset': name, 'sha256': digest})
        self.manifest = {'schema': 1, 'version': '1.2.3', 'artifacts': artifacts}
        self.save()

    def save(self):
        (self.directory / publish.MANIFEST).write_text(json.dumps(self.manifest))

    def test_source_and_unrelated_files_are_never_selected(self):
        (self.directory / 'herdr-relay-1.2.3-source.tar.gz').write_bytes(b'private')
        (self.directory / 'ci.log').write_text('private')
        version, files = publish.publication_files(self.directory)
        self.assertEqual(version, '1.2.3')
        self.assertEqual(len(files), 8)
        self.assertFalse(any('source' in p.name or p.suffix == '.log' for p in files))

    def test_wrong_digest_is_rejected_before_publication(self):
        (self.directory / self.manifest['artifacts'][0]['asset']).write_bytes(b'changed')
        with self.assertRaises(AssertionError):
            publish.publication_files(self.directory)

    def test_manifest_cannot_select_source_archive(self):
        self.manifest['artifacts'][0]['asset'] = 'herdr-relay-1.2.3-source.tar.gz'
        self.save()
        with self.assertRaises(AssertionError):
            publish.publication_files(self.directory)


if __name__ == '__main__':
    unittest.main()
