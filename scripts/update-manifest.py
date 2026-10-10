#!/usr/bin/env python3
"""Write the updater's readiness marker after validating all CLI archives."""
import hashlib
import json
from pathlib import Path
import tarfile
import tomllib

version = tomllib.loads(Path('Cargo.toml').read_text())['package']['version']
directory = Path('target/packages')
targets = (
    'aarch64-apple-darwin', 'x86_64-apple-darwin',
    'aarch64-unknown-linux-musl', 'x86_64-unknown-linux-musl',
)
artifacts = []
for target in targets:
    prefix = f'herdr-relay-{version}-{target}'
    archive = directory / f'{prefix}.tar.gz'
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    assert archive.with_suffix('.gz.sha256').read_text().split()[0] == digest
    with tarfile.open(archive) as bundle:
        assert bundle.extractfile(f'{prefix}/TARGET').read().decode().strip() == target
        assert bundle.getmember(f'{prefix}/bin/herdr-relay').isfile()
    artifacts.append({'target': target, 'asset': archive.name, 'sha256': digest})
(directory / 'herdr-relay-update.json').write_text(json.dumps({
    'schema': 1, 'version': version, 'artifacts': artifacts,
}, indent=2) + '\n')
