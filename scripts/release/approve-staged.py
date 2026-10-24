#!/usr/bin/env python3
"""List Cranibox stages; approve or reject all listed versions with one short-lived OTP."""
import argparse
import getpass
import os
import subprocess
import warnings

from npm_stage import NPM, REGISTRY, list_stages, package_names


def manage(package_name, action=None, otp=None):
    stages = [stage for name in package_names(package_name) for stage in list_stages(name)]
    for stage in stages:
        print(f"{stage['name']}@{stage['version']} tag={stage['tag']} stage={stage['id']}", flush=True)
    if not stages:
        print("No staged Cranibox versions found.")
        return
    if action is None:
        print("Use --approve or --reject to act on ALL listed versions of these four packages.")
        return
    print(f"Will {action} all {len(stages)} listed stages in one go: the 2FA OTP is short-lived.", flush=True)
    if not otp:
        # getpass otherwise falls back to an echoing input stream without a TTY.
        with warnings.catch_warnings():
            warnings.simplefilter("error", getpass.GetPassWarning)
            try:
                otp = getpass.getpass("npm 2FA OTP (hidden): ")
            except getpass.GetPassWarning:
                raise ValueError("A terminal is required for hidden OTP input.") from None
    if not otp or not otp.strip():
        raise ValueError("An OTP is required.")
    for stage in stages:
        # Do not echo the command, child output, or CalledProcessError: any of
        # those could contain the OTP. Report only the stage ID and exit status.
        result = subprocess.run([NPM, "stage", action, stage["id"], "--otp", otp,
                                 "--registry", REGISTRY], capture_output=True, text=True)
        if result.returncode:
            raise RuntimeError(f"npm stage {action} failed for {stage['id']} (exit {result.returncode}); "
                               "rerun to list remaining stages and use a fresh OTP.")
        print(f"Completed {action}: {stage['name']}@{stage['version']} ({stage['id']})", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package-name", default=os.environ.get("CRANIBOX_NPM_PACKAGE") or "@fliaping/cranibox")
    actions = parser.add_mutually_exclusive_group()
    actions.add_argument("--approve", action="store_true")
    actions.add_argument("--reject", action="store_true")
    parser.add_argument("--otp", help="2FA OTP; omit to prompt once without echo.")
    args = parser.parse_args()
    try:
        manage(args.package_name, "approve" if args.approve else "reject" if args.reject else None, args.otp)
    except (OSError, ValueError, RuntimeError, EOFError) as error:
        parser.exit(1, f"approve-staged: {error}\n")


if __name__ == "__main__":
    main()
