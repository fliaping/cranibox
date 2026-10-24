#!/usr/bin/env python3
"""Pack and publish native npm packages before their CLI entry point."""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess
from urllib.error import HTTPError, URLError
from urllib.parse import quote
from urllib.request import urlopen

from npm_stage import NPM, REGISTRY, list_stages


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


def stage_summary(outcomes, dry_run, package_name, completed):
    lines = ["npm stage dry run: nothing staged." if dry_run else "npm stage publication summary:"]
    if not completed:
        lines.append("Publication stopped before completion. Resolve the reported error before approving the package set.")
    pending = False
    for name, version, status, stages in outcomes:
        lines.append(f"- {name}@{version}: {status}")
        if status in ("staged", "already staged and verified"):
            pending = True
            lines.append(f"  Review / approve with 2FA: https://www.npmjs.com/package/{name}")
            for stage in stages:
                lines.append(f"  `npm stage approve {shlex.quote(stage['id'])} --otp <OTP>`")
    if pending:
        lines.append("Approve all four packages in one go with a fresh, short-lived 2FA OTP:")
        lines.append(f"`python scripts/release/approve-staged.py --package-name {shlex.quote(package_name)} --approve`")
        lines.append("Then configure Trusted Publishing for each package and switch CRANIBOX_NPM_PUBLISH_MODE to direct.")
    elif not dry_run and completed:
        lines.append("No new stages need approval from this invocation.")
    summary = "\n".join(lines) + "\n"
    print(summary, flush=True)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as stream:
            stream.write(summary)


def publish(directory, dry_run=False, dist_tag=None, mode="direct"):
    if mode not in ("direct", "stage"):
        raise ValueError("Publish mode must be direct or stage.")
    directory = directory.resolve()
    plan = json.loads((directory / "publish-plan.json").read_text())
    if not isinstance(plan, list) or not plan:
        raise ValueError("Publish plan must be a nonempty list of package directories.")
    for entry in plan:
        if not isinstance(entry, str) or Path(entry).name != entry or entry in (".", ".."):
            raise ValueError("Invalid package directory in publish plan.")
    package_name = json.loads((directory / plan[-1] / "package.json").read_text())["name"]
    outcomes = []
    completed = False
    try:
        publish_packages(directory, plan, dry_run, dist_tag, mode, outcomes)
        completed = True
    finally:
        if mode == "stage":
            stage_summary(outcomes, dry_run, package_name, completed)


def publish_packages(directory, plan, dry_run, dist_tag, mode, outcomes):
    npm = NPM
    for entry in plan:
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
        tag = dist_tag or ("next" if "-" in version else "latest")
        if not dry_run:
            existing = existing_integrity(name, version)
            if existing:
                if existing != integrity:
                    raise ValueError(f"{name}@{version} already exists with different contents. Publish a new version.")
                print(f"Already published and verified {name}@{version}", flush=True)
                outcomes.append((name, version, "already published and verified", []))
                continue
            if mode == "stage":
                staged = [stage for stage in list_stages(name, require_integrity=True) if stage["version"] == version]
                for stage in staged:
                    if stage["tag"] != tag:
                        raise ValueError(f"{name}@{version} is already staged with tag {stage['tag']!r}, requested {tag!r}. "
                                         "Staged tags cannot be changed; reject it and re-stage with a new version.")
                    if stage["integrity"] != integrity:
                        raise ValueError(f"{name}@{version} is already staged with different contents. Publish a new version.")
                if staged:
                    print(f"Already staged and verified {name}@{version} (tag {tag}); skipping.", flush=True)
                    outcomes.append((name, version, "already staged and verified", staged))
                    continue
        command = ([npm, "stage", "publish", str(package)] if mode == "stage"
                   else [npm, "publish", str(archive)])
        command += ["--access", "public", "--ignore-scripts", "--registry", REGISTRY, "--tag", tag]
        command += ["--dry-run", "--offline"] if dry_run else ["--provenance"]
        subprocess.run(command, check=True)
        outcomes.append((name, version, "dry run" if dry_run else "staged" if mode == "stage" else "published", []))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--mode", choices=("direct", "stage"), default="direct")
    parser.add_argument("--dist-tag", help="npm channel (default: latest for stable, next for prerelease). Use dev for CI builds.")
    args = parser.parse_args()
    try:
        publish(args.directory, args.dry_run, args.dist_tag, args.mode)
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"publish-npm: {error}\n")


if __name__ == "__main__":
    main()
