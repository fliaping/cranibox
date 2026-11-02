#!/usr/bin/env python3
"""Opt-in integration check against the installed persistent Debian runtime.

Builds a small local package, resolves a real dependency, checks installation
scripts and desktop entries, upgrades it and removes it. No host apt is used.
"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile


CONFIG = Path(os.environ.get('AGENT_WORKSPACE_CONFIG_ROOT', '/config'))
BACKEND = CONFIG / 'bin/agent-workspace-deb'
FRONTEND = CONFIG / 'bin/agent-workspace-deb-installer'
STATE = CONFIG / '.local/share/agent-workspace/deb-runtime'
NAME = 'agent-workspace-deb-smoke'
MARKER = '/etc/agent-workspace-deb-smoke'


def run(*args, capture=False, check=True):
    return subprocess.run([str(BACKEND), *args], check=check, text=True,
                          stdout=subprocess.PIPE if capture else None)


def build(directory, version):
    tree = directory / ('package-' + version)
    for path in ('DEBIAN', 'usr/bin', 'usr/share/applications'):
        (tree / path).mkdir(parents=True)
    for directory in [tree, *tree.rglob('*')]:
        if directory.is_dir():
            directory.chmod(0o755)
    (tree / 'DEBIAN/control').write_text(
        f'Package: {NAME}\nVersion: {version}\nArchitecture: all\n'
        'Maintainer: Cranibox Tests <noreply@example.invalid>\n'
        'Depends: x11-utils\nDescription: Persistent package installation smoke test\n')
    postinst = tree / 'DEBIAN/postinst'
    postinst.write_text(f'#!/bin/sh\nset -e\nprintf "%s" "{version}" > {MARKER}\n')
    postinst.chmod(0o755)
    postrm = tree / 'DEBIAN/postrm'
    postrm.write_text(f'#!/bin/sh\nif [ "$1" = remove ] || [ "$1" = purge ]; then rm -f {MARKER}; fi\n')
    postrm.chmod(0o755)
    executable = tree / 'usr/bin' / NAME
    executable.write_text(
        '#!/bin/sh\nset -e\n'
        'if [ "$1" = --self-test ]; then\n'
        f'  cat {MARKER}\n  printf "\\n"\n  shift\n  printf "%s\\n" "$@"\n'
        'else\n  exec xmessage -timeout 5 "Persistent Debian runtime works"\nfi\n')
    executable.chmod(0o755)
    (tree / f'usr/share/applications/smoke-{version}.desktop').write_text(
        '[Desktop Entry]\nType=Application\nName=Persistent Runtime Test\n'
        f'Exec=/usr/bin/{NAME}\nIcon=utilities-terminal\nCategories=Utility;\n')
    archive = directory / f'test package {version}.deb'
    subprocess.run(['dpkg-deb', '--build', '--root-owner-group', str(tree), str(archive)], check=True)
    return archive


def main():
    if Path(MARKER).exists():
        raise RuntimeError('Refusing to use an existing host marker.')
    original = hashlib.sha256(Path('/var/lib/dpkg/status').read_bytes()).hexdigest()
    cache = CONFIG / '.cache/agent-workspace/deb-tests'
    cache.mkdir(parents=True, exist_ok=True)
    run('init')
    previous_launchers = set()
    with tempfile.TemporaryDirectory(dir=cache) as temporary:
        for version in ('1.0', '2.0'):
            archive = build(Path(temporary), version)
            before = hashlib.sha256((STATE / 'rootfs/var/lib/dpkg/status').read_bytes()).hexdigest()
            run('install', '--dry-run', str(archive))
            assert hashlib.sha256((STATE / 'rootfs/var/lib/dpkg/status').read_bytes()).hexdigest() == before
            env = dict(os.environ, DISPLAY='', WAYLAND_DISPLAY='')
            subprocess.run([str(FRONTEND), '--yes', str(archive)], env=env, check=True)
            record = json.loads((STATE / 'packages.json').read_text())['packages'][NAME]
            assert record['status'] == 'installed' and record['version'] == version, record
            launchers = set(record['launchers'])
            assert len(launchers) == 1
            for stale in previous_launchers - launchers:
                assert not (CONFIG / '.local/share/applications' / stale).exists()
            previous_launchers = launchers
            output = run('run', '--', '/usr/bin/' + NAME, '--self-test',
                         'one argument;$(not-a-command)', capture=True).stdout
            assert output == version + '\none argument;$(not-a-command)\n', repr(output)
            assert not Path(MARKER).exists()
            assert (STATE / ('rootfs' + MARKER)).read_text() == version
            # Fresh processes read the same persisted package database and launchers.
            status = json.loads(run('status', capture=True).stdout)
            assert status['packages'][NAME]['version'] == version
        backup = Path(run('backup', capture=True).stdout.strip())
        assert backup.is_file()
        run('remove', '--dry-run', NAME)
        assert (STATE / ('rootfs' + MARKER)).exists()
        run('remove', '--yes', NAME)
        assert not (STATE / ('rootfs' + MARKER)).exists()
        for launcher in previous_launchers:
            assert not (CONFIG / '.local/share/applications' / launcher).exists()
        run('restore', '--yes', str(backup))
        output = run('run', '--', '/usr/bin/' + NAME, '--self-test', capture=True).stdout
        assert output.startswith('2.0\n'), repr(output)
        for launcher in previous_launchers:
            assert (CONFIG / '.local/share/applications' / launcher).is_file()
        run('remove', '--yes', NAME)
    assert hashlib.sha256(Path('/var/lib/dpkg/status').read_bytes()).hexdigest() == original
    assert not Path('/usr/bin/' + NAME).exists()
    print('PASS: dependency resolution, dry-run, desktop install, postinst, argv, upgrade, removal, backup, restore, host boundary')


if __name__ == '__main__':
    main()
