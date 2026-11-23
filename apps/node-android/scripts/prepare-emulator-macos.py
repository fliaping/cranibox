#!/usr/bin/env python3
"""Install a dedicated Android verification SDK and JDK in the user's Library.

Downloads pinned official packages, checks their published hashes, and installs
only emulator, platform-tools and the API 35 ARM64 Google APIs image. Does not
use sudo, change shell profiles, or replace an existing SDK/JDK.
"""
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import tarfile
import zipfile

TOOLS_URL = "https://dl.google.com/android/repository/commandlinetools-mac-12700392_latest.zip"
TOOLS_SHA1 = "625e146cf61d6b4fbdfdd9ad34105af23b0486c2"
JDK_URL = ("https://github.com/adoptium/temurin17-binaries/releases/download/"
           "jdk-17.0.21%2B9/OpenJDK17U-jdk_aarch64_mac_hotspot_17.0.21_9.tar.gz")
JDK_SHA256 = "78b08c7f5a0598242d5719c022e19479cb4a3aa45c37c19726a757a469ed1c15"


def download(url, destination, algorithm, expected):
    if not destination.exists():
        part = destination.with_suffix(destination.suffix + ".part")
        subprocess.run(["/usr/bin/curl", "--fail", "--location", "--retry", "3",
                        "--connect-timeout", "20", "--max-time", "900",
                        "--output", str(part), url], check=True)
        part.replace(destination)
    digest = hashlib.new(algorithm)
    with destination.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    if digest.hexdigest() != expected:
        raise RuntimeError(f"Checksum mismatch: {destination}")


def main():
    if platform.system() != "Darwin" or platform.machine() != "arm64":
        raise RuntimeError("This installer requires Apple Silicon macOS")
    root = Path.home() / "Library/Android/cranibox-verification"
    sdk = root / "sdk"
    cache = root / "downloads"
    cache.mkdir(parents=True, exist_ok=True)
    archive = cache / "jdk17.tar.gz"
    download(JDK_URL, archive, "sha256", JDK_SHA256)
    jdks = root / "jdks"
    java_home = jdks / "jdk-17.0.21+9/Contents/Home"
    if not (java_home / "bin/java").is_file():
        jdks.mkdir(exist_ok=True)
        # The archive is pinned to the official published SHA256 above.
        with tarfile.open(archive) as bundle:
            bundle.extractall(jdks)
    tools = cache / "cmdline-tools.zip"
    download(TOOLS_URL, tools, "sha1", TOOLS_SHA1)
    manager = sdk / "cmdline-tools/latest/bin/sdkmanager"
    if not manager.is_file():
        stage = root / "cmdline-tools-stage"
        stage.mkdir(exist_ok=True)
        with zipfile.ZipFile(tools) as bundle:
            bundle.extractall(stage)
        manager.parent.parent.parent.mkdir(parents=True, exist_ok=True)
        (stage / "cmdline-tools").rename(manager.parent.parent)
        for executable in manager.parent.iterdir():
            executable.chmod(executable.stat().st_mode | 0o111)
    env = dict(os.environ, JAVA_HOME=str(java_home), ANDROID_HOME=str(sdk))
    command = [str(manager), f"--sdk_root={sdk}"]
    print(f"Installing verification SDK in {sdk}", flush=True)
    # Keep verbose license/progress output on disk so remote job output retains
    # the final emulator results instead of truncating during SDK setup.
    setup_log = root / "sdk-install.log"
    with setup_log.open("w") as log:
        try:
            subprocess.run(command + ["--licenses"], env=env, input="y\n" * 100,
                           text=True, check=True, timeout=300, stdout=log, stderr=log)
            subprocess.run(command + ["platform-tools", "emulator",
                                      "system-images;android-35;google_apis;arm64-v8a"],
                           env=env, input="y\n" * 100, text=True, check=True,
                           timeout=1800, stdout=log, stderr=log)
        except (subprocess.SubprocessError, OSError):
            log.flush()
            print(setup_log.read_text(errors="replace")[-6000:], flush=True)
            raise
    report = {"sdk": str(sdk), "java_home": str(java_home),
              "image": "system-images;android-35;google_apis;arm64-v8a"}
    (root / "installation.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2), flush=True)


if __name__ == "__main__":
    main()
