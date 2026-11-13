#!/usr/bin/env python3
"""Conservative release gate: both plugins must use the latest released host API.

Equal sources are stricter than equal wire schemas, intentionally: a new host
release requires an explicit pin update and validation before publishing Infra.
Requires Python 3.11+ and gh (GH_TOKEN in Actions); fails closed on API errors.
"""
import json
from pathlib import Path
import subprocess
import sys
import tomllib

HOST = 'HakanSeven12/OpenCADStudio'


def gh(path):
    return json.loads(subprocess.check_output(['gh', 'api', f'repos/{HOST}/{path}']))


def check(root, revision, host_lock):
    for name in ('Cargo.toml', 'civiel/Cargo.toml'):
        manifest = tomllib.loads((root / name).read_text())
        api = manifest['dependencies']['ocs_plugin_api']
        if api.get('git') != f'https://github.com/{HOST}' or api.get('rev') != revision:
            raise ValueError(f'{name}: pin ocs_plugin_api to released host {revision}')
    packages = tomllib.loads((root / 'Cargo.lock').read_text())['package']
    api = [p for p in packages if p['name'] == 'ocs_plugin_api']
    expected = f'git+https://github.com/{HOST}?rev={revision}#{revision}'
    if len(api) != 1 or api[0].get('source') != expected:
        raise ValueError('Cargo.lock: resolved ocs_plugin_api differs from released host')
    # These packages generate the serde type registry hashed by wire_schema().
    for name in ('opencaddocument', 'acadrust', 'serde-reflection'):
        host = [p for p in host_lock['package'] if p['name'] == name]
        if len(host) != 1:
            raise ValueError(f'Host lockfile: expected one {name}')
        identity = lambda p: (p['version'], p.get('source'), p.get('checksum'))
        if identity(host[0]) not in [identity(p) for p in packages if p['name'] == name]:
            raise ValueError(f'Cargo.lock: {name} differs from released host')


def main():
    import base64
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parent.parent
    release = gh('releases/latest')['tag_name']
    revision = gh(f'commits/{release}')['sha']
    content = gh(f'contents/Cargo.lock?ref={revision}')['content']
    host_lock = tomllib.loads(base64.b64decode(content).decode())
    check(root, revision, host_lock)
    print(f'Both plugins match {release} ({revision}), including locked codec sources')


if __name__ == '__main__':
    main()
