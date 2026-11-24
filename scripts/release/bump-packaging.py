#!/usr/bin/env python3
"""Update packaging manifests from a release tag and its SHA256SUMS."""
import argparse
from pathlib import Path
import re
import json
import urllib.request

from release_common import ROOT, sha256, version_for_tag

PACKAGING = ROOT / "packaging"
ASSETS = {
    "macos-arm64": "cranibox-macos-arm64",
    "linux-x86_64": "cranibox-linux-x86_64",
    "linux-arm64": "cranibox-linux-arm64",
    "windows-cli": "cranibox-windows-x86_64.exe",
}


def load_checksums(path):
    checksums = {}
    for line in Path(path).read_text().splitlines():
        match = re.fullmatch(r"([a-fA-F0-9]{64}) [ *](.+)", line)
        if not match:
            raise ValueError("Invalid SHA256SUMS entry.")
        checksums[match.group(2)] = match.group(1).lower()
    return checksums


def download_checksums(tag, destination):
    url = f"https://github.com/fliaping/cranibox/releases/download/{tag}/SHA256SUMS"
    with urllib.request.urlopen(url) as response:
        destination.write_bytes(response.read())
    return load_checksums(destination)


def require(checksums, name):
    if name not in checksums:
        raise ValueError(f"SHA256SUMS is missing {name}.")
    return checksums[name]


def replace_once(path, pattern, replacement, flags=0):
    text = path.read_text()
    updated, count = re.subn(pattern, replacement, text, count=1, flags=flags)
    if count != 1:
        raise ValueError(f"{path}: expected one match for {pattern!r}, found {count}.")
    path.write_text(updated)


def bump(tag, checksums_path=None):
    version = version_for_tag(tag)
    if checksums_path is None:
        checksums = download_checksums(tag, Path("/tmp/SHA256SUMS"))
    else:
        checksums = load_checksums(checksums_path)
    macos = require(checksums, "cranibox-macos-arm64")
    linux_x64 = require(checksums, "cranibox-linux-x86_64")
    linux_arm = require(checksums, "cranibox-linux-arm64")
    windows_cli = require(checksums, "cranibox-windows-x86_64.exe")
    installer_name = f"Cranibox_{version}_x64-setup.exe"
    windows_setup = require(checksums, installer_name)
    sums = require(checksums, "SHA256SUMS") if "SHA256SUMS" in checksums else None
    # The uploaded SHA256SUMS file is not listed inside itself; AUR also pins
    # the checksum of the downloaded SHA256SUMS asset when present in a local file set.
    # When bumping from a release SHA256SUMS, compute it if the file is local.
    if checksums_path is not None:
        sums = sha256(Path(checksums_path))

    formula = PACKAGING / "homebrew/cranibox.rb"
    replace_once(formula, r'version "[^"]+"', f'version "{version}"')
    replace_once(
        formula,
        r'url "https://github.com/fliaping/cranibox/releases/download/v[^/]+/cranibox-macos-arm64"',
        f'url "https://github.com/fliaping/cranibox/releases/download/v{version}/cranibox-macos-arm64"',
    )
    replace_once(formula, r'sha256 "[0-9a-f]{64}"', f'sha256 "{macos}"')

    scoop = PACKAGING / "scoop/cranibox.json"
    manifest = json.loads(scoop.read_text())
    manifest["version"] = version
    url = f"https://github.com/fliaping/cranibox/releases/download/v{version}/cranibox-windows-x86_64.exe#/cranibox.exe"
    manifest["architecture"]["64bit"]["url"] = url
    manifest["architecture"]["64bit"]["hash"] = windows_cli
    scoop.write_text(json.dumps(manifest, indent=2) + "\n")

    for name in (
        "fliaping.Cranibox.yaml",
        "fliaping.Cranibox.installer.yaml",
        "fliaping.Cranibox.locale.en-US.yaml",
    ):
        path = PACKAGING / "winget" / name
        replace_once(path, r"PackageVersion: \S+", f"PackageVersion: {version}")
    installer = PACKAGING / "winget/fliaping.Cranibox.installer.yaml"
    replace_once(
        installer,
        r"InstallerUrl: https://github.com/fliaping/cranibox/releases/download/v\S+/Cranibox_\S+_x64-setup.exe",
        f"InstallerUrl: https://github.com/fliaping/cranibox/releases/download/v{version}/{installer_name}",
    )
    replace_once(installer, r"InstallerSha256: [0-9A-Fa-f]{64}", f"InstallerSha256: {windows_setup.upper()}")
    locale = PACKAGING / "winget/fliaping.Cranibox.locale.en-US.yaml"
    replace_once(
        locale,
        r"ReleaseNotesUrl: https://github.com/fliaping/cranibox/releases/tag/v\S+",
        f"ReleaseNotesUrl: https://github.com/fliaping/cranibox/releases/tag/v{version}",
    )

    pkgbuild = PACKAGING / "aur/cranibox-bin/PKGBUILD"
    replace_once(pkgbuild, r"pkgver=[0-9][^\n]*", f"pkgver={version}")
    # Reset pkgrel on a new version.
    replace_once(pkgbuild, r"pkgrel=\d+", "pkgrel=1")
    text = pkgbuild.read_text()
    text, count = re.subn(
        r"sha256sums_x86_64=\('[0-9a-f]{64}'\n\s+'[0-9a-f]{64}'\)",
        f"sha256sums_x86_64=('{linux_x64}'\n                   '{sums}')" if sums else
        "sha256sums_x86_64=KEEP",
        text,
        count=1,
    )
    if not sums:
        raise ValueError("Need the SHA256SUMS file itself to pin the AUR source checksums.")
    if count != 1:
        raise ValueError("Could not update sha256sums_x86_64.")
    text, count = re.subn(
        r"sha256sums_aarch64=\('[0-9a-f]{64}'\n\s+'[0-9a-f]{64}'\)",
        f"sha256sums_aarch64=('{linux_arm}'\n                    '{sums}')",
        text,
        count=1,
    )
    if count != 1:
        raise SystemExit("Could not update sha256sums_aarch64.")
    pkgbuild.write_text(text)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True, help="Release tag, for example v1.2.1")
    parser.add_argument("--checksums", type=Path, help="Local SHA256SUMS file; downloaded from the tag if omitted.")
    args = parser.parse_args()
    try:
        bump(args.tag, args.checksums)
    except (OSError, ValueError) as error:
        parser.exit(1, f"bump-packaging: {error}\n")


if __name__ == "__main__":
    main()
