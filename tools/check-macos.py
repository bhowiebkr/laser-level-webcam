#!/usr/bin/env python3
"""Opt-in hardware checks of the macOS CLI. Uses only Python's standard library.

Build `cargo build -p laser`, close other camera applications, then run:
  python3 tools/check-macos.py --binary target/debug/laser --camera 'Camera name'

Repeat --camera for multiple devices. --all-modes checks every advertised mode.
Frames stay in a temporary directory and are deleted; stdout contains only test results.
"""

import argparse
import json
import subprocess
import sys
import tempfile
from pathlib import Path


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def run(binary, *args):
    result = subprocess.run([binary, "camera", *args], capture_output=True, text=True, timeout=90)
    require(result.returncode == 0, f"camera {args}: {result.stderr.strip()}")
    return json.loads(result.stdout)


def verify_capture(result, frames):
    grabbed = result["grabbed"]
    require(grabbed["frames"] == frames, "wrong frame count")
    require(grabbed["fps"] > 0, "frame clock did not advance")
    # The CLI's fps includes missing frame slots. Also report the delivered rate.
    return {**grabbed, "delivered_fps": grabbed["fps"] * frames / (frames + grabbed["lost"])}


def check(binary, camera, all_modes):
    initial = run(binary, camera, "--frames", "120")
    report = {"camera": camera, "default_mode": initial["mode"],
              "capture": verify_capture(initial, 120), "controls": initial["controls"], "modes": []}
    controls = initial["controls"]
    restore = [arg for c in controls for arg in ("--set", f"{c['id']}={'auto' if c['auto'] else 0}")]
    try:
        if controls:
            auto = [arg for c in controls for arg in ("--set", f"{c['id']}=auto")]
            result = run(binary, camera, *auto, "--frames", "30")
            require(all(a["held"] and a["now"]["auto"] for a in result["applied"]), "auto mode not held")
            locked = run(binary, camera, "--lock", "--frames", "60")
            require(len(locked["applied"]) == len(controls), "lock omitted a reported control")
            require(all(a["held"] and a["now"]["auto"] is False for a in locked["applied"]), "lock not held")
            manual = [arg for c in controls for arg in ("--set", f"{c['id']}=0")]
            reopened = run(binary, camera, *manual, "--frames", "30")
            require(all(a["held"] for a in reopened["applied"]), "lock not reapplied on reopen")
            refused = run(binary, camera, "--set", f"{controls[0]['id']}=1")
            require(refused["applied"][0]["held"] is False, "unsupported numeric adjustment reported success")
            report["auto_lock_reopen_numeric_refusal"] = "passed"
            report["locked_capture"] = verify_capture(locked, 60)
        else:
            report["auto_lock_reopen_numeric_refusal"] = "unavailable: device exposes no auto/lock controls"

        modes = initial["modes"]
        if not all_modes:
            # Two common sizes plus the default's slowest and fastest rates.
            chosen = [initial["mode"]]
            for size in [(640, 480), (1280, 720), (initial["mode"]["width"], initial["mode"]["height"])]:
                available = [m for m in modes if (m["width"], m["height"]) == size]
                if available:
                    chosen.extend([min(available, key=lambda m: m["fps"]), max(available, key=lambda m: m["fps"])])
            modes = []
            for mode in chosen:
                if mode not in modes:
                    modes.append(mode)
        with tempfile.TemporaryDirectory(prefix="laser-macos-check-") as directory:
            frame = Path(directory) / "frame.pgm"
            for mode in modes:
                spec = f"{mode['width']}x{mode['height']}@{mode['fps']}:nv12"
                print(f"Checking {camera}: {spec}", file=sys.stderr, flush=True)
                result = run(binary, camera, "--mode", spec, "--frames", "12", "--save", str(frame))
                actual = result["mode"]
                require((actual["width"], actual["height"]) == (mode["width"], mode["height"]), "wrong mode size")
                require(abs(actual["fps"] - mode["fps"]) < 0.5, "wrong requested frame rate")
                timing = verify_capture(result, 12)
                require(timing["delivered_fps"] <= mode["fps"] * 1.15,
                        f"requested {mode['fps']} fps but received {timing['delivered_fps']}; output rate was not applied")
                # PGM output verifies that the luma buffer has exactly the advertised dimensions.
                with frame.open("rb") as stream:
                    require(stream.readline().strip() == b"P5", "wrong PGM encoding")
                    dimensions = tuple(map(int, stream.readline().split()))
                    require(dimensions == (mode["width"], mode["height"]), "wrong delivered dimensions")
                    require(stream.readline().strip() == b"255", "wrong luma depth")
                    pixels = stream.read()
                require(len(pixels) == mode["width"] * mode["height"], "truncated luma buffer")
                report["modes"].append({"mode": actual, "capture": timing,
                                        "luma_min": min(pixels), "luma_max": max(pixels)})
    finally:
        if restore:
            run(binary, camera, *restore)
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/debug/laser")
    parser.add_argument("--camera", action="append", required=True)
    parser.add_argument("--all-modes", action="store_true")
    args = parser.parse_args()
    names = run(args.binary, "--list")
    for camera in args.camera:
        require(camera != "Synthetic camera" and camera in names, f"real camera not found: {camera}")
        print(json.dumps(check(args.binary, camera, args.all_modes), indent=2), flush=True)


if __name__ == "__main__":
    main()
