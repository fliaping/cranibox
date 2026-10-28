#!/usr/bin/env python3
"""List Cranibox stages; approve or reject all listed versions.

Without --otp, npm confirms each one in the browser (WebAuthn / passkey).
"""
import argparse
import os
import subprocess

from npm_stage import NPM, REGISTRY, list_stages, package_names


def manage(package_name, action=None, otp=None):
    stages = [stage for name in package_names(package_name) for stage in list_stages(name)]
    for stage in stages:
        print(f"{stage['name']}@{stage['version']} tag={stage['tag']} stage={stage['id']}", flush=True)
    if not stages:
        print("No staged Cranibox versions found.")
        return
    if action is None:
        print("Use --approve or --reject to act on ALL listed versions of these packages.")
        return
    print(f"Will {action} all {len(stages)} listed stages.", flush=True)
    if otp is not None and not otp.strip():
        raise ValueError("The OTP is empty.")
    for stage in stages:
        command = [NPM, "stage", action, stage["id"], "--registry", REGISTRY]
        if otp:
            # Do not echo the command, child output, or CalledProcessError: any of
            # those could contain the OTP. Report only the stage ID and exit status.
            result = subprocess.run(command + ["--otp", otp], capture_output=True, text=True)
        else:
            # npm prints a login URL; confirm it in the browser with the security key.
            print(f"{stage['name']}@{stage['version']}: confirm in the browser when npm asks.", flush=True)
            result = subprocess.run(command + ["--auth-type", "web"])
        if result.returncode:
            raise RuntimeError(f"npm stage {action} failed for {stage['id']} (exit {result.returncode}); "
                               "rerun to list the remaining stages.")
        print(f"Completed {action}: {stage['name']}@{stage['version']} ({stage['id']})", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package-name", default=os.environ.get("CRANIBOX_NPM_PACKAGE") or "@fliaping/cranibox")
    actions = parser.add_mutually_exclusive_group()
    actions.add_argument("--approve", action="store_true")
    actions.add_argument("--reject", action="store_true")
    parser.add_argument("--otp", help="Authenticator-app OTP; omit to confirm in the browser (WebAuthn).")
    args = parser.parse_args()
    try:
        manage(args.package_name, "approve" if args.approve else "reject" if args.reject else None, args.otp)
    except (OSError, ValueError, RuntimeError, EOFError) as error:
        parser.exit(1, f"approve-staged: {error}\n")


if __name__ == "__main__":
    main()
