#!/usr/bin/env python3
"""Exercise an already-paired debug app on the dedicated API 35 verification AVD.

Reads the app's bridge token through run-as, keeping it out of output/artifacts.
Uses Google Clock on the isolated display. Saves screenshots for visual review.
Run after verify-emulator.py, then connect phone control in the app first.
"""
import argparse
import base64
import json
from pathlib import Path
import re
import subprocess
import time
import urllib.error
import urllib.request
import xml.etree.ElementTree as ET


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--adb", required=True, type=Path)
    parser.add_argument("--serial", default="emulator-5554")
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)

    def adb(*command):
        return subprocess.check_output([str(args.adb), "-s", args.serial, *command], timeout=45)

    assert args.serial.startswith("emulator-")
    assert adb("emu", "avd", "name").decode().splitlines()[0] == "cranibox-verify-api35"
    package = "com.cranibox.node"
    config = json.loads(adb("shell", "run-as", package, "cat", "files/.config/cranibox/peers.json"))
    token = adb("shell", "run-as", package, "cat", "files/bridge.token").decode().strip()
    port = int(adb("forward", "tcp:0", "tcp:" + config["computer"]["address"].rsplit(":", 1)[1]))
    results = []

    def action(name, expected=200, authenticated=True, **params):
        headers = {"Content-Type": "application/json", "X-Agent-Session": "emulator-verification"}
        if authenticated:
            headers["Authorization"] = "Bearer " + token
        request = urllib.request.Request(f"http://127.0.0.1:{port}/v1/action",
                                         data=json.dumps(dict(action=name, **params)).encode(), headers=headers)
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                status, body = response.status, json.load(response)
        except urllib.error.HTTPError as error:
            status, body = error.code, json.load(error)
        saved = dict(body)
        if "data" in saved:
            image = base64.b64decode(saved.pop("data"))
            assert image.startswith(b"\x89PNG\r\n\x1a\n")
            filename = f"{len(results):02d}-{name}.png"
            (out / filename).write_bytes(image)
            saved["screenshot"] = filename
        results.append(dict(action=name, http=status, body=saved))
        print(f"{name}: HTTP {status}", flush=True)
        assert status == expected, (name, status, saved)
        return body

    def hierarchy():
        adb("shell", "uiautomator", "dump", "/sdcard/cranibox-phone-check.xml")
        return ET.fromstring(adb("shell", "cat", "/sdcard/cranibox-phone-check.xml"))

    def tap(label):
        matches = [n for n in hierarchy().iter("node") if n.get("text") == label]
        assert len(matches) == 1, (label, len(matches))
        x1, y1, x2, y2 = map(int, re.findall(r"\d+", matches[0].get("bounds")))
        adb("shell", "input", "tap", str((x1 + x2) // 2), str((y1 + y2) // 2))
        time.sleep(1)

    def main_screen(name):
        tree = hierarchy()
        assert any(n.get("text") == "节点运行中" for n in tree.iter("node")), "main screen changed"
        (out / f"{name}.xml").write_bytes(ET.tostring(tree))
        (out / f"{name}.png").write_bytes(adb("exec-out", "screencap", "-p"))

    passed = False
    try:
        adb("shell", "am", "start", "-W", "-f", "0x14000000", "-n",
            f"{package}/com.cranibox.android.MainActivity")
        time.sleep(2)
        tap("概览")
        status = action("status")
        assert status["state"] == "ready" and status["display_id"] > 0
        action("status", expected=401, authenticated=False)
        action("app_list")
        main_screen("main-before")
        assert action("open_app", package=package)["result"] == "in_use_on_main_screen"
        assert action("open_app", package="com.google.android.deskclock")["result"] == "started"
        time.sleep(2)
        windows = action("window_list")["windows"]
        clock = next(w for w in windows if w["package"] == "com.google.android.deskclock")
        action("window_focus", window_id=clock["id"])
        time.sleep(1)
        main_screen("main-after-focus")
        action("screenshot")
        # API 35 Google Clock: select Clock tab, then Add city.
        width, height = status["resolution"]["width"], status["resolution"]["height"]
        action("left_click", coordinate=[int(width * .3), int(height * .95)])
        action("left_click", coordinate=[width // 2, int(height * .842)])
        time.sleep(1)
        action("type", text="London")
        time.sleep(2)
        action("screenshot")
        action("key", text="ctrl+a")
        action("type", text="北京")
        time.sleep(2)
        action("screenshot")
        action("zoom", region=[0, 0, width, 300])
        action("scroll", scroll_direction="down", scroll_amount=2)
        main_screen("main-after-input")
        tap("立即停止 Agent")
        assert action("status")["paused"]
        action("left_click", expected=423, coordinate=[100, 100])
        action("type", expected=423, text="must not appear")
        action("screenshot")
        (out / "main-paused.png").write_bytes(adb("exec-out", "screencap", "-p"))
        tap("在本机恢复")
        assert action("status")["state"] == "ready"
        action("key", text="back")
        passed = True
    finally:
        (out / "result.json").write_text(json.dumps({"passed": passed, "checks": results,
            "note": "Inspect saved screenshots to verify text rendering and UI; this is an emulator check."},
            ensure_ascii=False, indent=2) + "\n")
        adb("forward", "--remove", f"tcp:{port}")


if __name__ == "__main__":
    main()
