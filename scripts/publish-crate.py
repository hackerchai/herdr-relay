#!/usr/bin/env python3
"""Publish an immutable crate, or verify an exact retry of a published version."""
import hashlib
import json
from pathlib import Path
import subprocess
import urllib.error
import urllib.request

subprocess.run(['cargo', 'package', '--locked'], check=True)
package = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--no-deps', '--format-version', '1']))['packages'][0]
name, version = package['name'], package['version']
archive = Path('target/package') / f'{name}-{version}.crate'
request = urllib.request.Request(f'https://crates.io/api/v1/crates/{name}/{version}', headers={'User-Agent': 'herdr-relay-release (https://github.com/hackerchai/herdr-relay)'})
try:
    with urllib.request.urlopen(request, timeout=30) as response:
        published = json.load(response)['version']
except urllib.error.HTTPError as error:
    if error.code != 404:
        raise
    subprocess.run(['cargo', 'publish', '--locked'], check=True)
else:
    if published['checksum'] != hashlib.sha256(archive.read_bytes()).hexdigest():
        raise SystemExit('This version already exists with different source bytes; publish a new version.')
    print(f'{name} {version} is already published with the exact crate checksum.')
