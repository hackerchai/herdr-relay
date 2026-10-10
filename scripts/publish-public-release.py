#!/usr/bin/env python3
"""Publish only the CLI artifacts named by the validated readiness manifest."""
import hashlib
import json
from pathlib import Path
import re
import subprocess

REPOSITORY = 'hackerchai/herdr-relay-releases'
DIRECTORY = Path('target/packages')
MANIFEST = 'herdr-relay-update.json'
TARGETS = {
    'aarch64-apple-darwin', 'x86_64-apple-darwin',
    'aarch64-unknown-linux-musl', 'x86_64-unknown-linux-musl',
}


def publication_files(directory):
    manifest = json.loads((directory / MANIFEST).read_text())
    version = manifest['version']
    assert manifest['schema'] == 1 and re.fullmatch(r'\d+\.\d+\.\d+', version)
    artifacts = manifest['artifacts']
    assert len(artifacts) == 4 and {a['target'] for a in artifacts} == TARGETS
    files = []
    for artifact in artifacts:
        name = f"herdr-relay-{version}-{artifact['target']}.tar.gz"
        assert artifact['asset'] == name
        archive = directory / name
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        assert digest == artifact['sha256']
        checksum = directory / f'{name}.sha256'
        assert checksum.read_text().split() == [digest, name]
        files.extend((archive, checksum))
    return version, files


def gh(*args, **kwargs):
    return subprocess.run(['gh', *args, '--repo', REPOSITORY], check=True, **kwargs)


def main():
    version, files = publication_files(DIRECTORY)
    tag = f'v{version}'
    # List first so authentication/network failures cannot look like a missing release.
    releases = json.loads(gh('release', 'list', '--limit', '100', '--json',
                            'tagName,isDraft', capture_output=True, text=True).stdout)
    existing = next((r for r in releases if r['tagName'] == tag), None)
    expected = {p.name for p in files} | {'LICENSE', MANIFEST}
    if existing and not existing['isDraft']:
        release = json.loads(gh('release', 'view', tag, '--json', 'assets',
                                capture_output=True, text=True).stdout)
        assert {a['name'] for a in release['assets']} == expected, 'Incomplete public release'
        # Completed releases are immutable by convention. A retry must not replace
        # an archive while an existing client is using its published digest.
        print(f'{REPOSITORY} {tag} is already complete; leaving its assets unchanged.')
        return
    notes = DIRECTORY / 'public-release-notes.md'
    notes.write_text(f'''# Herdr Relay {version}

Native CLI binaries for macOS (Apple Silicon and Intel) and Linux (arm64 and x86_64).
Select your archive, verify its adjacent SHA-256 checksum, extract it, and run `./install.sh`.
Rust and Node are not needed for binary installation. Herdr 0.9.3 is installed separately.

Run `herdr-relay serve` to share sessions, `herdr-relay pair` to paste a pairing link,
and `herdr-relay` to choose a host and session. Run `herdr-relay update` for future releases.

This release includes interactive session selection, named hosts, pairing links,
macOS/Linux user services, and coordinated CLI/service updates from this public channel.
Active sessions defer the service restart. Identity, grants, and sharing settings are preserved.

These CLI binaries passed the required Rust/TypeScript checks before publication.
The matching public crate and production Relay deployment completed before this release.
MIT license. Source packages are available from https://crates.io/crates/herdr-relay/{version}.
GitHub's automatic source archives contain only this distribution repository's documentation.
''')
    if not existing:
        # The tag points at the distribution repository's documentation, not source history.
        gh('release', 'create', tag, '--draft', '--title', f'Herdr Relay {version}',
           '--notes-file', str(notes), '--target', 'main')
    else:
        gh('release', 'edit', tag, '--notes-file', str(notes))
    gh('release', 'upload', tag, *map(str, files), 'LICENSE', '--clobber')
    gh('release', 'upload', tag, str(DIRECTORY / MANIFEST), '--clobber')
    gh('release', 'edit', tag, '--draft=false', '--latest')


if __name__ == '__main__':
    main()
