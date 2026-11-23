#!/usr/bin/env python3
"""Unpack the reviewed verification bundle, prepare SDK, and capture the app.

Run on the explicitly selected Apple Silicon Mac, after transferring
/tmp/cranibox-android-verification.tar.gz. Only creates verification files and
the dedicated cranibox-verify-api35 AVD. Leaves the emulator open for inspection.
"""
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import tarfile

APK_SHA256 = "fac0dbd53e8519bd87daa448ed2f179d5e95452bcc317795f8ac633a3d64be19"


def main():
    if platform.system() != "Darwin" or platform.machine() != "arm64":
        raise RuntimeError("Requires Apple Silicon macOS")
    root = Path.home() / "Library/Android/cranibox-verification"
    inputs = root / "inputs"
    inputs.mkdir(parents=True, exist_ok=True)
    names = {"cranibox-android-verification/" + name for name in (
        "prepare-emulator-macos.py", "verify-emulator.py", "cranibox.apk")}
    with tarfile.open("/tmp/cranibox-android-verification.tar.gz") as bundle:
        for member in bundle.getmembers():
            if member.name not in names or not member.isfile():
                raise RuntimeError(f"Unexpected archive entry: {member.name}")
            source = bundle.extractfile(member)
            if source is None:
                raise RuntimeError(f"Missing file: {member.name}")
            (inputs / Path(member.name).name).write_bytes(source.read())
    apk = inputs / "cranibox.apk"
    if hashlib.sha256(apk.read_bytes()).hexdigest() != APK_SHA256:
        raise RuntimeError("APK checksum mismatch")
    subprocess.run(["/usr/bin/python3", str(inputs / "prepare-emulator-macos.py")],
                   check=True)
    installed = json.loads((root / "installation.json").read_text())
    env = dict(os.environ, JAVA_HOME=installed["java_home"], ANDROID_HOME=installed["sdk"])
    evidence = root / "evidence"
    try:
        subprocess.run(["/usr/bin/python3", str(inputs / "verify-emulator.py"),
                        "--apk", str(apk), "--sdk", installed["sdk"],
                        "--output", str(evidence)], env=env, check=True)
    finally:
        if evidence.exists():
            with tarfile.open("/tmp/cranibox-android-emulator-evidence.tar.gz", "w:gz") as bundle:
                bundle.add(evidence, arcname="evidence")


if __name__ == "__main__":
    main()
