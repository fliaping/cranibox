#!/usr/bin/env python3
"""Launch a dedicated ARM64 AVD and collect real Android screenshots and logs.

Uses an installed SDK/system image. Does not download tools, wipe data, stop other
emulators, or claim to test wireless pairing. Leaves its emulator open for review.
"""
import argparse
import json
import os
from pathlib import Path
import re
import socket
import subprocess
import sys
import time
import xml.etree.ElementTree as ET

PACKAGE = "com.cranibox.node"
ACTIVITY = "com.cranibox.android.MainActivity"
AVD = "cranibox-verify-api35"
IMAGE = "system-images;android-35;google_apis;arm64-v8a"


def run(args, **kwargs):
    result = subprocess.run([str(a) for a in args], check=False, timeout=60, **kwargs)
    if result.returncode:
        for output in (result.stdout, result.stderr):
            if output:
                if isinstance(output, bytes):
                    output = output.decode("utf-8", errors="replace")
                print(output[-6000:], file=sys.stderr)
        result.check_returncode()
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--apk", required=True, type=Path)
    parser.add_argument("--sdk", type=Path, default=Path(os.environ.get(
        "ANDROID_HOME", os.environ.get("ANDROID_SDK_ROOT", str(Path.home() / "Library/Android/sdk")))))
    parser.add_argument("--output", type=Path, default=Path("artifacts/emulator"))
    parser.add_argument("--connect-phone", action="store_true",
                        help="Reconnect already-paired wireless debugging and require the helper to be ready")
    args = parser.parse_args()
    if not args.apk.is_file():
        parser.error(f"APK does not exist: {args.apk}")
    sdk = args.sdk.expanduser().resolve()
    adb = sdk / "platform-tools/adb"
    emulator = sdk / "emulator/emulator"
    for tool in (adb, emulator):
        if not tool.is_file():
            parser.error(f"Missing SDK tool: {tool}")
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    installed = run([emulator, "-list-avds"], capture_output=True, text=True).stdout.splitlines()
    if AVD not in installed:
        avdmanager = sdk / "cmdline-tools/latest/bin/avdmanager"
        if not avdmanager.is_file() or not (sdk / "system-images/android-35/google_apis/arm64-v8a").is_dir():
            parser.error("Install cmdline-tools and the ARM64 API 35 Google APIs image first: "
                         f"sdkmanager '{IMAGE}'")
        run([avdmanager, "create", "avd", "-n", AVD, "-k", IMAGE, "-d", "pixel_7"],
            input="no\n", text=True, capture_output=True)

    # Reuse only our dedicated AVD; never pick an arbitrary connected phone.
    serial = None
    devices = run([adb, "devices"], capture_output=True, text=True).stdout.splitlines()[1:]
    for line in devices:
        parts = line.split()
        if len(parts) != 2 or parts[1] != "device" or not parts[0].startswith("emulator-"):
            continue
        name = run([adb, "-s", parts[0], "emu", "avd", "name"], capture_output=True, text=True).stdout
        if name.splitlines()[0] == AVD:
            serial = parts[0]
            break
    if serial is None:
        for port in range(5554, 5584, 2):
            try:
                with socket.socket() as first, socket.socket() as second:
                    first.bind(("127.0.0.1", port))
                    second.bind(("127.0.0.1", port + 1))
                break
            except OSError:
                continue
        else:
            raise RuntimeError("No free emulator port")
        with (out / "emulator.log").open("wb") as log:
            subprocess.Popen([str(emulator), "-avd", AVD, "-port", str(port),
                              "-no-snapshot", "-no-boot-anim"], stdout=log, stderr=log,
                             start_new_session=True)
        serial = f"emulator-{port}"
    print(f"Waiting for {AVD} ({serial})…", flush=True)

    def device(*command, **kwargs):
        return run([adb, "-s", serial, *command], **kwargs)

    deadline = time.monotonic() + 180
    while time.monotonic() < deadline:
        try:
            if device("shell", "getprop", "sys.boot_completed", capture_output=True, text=True).stdout.strip() == "1":
                break
        except subprocess.CalledProcessError:
            pass
        time.sleep(2)
    else:
        raise RuntimeError(f"Emulator did not boot; inspect {out / 'emulator.log'}")

    def hierarchy():
        device("shell", "uiautomator", "dump", "/sdcard/cranibox-verify.xml", capture_output=True)
        xml = device("shell", "cat", "/sdcard/cranibox-verify.xml", capture_output=True).stdout
        return xml, ET.fromstring(xml)

    def tap_text(label):
        _, root = hierarchy()
        matches = [n for n in root.iter("node") if n.get("text") == label]
        if len(matches) != 1:
            raise RuntimeError(f"Expected one visible control labelled {label!r}, found {len(matches)}")
        x1, y1, x2, y2 = map(int, re.findall(r"\d+", matches[0].get("bounds", "")))
        device("shell", "input", "tap", str((x1 + x2) // 2), str((y1 + y2) // 2))
        time.sleep(1)

    def capture(name):
        xml, _ = hierarchy()
        (out / f"{name}.xml").write_bytes(xml)
        with (out / f"{name}.png").open("wb") as image:
            device("exec-out", "screencap", "-p", stdout=image)
        print(f"Captured {name}.png", flush=True)

    def launch():
        # Settings opened by the app can sit above MainActivity in the same task.
        # Bring the actual app screen forward instead of resuming that Settings page.
        device("shell", "am", "start", "-W", "-f", "0x14000000", "-n",
               f"{PACKAGE}/{ACTIVITY}", capture_output=True)
        time.sleep(2)

    device("install", "-r", str(args.apk.resolve()), capture_output=True)
    device("shell", "pm", "grant", PACKAGE, "android.permission.POST_NOTIFICATIONS", capture_output=True)
    # adb shell joins argv into shell text; an unquoted space splits date's
    # format into two arguments on the device. Use a separator and replace it
    # locally to produce the format expected by logcat -T.
    since = device("shell", "date", "+%m-%dT%H:%M:%S.000",
                   capture_output=True, text=True).stdout.strip().replace("T", " ", 1)
    device("shell", "cmd", "uimode", "night", "no", capture_output=True)
    device("shell", "settings", "put", "system", "font_scale", "1.0", capture_output=True)
    launch()
    node_running = False
    phone_connected = False
    try:
        for _ in range(20):
            _, root = hierarchy()
            if any(n.get("text") == "节点运行中" for n in root.iter("node")):
                node_running = True
                break
            time.sleep(1)
        if args.connect_phone:
            tap_text("重新连接")
            for _ in range(20):
                _, root = hierarchy()
                if any(n.get("text") == "Agent 控制已就绪" for n in root.iter("node")):
                    phone_connected = True
                    break
                time.sleep(1)
            if not phone_connected:
                capture("phone-reconnect-failed")
                raise RuntimeError("Phone control did not reconnect; complete wireless pairing first")
        capture("01-overview-light")
        tap_text("手机")
        capture("02-phone-light")
        tap_text("设备")
        capture("03-devices-light")
        device("shell", "cmd", "uimode", "night", "yes", capture_output=True)
        # Activity normally recreates when the system theme changes.
        time.sleep(2)
        tap_text("概览")
        capture("04-overview-dark")
        tap_text("手机")
        capture("05-phone-dark")
        device("shell", "settings", "put", "system", "font_scale", "1.3", capture_output=True)
        time.sleep(2)
        capture("06-phone-large-text")
    finally:
        # Restore the dedicated emulator's display settings even after an assertion fails.
        device("shell", "settings", "put", "system", "font_scale", "1.0", capture_output=True)
        device("shell", "cmd", "uimode", "night", "no", capture_output=True)
        with (out / "logcat.txt").open("wb") as log:
            device("logcat", "-d", "-v", "threadtime", "-T", since, stdout=log)
    report = {"avd": AVD, "serial": serial, "apk": str(args.apk.resolve()),
              "node_running": node_running, "wireless_pairing_verified": False,
              "phone_control_connected": phone_connected,
              "note": "Screenshots require visual inspection. Wireless pairing and helper control require separate runtime checks."}
    (out / "result.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(f"Evidence: {out}\nEmulator left open for wireless pairing checks.")
    if not node_running:
        raise RuntimeError("Node never reached running state; inspect screenshots and logcat")


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, subprocess.SubprocessError) as error:
        print(f"Verification failed: {error}", file=sys.stderr)
        sys.exit(1)
