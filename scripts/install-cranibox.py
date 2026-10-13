"""Atomic server installation with persistent backups and rollback of service state."""
import datetime
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import uuid


def install(root, binary, ensure, start):
    if any(c in str(root) for c in '\n\r"\\%'):
        raise ValueError('Unsupported config root for service unit')
    core = root / 'bin/cranibox'
    unit = root / '.config/systemd/user/agent-workspace-web.service'
    runtime_unit = unit.with_name('agent-workspace-runtime.service')
    credentials = root / '.config/agent-workspace/runtime.json'
    marker = credentials.with_name('unified-server')
    first = not marker.exists()
    backup = root / '.local/state/cranibox' / ('install-' + datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%SZ') + '-' + uuid.uuid4().hex[:8])
    backup.mkdir(parents=True, mode=0o700)
    old_manager = root / 'bin/workspace-services'
    records = []
    states = {}

    def manager(path, *args, check=True):
        return subprocess.run([str(path), '--user', *args], capture_output=True, text=True, check=check, timeout=20)

    if old_manager.exists():
        for service in (unit, runtime_unit):
            if service.exists():
                states[service.name] = {
                    'active': manager(old_manager, 'is-active', service.name, check=False).returncode == 0,
                    'enabled': manager(old_manager, 'is-enabled', service.name, check=False).returncode == 0,
                }

    def save(path):
        if any(record['path'] == str(path.relative_to(root)) for record in records):
            return
        record = {'path': str(path.relative_to(root)), 'kind': 'missing'}
        if path.is_symlink():
            record.update(kind='symlink', target=os.readlink(path))
        elif path.exists():
            target = backup / str(len(records))
            shutil.copy2(path, target)
            record.update(kind='file', backup=target.name)
        records.append(record)
        (backup / 'manifest.json').write_text(json.dumps({'files': records, 'services': states}, indent=2))

    def link(path, target):
        if path.is_symlink() and os.readlink(path) == str(target):
            return
        save(path)
        path.parent.mkdir(parents=True, exist_ok=True)
        temporary = path.with_name('.' + path.name + '-' + uuid.uuid4().hex)
        temporary.symlink_to(target)
        os.replace(temporary, path)

    try:
        # The compatibility port moves into the Web process; retire its previous owner first.
        if first and runtime_unit.exists() and old_manager.exists():
            manager(old_manager, 'stop', runtime_unit.name)
            manager(old_manager, 'disable', runtime_unit.name)
        if start and unit.exists() and old_manager.exists():
            manager(old_manager, 'stop', unit.name)
        if not ensure or not core.exists():
            save(core)
            core.parent.mkdir(parents=True, exist_ok=True)
            fd, name = tempfile.mkstemp(prefix='.cranibox-', dir=core.parent)
            os.close(fd)
            try:
                shutil.copyfile(binary, name)
                os.chmod(name, 0o755)
                os.replace(name, core)
            finally:
                if os.path.exists(name):
                    os.unlink(name)
        for name in ('workspacectl', 'workspace-services', 'workspace-runtime'):
            link(root / 'bin' / name, core)
        link(root / 'opt/agent-workspace-web/workspace-web', core)
        link(root / 'bin/systemctl', root / 'bin/workspace-services')
        link(root / '.local/bin/systemctl', root / 'bin/workspace-services')
        link(root / '.local/bin/workspacectl', root / 'bin/workspacectl')
        link(root / '.local/bin/cranibox', core)
        save(credentials)
        if not credentials.exists():
            subprocess.run([str(core), 'runtime', 'init', '--credentials', str(credentials)], check=True, timeout=10)
        credentials.chmod(0o600)
        if not unit.exists():
            save(unit)
            unit.parent.mkdir(parents=True, exist_ok=True)
            unit.write_text(f'''[Unit]
Description=Cranibox Web, node gateway and workspace control (Rust)

[Service]
Type=simple
Environment="HOME={root}"
Environment="AGENT_WORKSPACE_CONFIG_ROOT={root}"
Environment="PATH={root}/bin:{root}/.local/bin:{root}/.npm-global/bin:{root}/node/bin:{root}/.linuxbrew/bin:/usr/local/bin:/usr/bin:/bin"
ExecStart="{core}" serve --config-root "{root}"
Restart=on-failure
RestartSec=3
TimeoutStopSec=10

[Install]
WantedBy=default.target
''')
            unit.chmod(0o644)
        # Existing workspace-web ExecStart uses the compatible alias, retaining all custom arguments.
        save(marker)
        marker.parent.mkdir(parents=True, exist_ok=True)
        marker.write_text('1\n')
        manager(root / 'bin/workspace-services', 'daemon-reload')
        if 'agent-workspace-web.service' not in states or start:
            manager(root / 'bin/workspace-services', 'enable', unit.name)
        if start:
            manager(root / 'bin/workspace-services', 'restart', unit.name)
        print(f'Cranibox server installed: {core}; rollback files: {backup}')
    except BaseException:
        for record in reversed(records):
            path = root / record['path']
            if path.exists() or path.is_symlink():
                path.unlink()
            if record['kind'] == 'symlink':
                path.symlink_to(record['target'])
            elif record['kind'] == 'file':
                shutil.copy2(backup / record['backup'], path)
        if old_manager.exists():
            manager(old_manager, 'daemon-reload', check=False)
            for name, state in states.items():
                manager(old_manager, 'enable' if state['enabled'] else 'disable', name, check=False)
                if state['active']:
                    manager(old_manager, 'restart', name, check=False)
        raise


if __name__ == '__main__':
    install(Path(sys.argv[1]).absolute(), Path(sys.argv[2]).absolute(), sys.argv[3] == 'true', sys.argv[4] == 'true')
