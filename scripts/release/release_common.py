"""Shared checks for publishing already-built Cranibox release assets (stdlib only)."""
import hashlib
import json
from pathlib import Path
import re
import tomllib

ROOT = Path(__file__).resolve().parents[2]
PLATFORMS = {
    "darwin-arm64": ("cranibox-macos-arm64", "darwin", "arm64"),
    "linux-x64-gnu": ("cranibox-linux-x86_64", "linux", "x64"),
    "win32-x64": ("cranibox-windows-x86_64.exe", "win32", "x64"),
}
VERSION_RE = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?")


def validate_version(version):
    if not VERSION_RE.fullmatch(version):
        raise ValueError("Expected a semver like 1.1.0 or 1.2.0-beta.1 (no build metadata).")
    if "-" in version:
        for part in version.split("-", 1)[1].split("."):
            if part.isdigit() and len(part) > 1 and part.startswith("0"):
                raise ValueError("Numeric prerelease identifiers cannot have leading zeros.")
    return version


def version_for_tag(tag):
    if not tag.startswith("v"):
        raise ValueError("Expected a release tag like v1.1.0 or v1.2.0-beta.1 (no build metadata).")
    return validate_version(tag[1:])


def check_versions(tag, root=ROOT):
    version = version_for_tag(tag)
    files = ["crates/cranibox/Cargo.toml", "apps/node-desktop/src-tauri/tauri.conf.json",
             "apps/node-desktop/package.json", "npm/cli/package.json"]
    for name in files:
        text = (root / name).read_text()
        actual = tomllib.loads(text)["package"]["version"] if name.endswith("toml") else json.loads(text)["version"]
        if actual != version:
            raise ValueError(f"{name}: version {actual!r} does not match {tag}.")
    return version


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def verified_assets(directory):
    """Reject unlisted, missing, unsafe and changed assets before any publication."""
    directory = Path(directory)
    checksums = directory / "SHA256SUMS"
    expected = {}
    for line in checksums.read_text().splitlines():
        match = re.fullmatch(r"([a-fA-F0-9]{64}) [ *](.+)", line)
        if not match:
            raise ValueError("Invalid SHA256SUMS entry.")
        digest, name = match.groups()
        if name in (".", "..", "SHA256SUMS") or "/" in name or "\\" in name or name.startswith(".") or name in expected:
            raise ValueError(f"Unsafe or duplicate release filename: {name!r}.")
        expected[name] = digest.lower()
    if not expected:
        raise ValueError("SHA256SUMS must list at least one asset.")
    entries = list(directory.iterdir())
    if any(p.is_symlink() or not p.is_file() for p in entries):
        raise ValueError("Release assets must be regular files, without symlinks or directories.")
    if {p.name for p in entries} != set(expected) | {"SHA256SUMS"}:
        raise ValueError("Release files do not match SHA256SUMS.")
    for name, digest in expected.items():
        if sha256(directory / name) != digest:
            raise ValueError(f"Checksum mismatch: {name}.")
    return [directory / name for name in sorted(expected)] + [checksums]


def check_binary(path, target):
    """Check actual executable architecture instead of trusting runner labels."""
    with path.open("rb") as stream:
        header = stream.read(64)
        valid = False
        if target == "linux-x64-gnu":
            valid = header[:6] == b"\x7fELF\x02\x01" and int.from_bytes(header[18:20], "little") == 62
        elif target == "darwin-arm64":
            valid = header[:4] == b"\xcf\xfa\xed\xfe" and int.from_bytes(header[4:8], "little") == 0x0100000C
        elif target == "win32-x64" and len(header) == 64 and header[:2] == b"MZ":
            stream.seek(int.from_bytes(header[60:64], "little"))
            valid = stream.read(6) == b"PE\0\0\x64\x86"
    if not valid:
        raise ValueError(f"{path.name}: executable format/architecture does not match {target}.")
