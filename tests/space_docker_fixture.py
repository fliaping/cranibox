#!/usr/bin/env python3
"""Strict fake Docker CLI. Starts a real Rust node instead of touching any engine."""
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time


def main():
    root = Path(os.environ['CRANIBOX_SPACE_FIXTURE'])
    state_file = root / 'engine.json'
    state = json.loads(state_file.read_text())
    argv = sys.argv[1:]
    state['events'].append(argv)
    state_file.write_text(json.dumps(state))

    def emit(value):
        print(json.dumps(value))

    def save():
        state_file.write_text(json.dumps(state))

    def labels():
        return dict(argv[i+1].split('=', 1) for i, value in enumerate(argv) if value == '--label')

    def source():
        return dict(value.split('=', 1) for value in argv[argv.index('--mount')+1].split(','))['source']

    if argv[0] == 'context':
        emit('ssh://remote.example.com' if (root / 'remote-context').exists() else 'unix:///fake/docker.sock')
    elif argv[0] == 'info':
        emit({'Name': 'isolated-test-engine', 'ServerVersion': 'test', 'OSType': 'linux', 'OperatingSystem': 'Linux fixture', 'Architecture': 'x86_64'})
    elif argv[:2] == ['image', 'pull']:
        if (root / 'fail-pull').exists():
            sys.exit('image download failed: retry when online')
        (root / 'missing-image').unlink(missing_ok=True)
        print('image downloaded')
    elif argv[:2] == ['image', 'inspect']:
        if (root / 'missing-image').exists():
            sys.exit('No such image: local build is not loaded')
        emit({} if (root / 'old-image').exists() else {'io.cranibox.space-protocol': '1'})
    elif argv[:2] in (['container', 'ls'], ['volume', 'ls']):
        kind = 'containers' if argv[0] == 'container' else 'volumes'
        prefix = 'name=^/' if kind == 'containers' else 'name=^'
        rule = argv[argv.index('--filter')+1]
        assert rule.startswith(prefix) and rule.endswith('$'), rule
        name = rule[len(prefix):-1]
        if name in state[kind]:
            print(name)
    elif argv[:2] == ['container', 'inspect'] and argv[-1] == os.environ.get('HOSTNAME'):
        emit({'Config': {'Hostname': os.environ['HOSTNAME']}, 'State': {'Running': True}, 'NetworkSettings': {'Networks': {'fixture-network': {}}}})
    elif argv[:2] in (['container', 'inspect'], ['volume', 'inspect']):
        emit(state['containers' if argv[0] == 'container' else 'volumes'][argv[-1]])
    elif argv[:2] == ['volume', 'create']:
        name = argv[-1]
        state['volumes'][name] = {'Name': name, 'Labels': labels()}
        (root / name).mkdir(exist_ok=True)
        save()
        print(name)
    elif argv[0] == 'run':
        # Volume seeding is idempotent and retains the existing node configuration.
        config = json.load(sys.stdin)
        path = root / source() / 'peers.json'
        if not path.exists():
            config['listener']['address'] = state['address'] + ':' + str(state['port'])
            path.write_text(json.dumps(config))
        print('volume initialized')
    elif argv[:2] == ['container', 'create']:
        name = argv[argv.index('--name')+1]
        assert name not in state['containers']
        assert '--privileged' not in argv and 'docker.sock' not in ' '.join(argv)
        environment = Path(argv[argv.index('--env-file')+1]).read_text()
        assert 'PASSWORD=' in environment and 'START_DOCKER=false' in environment
        state['containers'][name] = {
            'Id': 'fixture-' + name,
            'Config': {'Labels': labels()}, 'State': {'Running': False}, 'volume': source(),
            'NetworkSettings': {'Networks': {'fixture-network': {'IPAddress': state['address']}}, 'Ports': {
                '9020/tcp': [{'HostIp': '127.0.0.1', 'HostPort': str(state['port'])}],
                '3001/tcp': [{'HostIp': '127.0.0.1', 'HostPort': '43001'}],
                '8443/tcp': [{'HostIp': '127.0.0.1', 'HostPort': '48443'}],
            }}}
        save()
        print(name)
    elif argv[:2] == ['container', 'start']:
        container = state['containers'][argv[-1]]
        if not container['State']['Running']:
            home = root / container['volume']
            with (home / 'node.log').open('ab') as log:
                child = subprocess.Popen([state['node'], '--config', str(home / 'settings.json'), 'peer', 'serve'],
                    stdin=subprocess.DEVNULL, stdout=log, stderr=log,
                    env={**os.environ, 'HOME': str(home)}, start_new_session=True)
            container['pid'] = child.pid
            state['pids'].append(child.pid)
            container['State']['Running'] = True
        save()
        print(argv[-1])
    elif argv[:2] == ['container', 'stop']:
        container = state['containers'][argv[-1]]
        if container['State']['Running']:
            try:
                os.kill(container['pid'], signal.SIGTERM)
            except ProcessLookupError:
                pass
            time.sleep(.15)
            container['State']['Running'] = False
        save()
        print(argv[-1])
    elif argv[:2] == ['container', 'rm']:
        assert '--volumes' not in argv and '-v' not in argv
        del state['containers'][argv[-1]]
        save()
        print(argv[-1])
    else:
        raise AssertionError('Unexpected Docker command: ' + repr(argv))


if __name__ == '__main__':
    main()
