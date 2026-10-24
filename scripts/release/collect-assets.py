#!/usr/bin/env python3
"""Collect standalone CLI artifacts, licenses and optional installers with SHA256SUMS."""
import argparse
from pathlib import Path
import shutil

from release_common import ROOT, PLATFORMS, sha256


def collect(artifacts, output, installers=False, root=ROOT):
    artifacts, output = Path(artifacts), Path(output)
    if output.exists() and any(output.iterdir()):
        raise ValueError(f"Output directory is not empty: {output}.")
    platforms = {"darwin": "macos", "linux": "linux", "win32": "windows"}
    files = []
    for filename, os_name, _ in PLATFORMS.values():
        files.append((artifacts / f"cranibox-cli-{platforms[os_name]}" / filename, os_name != "win32"))
    files += [(root / "LICENSE", False)]
    files += [(notice, False) for notice in sorted((root / "docs/licenses").glob("*.txt"))]
    if installers:
        for directory in sorted(artifacts.glob("cranibox-desktop-*")):
            files += [(file, False) for file in sorted(directory.rglob("*"))
                      if file.is_file() and file.suffix in (".dmg", ".exe", ".deb", ".AppImage")]
    names = set()
    for file, _ in files:
        if not file.is_file():
            raise ValueError(f"Missing release asset: {file}.")
        if file.name in names:
            raise ValueError(f"Duplicate release filename: {file.name}.")
        names.add(file.name)
    output.mkdir(parents=True, exist_ok=True)
    for file, executable in files:
        destination = output / file.name
        shutil.copyfile(file, destination)
        if executable:
            destination.chmod(0o755)
    (output / "SHA256SUMS").write_text("".join(
        f"{sha256(output / name)}  {name}\n" for name in sorted(names)))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifacts", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--installers", action="store_true")
    args = parser.parse_args()
    try:
        collect(args.artifacts, args.output, args.installers)
    except (OSError, ValueError) as error:
        parser.exit(1, f"collect-assets: {error}\n")


if __name__ == "__main__":
    main()
