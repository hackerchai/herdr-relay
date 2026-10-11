#!/usr/bin/env python3
"""Upload the CLI archives and update manifest to the R2 download site.

Run from relay/ (for Wrangler) after the GitHub Release is final. Versioned
files are uploaded first; `latest/herdr-relay-update.json` is uploaded last, so
the updater never sees a version whose archives are missing. Retries upload the
same bytes again. `--check` writes a small marker object, proving that the
credentials can publish (they are scoped to this bucket's objects only).
"""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile

BUCKET = 'herdr-relay-dl'
MANIFEST = 'herdr-relay-update.json'
IMMUTABLE = 'public, max-age=31536000, immutable'
LATEST = 'public, max-age=300'


def plan(directory):
    """(key, file, cache-control) in upload order, after verifying checksums."""
    manifest = json.loads((directory / MANIFEST).read_text())
    version = manifest['version']
    if manifest.get('schema') != 1 or len(manifest.get('artifacts', [])) != 4:
        raise SystemExit('Unexpected update manifest; download site not updated.')
    uploads = []
    for artifact in manifest['artifacts']:
        archive = directory / artifact['asset']
        if hashlib.sha256(archive.read_bytes()).hexdigest() != artifact['sha256']:
            raise SystemExit(f'Checksum mismatch for {archive.name}; download site not updated.')
        checksum = archive.with_name(archive.name + '.sha256')
        uploads += [(f'v{version}/{archive.name}', archive, IMMUTABLE),
                    (f'v{version}/{checksum.name}', checksum, IMMUTABLE)]
    uploads.append((f'v{version}/{MANIFEST}', directory / MANIFEST, IMMUTABLE))
    uploads.append((f'latest/{MANIFEST}', directory / MANIFEST, LATEST))
    return uploads


def wrangler(*args):
    subprocess.run(['npx', 'wrangler', *args], check=True)


def main():
    if sys.argv[1:] == ['--check']:
        with tempfile.NamedTemporaryFile('w', suffix='.txt') as marker:
            marker.write('publish check\n')
            marker.flush()
            wrangler('r2', 'object', 'put', f'{BUCKET}/preflight/check.txt', '--remote',
                     '--file', marker.name, '--content-type', 'text/plain',
                     '--cache-control', 'no-store')
        return
    directory = Path(sys.argv[1] if len(sys.argv) > 1 else '../target/packages')
    for key, path, cache in plan(directory):
        kind = 'application/json' if key.endswith('.json') else (
            'text/plain' if key.endswith('.sha256') else 'application/gzip')
        wrangler('r2', 'object', 'put', f'{BUCKET}/{key}', '--remote', '--file', str(path),
                 '--content-type', kind, '--cache-control', cache)
        print('Uploaded', key)


if __name__ == '__main__':
    main()
