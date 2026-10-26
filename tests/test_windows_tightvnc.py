"""Opt-in, real Windows TightVNC install through the node's desktop API.

CRANIBOX_TEST_WINDOWS_VNC=1 python -m unittest discover -s tests -p test_windows_tightvnc.py -v
Intended for an administrator GitHub windows-latest runner with no UAC prompt.
The service remains on that disposable runner after sharing stops.
Only Python stdlib is used; Windows CryptoAPI supplies DES for VNC authentication.
"""
import ctypes
from ctypes import wintypes
import os
from pathlib import Path
import socket
import struct
import sys
import unittest

from workspace_web_harness import CORE, Harness, free_port, until

PINNED_SHA256 = '9693cb4cdc2e7e482e8e3f5890415b57ef87de3369fa5c337db516218a1170ce'


def des_encrypt(key, data):
    """DES ECB, no padding. CryptoAPI ignores the DES key's parity bits."""
    api = ctypes.WinDLL('advapi32', use_last_error=True)
    handle = ctypes.c_size_t
    byte_pointer = ctypes.POINTER(ctypes.c_ubyte)
    api.CryptAcquireContextW.argtypes = [ctypes.POINTER(handle), wintypes.LPCWSTR, wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD]
    api.CryptImportKey.argtypes = [handle, byte_pointer, wintypes.DWORD, handle, wintypes.DWORD, ctypes.POINTER(handle)]
    api.CryptSetKeyParam.argtypes = [handle, wintypes.DWORD, byte_pointer, wintypes.DWORD]
    api.CryptEncrypt.argtypes = [handle, handle, wintypes.BOOL, wintypes.DWORD, byte_pointer, ctypes.POINTER(wintypes.DWORD), wintypes.DWORD]
    api.CryptDestroyKey.argtypes = [handle]
    api.CryptReleaseContext.argtypes = [handle, wintypes.DWORD]
    for name in ('CryptAcquireContextW', 'CryptImportKey', 'CryptSetKeyParam', 'CryptEncrypt', 'CryptDestroyKey', 'CryptReleaseContext'):
        getattr(api, name).restype = wintypes.BOOL

    def check(result):
        if not result:
            raise ctypes.WinError(ctypes.get_last_error())

    provider, imported = handle(), handle()
    check(api.CryptAcquireContextW(ctypes.byref(provider), None, 'Microsoft Enhanced Cryptographic Provider v1.0', 1, 0xF0000000))
    try:
        # PLAINTEXTKEYBLOB, CUR_BLOB_VERSION, CALG_DES, eight key bytes.
        blob = struct.pack('<BBHII', 8, 2, 0, 0x6601, 8) + key
        buffer = (ctypes.c_ubyte * len(blob)).from_buffer_copy(blob)
        check(api.CryptImportKey(provider, buffer, len(blob), 0, 0, ctypes.byref(imported)))
        mode = wintypes.DWORD(2)  # CRYPT_MODE_ECB; KP_MODE = 4.
        check(api.CryptSetKeyParam(imported, 4, ctypes.cast(ctypes.byref(mode), byte_pointer), 0))
        buffer = (ctypes.c_ubyte * len(data)).from_buffer_copy(data)
        size = wintypes.DWORD(len(data))
        check(api.CryptEncrypt(imported, 0, False, 0, buffer, ctypes.byref(size), len(data)))
        return bytes(buffer[:size.value])
    finally:
        if imported.value:
            api.CryptDestroyKey(imported)
        api.CryptReleaseContext(provider, 0)


def vnc_response(password, challenge):
    key = bytes(int(f'{b:08b}'[::-1], 2) for b in password.encode('ascii')[:8].ljust(8, b'\0'))
    return des_encrypt(key, challenge)


def read_exact(connection, size):
    result = b''
    while len(result) < size:
        chunk = connection.recv(size - len(result))
        if not chunk:
            raise AssertionError('VNC connection closed during authentication')
        result += chunk
    return result


def authenticate(password):
    with socket.create_connection(('127.0.0.1', 5900), timeout=5) as connection:
        if read_exact(connection, 12) not in (b'RFB 003.007\n', b'RFB 003.008\n'):
            raise AssertionError('Unexpected TightVNC RFB greeting')
        connection.sendall(b'RFB 003.008\n')
        methods = read_exact(connection, read_exact(connection, 1)[0])
        if 2 not in methods or 1 in methods:
            raise AssertionError('TightVNC must require VNC password authentication')
        connection.sendall(b'\x02')
        connection.sendall(vnc_response(password, read_exact(connection, 16)))
        return read_exact(connection, 4) == b'\0\0\0\0'


class WindowsHarness(Harness):
    """Reuse sessions/CSRF and cleanup without the Linux command/snapshot fixture."""
    def initialize(self):
        self.web_port = free_port()
        self.command = Path(sys.executable)
        self.start_web()
        self.login()

    def start_web(self):
        binary = CORE if CORE.suffix == '.exe' else CORE.with_suffix('.exe')
        self.web = self.start(binary, 'serve', '--config-root', self.root,
                              '--command', self.command, '--port', self.web_port)
        until(lambda: self.http('GET', '/healthz')[0] == 200, seconds=30)


@unittest.skipUnless(sys.platform == 'win32' and os.environ.get('CRANIBOX_TEST_WINDOWS_VNC') == '1',
                     'opt in on a disposable Windows administrator runner')
class WindowsTightVncTests(unittest.TestCase):
    def service(self, **command):
        status, body = self.h.post('/api/desktop/service', command)
        self.assertEqual(status, 200, body)
        return body

    def test_install_authentication_loopback_and_stop(self):
        # Published DES known-answer vector catches wrong mode/key/blob handling.
        self.assertEqual(des_encrypt(bytes.fromhex('133457799bbcdff1'), bytes.fromhex('0123456789abcdef')),
                         bytes.fromhex('85e813540f0ab405'))
        self.h = WindowsHarness(unified=True)
        self.addCleanup(self.h.close)
        initial = self.service(operation='status')
        self.assertEqual(initial['provider'], 'tightvnc')
        self.assertTrue(initial['supported'] and initial['can_install'])
        self.assertFalse(initial['installed'], 'Use a clean disposable runner without an existing VNC server')
        plan = self.service(operation='plan')
        self.assertEqual(plan['sha256'], PINNED_SHA256)
        self.assertEqual(plan['license'], 'GPL-2.0')
        self.assertTrue(plan['administrator'])
        self.assertEqual(plan['commands'].count('********'), 2)
        self.assertFalse(self.service(operation='status')['enabled'])
        self.service(operation='install', plan_id=plan['id'])

        def finished():
            state = self.service(operation='status')
            return state if state['installation']['phase'] != 'running' else None

        installed = until(finished, seconds=1000)
        self.assertEqual(installed['installation']['phase'], 'succeeded', installed)
        self.assertTrue(installed['installed'] and installed['managed_installation'])
        self.assertFalse(installed['enabled'], 'Installing must not enable sharing')
        self.assertTrue(self.service(operation='start', provider='tightvnc', display='', port=5900)['running'])
        self.addCleanup(lambda: self.service(operation='stop'))
        with socket.create_connection(('127.0.0.1', 5900), timeout=5) as connection:
            self.assertIn(read_exact(connection, 12), (b'RFB 003.007\n', b'RFB 003.008\n'))
        password = self.service(operation='credentials')['password']
        self.assertTrue(len(password) == 8 and password.isalnum())
        self.assertTrue(authenticate(password), 'Credentials API password must authenticate')
        wrong = ('A' if password[0] != 'A' else 'B') + password[1:]
        self.assertFalse(authenticate(wrong), 'A wrong VNC password must be refused')
        addresses = {entry[4][0] for entry in socket.getaddrinfo(socket.gethostname(), None, socket.AF_INET, socket.SOCK_STREAM)
                     if not entry[4][0].startswith(('127.', '169.254.')) and entry[4][0] != '0.0.0.0'}
        self.assertTrue(addresses, 'Runner needs a non-loopback IPv4 address for isolation verification')
        for address in addresses:
            with socket.socket() as connection:
                connection.settimeout(3)
                self.assertNotEqual(connection.connect_ex((address, 5900)), 0, 'VNC must not accept connections on a non-loopback NIC')
        self.h.stop(self.h.web)
        self.h.start_web()
        self.h.login()
        until(lambda: self.service(operation='status')['running'], seconds=30)
        self.assertTrue(authenticate(self.service(operation='credentials')['password']))
        stopped = self.service(operation='stop')
        self.assertFalse(stopped['running'] or stopped['enabled'])
        code, config = self.h.post('/api/peers', {'operation': 'list'})
        self.assertEqual(code, 200)
        self.assertIsNone(config['desktop'])
        with socket.create_connection(('127.0.0.1', 5900), timeout=5) as connection:
            self.assertTrue(read_exact(connection, 12).startswith(b'RFB '), 'Stopping sharing must leave the Windows service running')
