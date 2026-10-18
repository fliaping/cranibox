#!/usr/bin/env python3
"""Isolated x11vnc stand-in: real loopback TCP, no desktop or package access."""
import json
import os
from pathlib import Path
import socket
import sys
import threading

root = Path(os.environ['CRANIBOX_DESKTOP_TEST_ROOT'])
args = sys.argv[1:]
wayland = Path(sys.argv[0]).name == 'wayvnc'
if wayland and args == ['--version']:
    version = (root / 'wayvnc-version').read_text().strip() if (root / 'wayvnc-version').exists() else '0.9.1'
    print('wayvnc: ' + version)
    sys.exit(0)


def record(event):
    with (root / 'events.jsonl').open('a') as stream:
        stream.write(json.dumps({'event': event, 'pid': os.getpid(), 'args': args}) + '\n')


if (root / 'fail-start').exists():
    record('failed')
    sys.exit(3)

# Refuse any invocation that would accidentally expose the fixture externally.
if wayland:
    assert args[-2] == '127.0.0.1'
    assert os.environ['WAYLAND_DISPLAY'] == 'wayland-42'
    config_file = Path(args[args.index('--config') + 1])
    assert config_file.stat().st_mode & 0o777 == 0o600
    config = dict(line.split('=', 1) for line in config_file.read_text().splitlines())
    assert config['enable_auth'] == 'true' and config['relax_encryption'] == 'true'
    assert config['username'] == 'cranibox' and config['address'] == '127.0.0.1'
    assert len(config['password']) == 8 and config['password'].isalnum()
    control = Path(args[args.index('--socket') + 1])
    assert control.parent.stat().st_mode & 0o777 == 0o700
    ipc = socket.socket(socket.AF_UNIX)
    ipc.bind(str(control))
    port = int(args[-1])
else:
    assert args[args.index('-listen') + 1] == '127.0.0.1'
    assert all(flag in args for flag in ('-localhost', '-noremote', '-norc', '-noipv6', '-forever', '-shared'))
    password_file = Path(args[args.index('-passwdfile') + 1])
    assert password_file.is_file() and len(password_file.read_text().strip()) == 8
    port = int(args[args.index('-rfbport') + 1])
listener = socket.socket()
listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
listener.bind(('127.0.0.1', port))
listener.listen()
record('started')


def serve(connection):
    try:
        connection.sendall(b'RFB 003.008\n')
        while data := connection.recv(65536):
            connection.sendall(data)
    except OSError:
        pass
    finally:
        connection.close()


while True:
    connection, _ = listener.accept()
    threading.Thread(target=serve, args=(connection,), daemon=True).start()
