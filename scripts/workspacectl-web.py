#!/usr/bin/env python3
"""Install and manage the persistent, loopback-only Web control center."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
from urllib.parse import urlsplit

ROOT = Path(os.environ.get('AGENT_WORKSPACE_CONFIG_ROOT', os.environ.get('CONFIG_ROOT', '/config')))
RUNTIME = ROOT / 'opt/agent-workspace-web'
CONFIG = ROOT / '.config/agent-workspace/web.json'
UNIT = 'agent-workspace-web.service'
DEFAULTS = {'port': 9080, 'publicUrl': '', 'workspaceUrl': 'http://localhost:8443',
            'desktopUrl': 'https://localhost:3001', 'runtimeUrl': 'http://127.0.0.1:9090'}


def config():
    return {**DEFAULTS, **(json.loads(CONFIG.read_text()) if CONFIG.exists() else {})}


def unit(*args):
    return subprocess.run(['systemctl', '--user', *args], check=True, start_new_session=True)


def install():
    source = Path(__file__).resolve().parent.parent
    subprocess.run(['bash', str(source / 'scripts/install-workspace-web.sh')], check=True)


def valid_url(value):
    parsed = urlsplit(value)
    if parsed.scheme not in {'http', 'https'} or not parsed.hostname or parsed.username or parsed.password:
        raise argparse.ArgumentTypeError('Use an http(s) URL without credentials')
    if parsed.query or parsed.fragment:
        raise argparse.ArgumentTypeError('URL must not include query or fragment')
    return value


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='action')
    for action in ['install', 'start', 'stop', 'restart', 'status', 'url']:
        sub.add_parser(action)
    configure = sub.add_parser('configure', help='Configure trusted gateway origin and entry links')
    configure.add_argument('--public-url', type=valid_url)
    configure.add_argument('--workspace-url', type=valid_url)
    configure.add_argument('--desktop-url', type=valid_url)
    configure.add_argument('--runtime-url', type=valid_url)
    configure.add_argument('--port', type=int)
    args = parser.parse_args()
    if args.action == 'install':
        install()
    elif args.action == 'configure':
        value = config()
        if args.port is not None:
            if not 1024 <= args.port <= 65535:
                parser.error('port must be between 1024 and 65535')
            value['port'] = args.port
        if args.public_url and urlsplit(args.public_url).path not in {'', '/'}:
            parser.error('public-url must use a dedicated origin without a path')
        if args.runtime_url:
            parsed = urlsplit(args.runtime_url)
            if parsed.scheme != 'http' or parsed.hostname not in {'127.0.0.1', 'localhost', '::1'} or parsed.path not in {'', '/'}:
                parser.error('runtime-url must be an HTTP loopback origin')
        for key in ['public_url', 'workspace_url', 'desktop_url', 'runtime_url']:
            if getattr(args, key) is not None:
                camel = key.split('_')[0] + 'Url'
                value[camel] = getattr(args, key).rstrip('/')
        CONFIG.parent.mkdir(parents=True, exist_ok=True)
        temporary = CONFIG.with_suffix('.tmp')
        temporary.write_text(json.dumps(value, indent=2) + '\n')
        temporary.chmod(0o600)
        temporary.replace(CONFIG)
        if (ROOT / '.config/systemd/user' / UNIT).exists():
            unit('restart', UNIT)
        print(json.dumps(value, indent=2))
    elif args.action == 'url':
        value = config()
        print(value['publicUrl'] or f'http://127.0.0.1:{value["port"]}')
    elif args.action in {'start', 'stop', 'restart', 'status'}:
        unit(args.action, UNIT)
    else:
        parser.print_help()


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f'workspacectl web: {error}', file=sys.stderr)
        sys.exit(1)
