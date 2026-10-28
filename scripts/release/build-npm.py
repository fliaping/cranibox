#!/usr/bin/env python3
"""Package the existing native release files for npm, without install scripts."""
import argparse
import json
from pathlib import Path, PurePosixPath
import re
import shutil
import tarfile

from release_common import ROOT, PLATFORMS, check_binary, check_versions, validate_version, verified_assets


# The desktop app for each target, as the desktop CI job names it, and its place in the package.
APPS = {
    "darwin-arm64": ("cranibox-app-darwin-arm64.tar.gz", "Cranibox.app"),
    "linux-x64-gnu": ("cranibox-app-linux-x64-gnu.AppImage", "Cranibox.AppImage"),
    "win32-x64": ("cranibox-app-win32-x64-setup.exe", "cranibox-setup.exe"),
}


def unpack_bundle(archive, destination):
    """Extract a macOS app bundle: regular files and directories only, under one bundle root."""
    with tarfile.open(archive) as bundle:
        for member in bundle.getmembers():
            path = PurePosixPath(member.name)
            if path.is_absolute() or ".." in path.parts or path.parts[:1] != ("Cranibox.app",):
                raise ValueError(f"Unsafe path in {archive.name}: {member.name}.")
            if not (member.isfile() or member.isdir()):
                raise ValueError(f"{archive.name} may contain only files and directories: {member.name}.")
            target = destination.joinpath(*path.parts)
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            with bundle.extractfile(member) as source, target.open("wb") as stream:
                shutil.copyfileobj(source, stream)
            target.chmod(0o755 if member.mode & 0o111 else 0o644)
    if not (destination / "Cranibox.app/Contents/Info.plist").is_file():
        raise ValueError(f"{archive.name} is not a Cranibox.app bundle.")


def build(assets, output, tag=None, package_name="@fliaping/cranibox", root=ROOT, version=None, apps=None):
    if version is None and tag is None:
        raise ValueError("Provide --tag or --version.")
    version = validate_version(version) if version is not None else check_versions(tag, root)
    if not re.fullmatch(r"@[a-z0-9][a-z0-9._-]*/[a-z0-9][a-z0-9._-]*", package_name):
        raise ValueError("The npm package must be a scoped lowercase name, e.g. @fliaping/cranibox.")
    files = {p.name: p for p in verified_assets(assets)}
    for target, (filename, _, _) in PLATFORMS.items():
        if filename not in files:
            raise ValueError(f"Missing release asset: {filename}.")
        check_binary(files[filename], target)
    if apps is not None:
        apps = Path(apps)
        for filename, _ in APPS.values():
            if not (apps / filename).is_file():
                raise ValueError(f"Missing desktop app: {filename}.")
    output = Path(output)
    if output.exists() and any(output.iterdir()):
        raise ValueError(f"Output directory is not empty: {output}.")
    output.mkdir(parents=True, exist_ok=True)
    template = json.loads((root / "npm/cli/package.json").read_text())
    template["name"] = package_name
    template["version"] = version
    template["optionalDependencies"] = {}
    template["craniboxPlatforms"] = {}
    plan = []

    def write_package(directory, manifest):
        directory.mkdir(parents=True)
        (directory / "package.json").write_text(json.dumps(manifest, indent=2) + "\n")
        shutil.copyfile(root / "LICENSE", directory / "LICENSE")
        shutil.copytree(root / "docs/licenses", directory / "licenses")
        readme = (root / "npm/cli/README.md").read_text().replace("@fliaping/cranibox", package_name)
        (directory / "README.md").write_text(readme)

    for target, (filename, os_name, cpu) in PLATFORMS.items():
        name = f"{package_name}-{target}"
        template["optionalDependencies"][name] = version
        template["craniboxPlatforms"][target] = name
        manifest = {key: value for key, value in template.items()
                    if key not in ("bin", "optionalDependencies", "craniboxPlatforms", "engines")}
        manifest.update(name=name, description=f"Cranibox native binary for {target}", os=[os_name], cpu=[cpu])
        if os_name == "linux":
            manifest["libc"] = ["glibc"]
        directory = output / f"cli-{target}"
        write_package(directory, manifest)
        (directory / "bin").mkdir()
        binary = directory / "bin" / ("cranibox.exe" if os_name == "win32" else "cranibox")
        shutil.copyfile(files[filename], binary)
        binary.chmod(0o755)
        plan.append(directory.name)
    directory = output / "cli"
    write_package(directory, template)
    shutil.copytree(root / "npm/cli/bin", directory / "bin")
    (directory / "bin/cranibox.cjs").chmod(0o755)
    plan.append(directory.name)  # Native packages must exist before publishing the entry point.
    # Last: a new desktop package name that fails to publish cannot hold back the runtime.
    for target, (filename, os_name, cpu) in PLATFORMS.items() if apps is not None else ():
        manifest = {key: value for key, value in template.items()
                    if key not in ("bin", "optionalDependencies", "craniboxPlatforms", "engines")}
        manifest.update(name=f"{package_name}-desktop-{target}", description=f"Cranibox desktop app for {target}",
                        os=[os_name], cpu=[cpu])
        if os_name == "linux":
            manifest["libc"] = ["glibc"]
        directory = output / f"desktop-{target}"
        write_package(directory, manifest)
        (directory / "app").mkdir()
        source, name = APPS[target]
        if name.endswith(".app"):
            unpack_bundle(apps / source, directory / "app")
        else:
            shutil.copyfile(apps / source, directory / "app" / name)
            (directory / "app" / name).chmod(0o755)
        plan.append(directory.name)
    (output / "publish-plan.json").write_text(json.dumps(plan, indent=2) + "\n")
    return plan


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--assets", type=Path, required=True)
    parser.add_argument("--output", type=Path, default=ROOT / "dist/npm")
    parser.add_argument("--tag")
    parser.add_argument("--version", help="Override all package versions without checking the tag/source versions.")
    parser.add_argument("--package-name", default="@fliaping/cranibox")
    parser.add_argument("--apps", type=Path, help="Also package the desktop apps from this directory.")
    args = parser.parse_args()
    try:
        for package in build(args.assets, args.output, args.tag, args.package_name, version=args.version, apps=args.apps):
            print(f"Prepared {package}")
    except (OSError, ValueError) as error:
        parser.exit(1, f"build-npm: {error}\n")


if __name__ == "__main__":
    main()
