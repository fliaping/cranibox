#!/usr/bin/env python3
"""Pack and publish native npm packages before their CLI entry point."""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import subprocess
from urllib.error import HTTPError, URLError
from urllib.parse import quote
from urllib.request import urlopen

REGISTRY = "https://registry.npmjs.org"


def existing_integrity(name, version):
    try:
        with urlopen(f"{REGISTRY}/{quote(name, safe='@')}/{quote(version, safe='')}", timeout=60) as response:
            return json.load(response)["dist"]["integrity"]
    except HTTPError as error:
        if error.code == 404:
            return None
        raise RuntimeError(f"npm registry returned HTTP {error.code} for {name}.") from None
    except URLError:
        raise RuntimeError(f"Could not reach npm registry for {name}.") from None


def publish(directory, dry_run=False):
    directory = directory.resolve()
    plan = json.loads((directory / "publish-plan.json").read_text())
    npm = "npm.cmd" if __import__("os").name == "nt" else "npm"
    for entry in plan:
        if Path(entry).name != entry or entry in (".", ".."):
            raise ValueError("Invalid package directory in publish plan.")
        package = directory / entry
        manifest = json.loads((package / "package.json").read_text())
        # Actions artifacts do not preserve executable bits on unpacked files.
        for executable in (package / "bin").iterdir():
            executable.chmod(0o755)
        result = subprocess.run([npm, "pack", "--json", "--ignore-scripts", "--pack-destination", str(directory)],
                                cwd=package, check=True, capture_output=True, text=True)
        archive = directory / json.loads(result.stdout)[0]["filename"]
        with archive.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha512").digest()
        integrity = "sha512-" + base64.b64encode(digest).decode()
        name, version = manifest["name"], manifest["version"]
        if not dry_run:
            existing = existing_integrity(name, version)
            if existing:
                if existing != integrity:
                    raise ValueError(f"{name}@{version} already exists with different contents. Publish a new version.")
                print(f"Already published and verified {name}@{version}", flush=True)
                continue
        command = [npm, "publish", str(archive), "--access", "public", "--ignore-scripts",
                   "--registry", REGISTRY, "--tag", "next" if "-" in version else "latest"]
        command += ["--dry-run", "--offline"] if dry_run else ["--provenance"]
        subprocess.run(command, check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    try:
        publish(args.directory, args.dry_run)
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"publish-npm: {error}\n")


if __name__ == "__main__":
    main()
