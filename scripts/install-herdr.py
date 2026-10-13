#!/usr/bin/env python3
"""Install the independent terminal engine; verify official release SHA-256."""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tempfile
import urllib.request
import zipfile

VERSION = '0.9.3'
CHECKSUMS = {
    'herdr-linux-aarch64': '4de7aa3e25678812e92960de64f7c2aaa1bca1f0f80a3c5e559837e231e1f5c0',
    'herdr-linux-x86_64': '18a8dc65f1c2fa485884344356dea1cfd911c6f06cf46fa78e193f4087f4dba7',
    'herdr-macos-aarch64': '5173a3e0ae42d5d1ab7ebfa5d5e6329f7c3d23f8e1a3677c7ce3231da2884157',
    'herdr-macos-x86_64': 'db62d548ff3e832b087a96b1894a08d26be3905f1830309cd556783f215d4054',
    'herdr-windows-x86_64.zip': 'c75b1fa49f7a3ba4b8b11789912a6147e4214a3b6fd3556d0f80076c8887d795',
}

def download(url):
    request = urllib.request.Request(url, headers={'User-Agent': 'cranibox-herdr-installer'})
    with urllib.request.urlopen(request, timeout=120) as response:
        data = response.read(100 * 1024 * 1024 + 1)
    if len(data) > 100 * 1024 * 1024:
        raise ValueError('Release asset exceeds download limit')
    return data

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--version', default=VERSION)
    parser.add_argument('--output-directory', type=Path, default=Path(os.environ.get('AGENT_WORKSPACE_CONFIG_ROOT', str(Path.home()))) / 'bin')
    parser.add_argument('--asset-file', type=Path, help='Use an already downloaded official asset (checksum still required)')
    args = parser.parse_args()
    if not args.version or any(c not in '0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ.-' for c in args.version):
        parser.error('Invalid version')
    system = {'Linux': 'linux', 'Darwin': 'macos', 'Windows': 'windows'}.get(platform.system())
    arch = {'amd64': 'x86_64', 'x86_64': 'x86_64', 'arm64': 'aarch64', 'aarch64': 'aarch64'}.get(platform.machine().lower())
    if not system or not arch:
        parser.error('Unsupported OS or architecture')
    name = f'herdr-{system}-{arch}' + ('.zip' if system == 'windows' else '')
    if args.version == VERSION:
        expected = CHECKSUMS.get(name)
    else:
        release = json.loads(download(f'https://api.github.com/repos/devswha/herdr/releases/tags/v{args.version}'))
        digest = next((asset.get('digest', '') for asset in release['assets'] if asset['name'] == name), '')
        expected = digest.removeprefix('sha256:') if digest.startswith('sha256:') else None
    if not expected:
        parser.error('No verified official asset for this platform/version')
    data = args.asset_file.read_bytes() if args.asset_file else download(f'https://github.com/devswha/herdr/releases/download/v{args.version}/{name}')
    if hashlib.sha256(data).hexdigest() != expected:
        raise ValueError('Release SHA-256 mismatch; installation aborted')
    if system == 'windows':
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            matches = [entry for entry in archive.namelist() if Path(entry).name == 'herdr.exe']
            if len(matches) != 1:
                raise ValueError('Release must contain exactly one herdr.exe')
            data = archive.read(matches[0])
    args.output_directory.mkdir(parents=True, exist_ok=True)
    destination = args.output_directory / ('herdr.exe' if system == 'windows' else 'herdr')
    fd, temporary = tempfile.mkstemp(prefix='.herdr-', suffix='.exe' if system == 'windows' else '', dir=args.output_directory)
    try:
        with os.fdopen(fd, 'wb') as stream:
            stream.write(data)
        os.chmod(temporary, 0o755)
        version = subprocess.check_output([temporary, '--version'], text=True, timeout=10).strip()
        if args.version not in version.split():
            raise ValueError('Unexpected binary version: ' + version)
        if destination.exists():
            shutil.copy2(destination, destination.with_name(destination.name + '.previous'))
        os.replace(temporary, destination)
        print(f'{version} installed: {destination}')
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)

if __name__ == '__main__':
    main()
