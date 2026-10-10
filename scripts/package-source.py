#!/usr/bin/env python3
"""Archive tracked release sources; no profiles, build output or credentials."""
import hashlib
import io
import json
from pathlib import Path
import subprocess
import tarfile

ROOT = Path(__file__).resolve().parent.parent
metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--format-version', '1', '--no-deps', '--locked'], cwd=ROOT))
version = metadata['packages'][0]['version']
name = f'herdr-relay-{version}-source'
files = subprocess.check_output(['git', 'ls-files', '-z'], cwd=ROOT).decode().split('\0')
output = ROOT / 'target' / 'packages'
output.mkdir(parents=True, exist_ok=True)
archive = output / f'{name}.tar.gz'
manifest = []
with tarfile.open(archive, 'w:gz') as tar:
    for relative in sorted(filter(None, files)):
        path = ROOT / relative
        if path.is_symlink() or not path.is_file():
            raise SystemExit(f'Unexpected source entry: {relative}')
        if any(part in ('target', '.git', '.wrangler', 'node_modules', '__pycache__') for part in path.relative_to(ROOT).parts):
            raise SystemExit(f'Private or generated directory tracked: {relative}')
        data = path.read_bytes()
        manifest.append(f'{hashlib.sha256(data).hexdigest()}  {relative}\n')
        info = tar.gettarinfo(str(path), f'{name}/{relative}')
        info.uid = info.gid = 0
        info.uname = info.gname = ''
        tar.addfile(info, io.BytesIO(data))
    data = ''.join(manifest).encode()
    info = tarfile.TarInfo(f'{name}/SOURCE-MANIFEST.sha256')
    info.size = len(data)
    tar.addfile(info, io.BytesIO(data))
digest = hashlib.sha256(archive.read_bytes()).hexdigest()
archive.with_suffix('.gz.sha256').write_text(f'{digest}  {archive.name}\n')
print(archive)
