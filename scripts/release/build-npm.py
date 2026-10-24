#!/usr/bin/env python3
"""Package the existing native release files for npm, without install scripts."""
import argparse
import json
from pathlib import Path
import re
import shutil

from release_common import ROOT, PLATFORMS, check_binary, check_versions, verified_assets


def build(assets, output, tag, package_name="@fliaping/cranibox", root=ROOT):
    version = check_versions(tag, root)
    if not re.fullmatch(r"@[a-z0-9][a-z0-9._-]*/[a-z0-9][a-z0-9._-]*", package_name):
        raise ValueError("The npm package must be a scoped lowercase name, e.g. @fliaping/cranibox.")
    files = {p.name: p for p in verified_assets(assets)}
    for target, (filename, _, _) in PLATFORMS.items():
        if filename not in files:
            raise ValueError(f"Missing release asset: {filename}.")
        check_binary(files[filename], target)
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
    (output / "publish-plan.json").write_text(json.dumps(plan, indent=2) + "\n")
    return plan


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--assets", type=Path, required=True)
    parser.add_argument("--output", type=Path, default=ROOT / "dist/npm")
    parser.add_argument("--tag", required=True)
    parser.add_argument("--package-name", default="@fliaping/cranibox")
    args = parser.parse_args()
    try:
        for package in build(args.assets, args.output, args.tag, args.package_name):
            print(f"Prepared {package}")
    except (OSError, ValueError) as error:
        parser.exit(1, f"build-npm: {error}\n")


if __name__ == "__main__":
    main()
