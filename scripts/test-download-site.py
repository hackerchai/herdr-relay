#!/usr/bin/env python3
"""Focused checks for the R2 download site upload plan; no network needed."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('site', Path(__file__).with_name('publish-download-site.py'))
site = importlib.util.module_from_spec(spec)
spec.loader.exec_module(site)
TARGETS = ('aarch64-apple-darwin', 'x86_64-apple-darwin',
           'aarch64-unknown-linux-musl', 'x86_64-unknown-linux-musl')


class PlanTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        artifacts = []
        for target in TARGETS:
            name = f'herdr-relay-1.2.3-{target}.tar.gz'
            data = target.encode()
            digest = hashlib.sha256(data).hexdigest()
            (self.directory / name).write_bytes(data)
            (self.directory / f'{name}.sha256').write_text(f'{digest}  {name}\n')
            artifacts.append({'target': target, 'asset': name, 'sha256': digest})
        self.manifest = {'schema': 1, 'version': '1.2.3', 'artifacts': artifacts}
        (self.directory / site.MANIFEST).write_text(json.dumps(self.manifest))

    def test_archives_first_and_the_latest_manifest_last(self):
        (self.directory / 'herdr-relay-1.2.3-source.tar.gz').write_bytes(b'source')
        plan = site.plan(self.directory)
        keys = [key for key, _, _ in plan]
        self.assertEqual(keys[-1], 'latest/herdr-relay-update.json')
        self.assertEqual(keys[-2], 'v1.2.3/herdr-relay-update.json')
        self.assertEqual(len(keys), 10)
        self.assertFalse(any('source' in key for key in keys))
        for key, _, cache in plan[:-1]:
            self.assertTrue(key.startswith('v1.2.3/'), key)
            self.assertIn('immutable', cache)
        self.assertIn('max-age=300', plan[-1][2])

    def test_a_checksum_mismatch_stops_the_upload(self):
        name = f'herdr-relay-1.2.3-{TARGETS[0]}.tar.gz'
        (self.directory / name).write_bytes(b'tampered')
        with self.assertRaises(SystemExit):
            site.plan(self.directory)


if __name__ == '__main__':
    unittest.main()
