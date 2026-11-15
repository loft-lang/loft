#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The Android guards: each `tests/android/*.loft` built as an app through the C backend
(`loft --native-c --native-android`), run on a device or emulator, and its whole stdout and
exit status compared with `<name>.expected` (status 0).

    scripts/android_guards.py [--lib DIR]... [name...]   # every guard, or the ones named

They are what the corpus sweep (`c_sweep.py --android`) cannot be: programs that draw, take
input and play sound, each printing what it read back (a captured pixel, an event, the frames
the audio device took), so the comparison is exact and hand-computed.  A guard is a file in
`tests/android/` or one directory below it (a program with files of its own beside it); a
`<name>.assets` holds the `--assets` list of the files the app carries.  A `<name>.input` holds
`adb shell input` lines (`tap 100 200`), sent once the program prints `ready`; the device's
animations are switched off first.  `--lib` points
at a library checkout (`loft --lib`); without it the installed `graphics` and `audio` are used.
Needs `adb` (ANDROID_HOME), the NDK (ANDROID_NDK_HOME) and a JDK for the packaging.
"""
import os
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from c_sweep import LOFT, ROOT, adb, run  # noqa: E402

GUARDS = ROOT / "tests" / "android"
RUN_LIMIT = 90


def read_file(a, pkg, name):
    """A file in the app's files directory, b"" while it is not there."""
    _, out, _ = run([a, "exec-out", "run-as", pkg, "cat", f"files/{name}"], 30)
    return b"" if out.startswith(b"cat: ") else out


def one(src: Path, libs: list) -> tuple:
    """(passed, what it said) for one guard."""
    expected = src.with_suffix(".expected").read_bytes()
    inputs = src.with_suffix(".input")
    lines = inputs.read_text(encoding="utf-8").split("\n") if inputs.exists() else []
    stem = src.stem.replace("-", "_")
    pkg = f"com.loft.{stem}"
    a = adb()
    with tempfile.TemporaryDirectory(prefix="loft-android-guard-") as tmp:
        apk = os.path.join(tmp, f"{stem}.apk")
        cmd = [LOFT] + [x for d in libs for x in ("--lib", d)]
        listed = src.with_suffix(".assets")
        if listed.exists():
            cmd += ["--assets", listed.read_text(encoding="utf-8").strip()]
        code, _, err = run(cmd + ["--native-c", "--native-android", apk, str(src)], 600,
                           env={"LOFT_ANDROID_TARGET": os.environ.get("LOFT_ANDROID_TARGET",
                                                                       "x86_64-linux-android")})
        if code != 0:
            return False, "build: " + err.decode(errors="replace").strip()[-300:]
        run([a, "uninstall", pkg], 60)
        code, _, err = run([a, "install", apk], 120)
        if code != 0:
            return False, "install: " + err.decode(errors="replace").strip()[-200:]
    run([a, "shell", "am", "start", "-W", "-n", f"{pkg}/android.app.NativeActivity"], 60)
    t0 = time.monotonic()
    sent = not lines
    status = b""
    while time.monotonic() - t0 < RUN_LIMIT:
        if not sent and b"ready\n" in read_file(a, pkg, "loft-stdout.txt"):
            for line in filter(None, (l.strip() for l in lines)):
                run([a, "shell", "input"] + line.split(), 30)
            sent = True
        status = read_file(a, pkg, "loft-exit.txt").strip()
        if status:
            break
        time.sleep(0.3)
    out = read_file(a, pkg, "loft-stdout.txt")
    run([a, "shell", "am", "force-stop", pkg], 30)
    run([a, "uninstall", pkg], 60)
    if not status:
        return False, f"no exit status within {RUN_LIMIT} s; stdout so far: {out[-200:]!r}"
    if status != b"0" or out != expected:
        return False, f"exit {status.decode()}, stdout {out!r}, expected {expected!r}"
    return True, ""


def main(argv) -> int:
    if any(a in ("-h", "--help") for a in argv[1:]):
        print(__doc__.strip())
        return 0
    libs, names, i = [], [], 1
    while i < len(argv):
        if argv[i] == "--lib":
            libs.append(argv[i + 1])
            i += 2
        else:
            names.append(argv[i])
            i += 1
    # A window's opening animation moves it under the input the runner sends, so a tap lands
    # off its point: the device's animations are switched off, as UI test runners do.
    a = adb()
    for scale in ("window_animation_scale", "transition_animation_scale",
                  "animator_duration_scale"):
        run([a, "shell", "settings", "put", "global", scale, "0"], 30)
    guards = sorted(g for g in [*GUARDS.glob("*.loft"), *GUARDS.glob("*/*.loft")] if g.is_file())
    if names:
        guards = [g for g in guards if g.stem in names]
    failed = 0
    for g in guards:
        ok, why = one(g, libs)
        if not ok:
            failed += 1
            print(f"FAIL  {g.relative_to(ROOT)} — {why}")
    if failed:
        print(f"android guards: {failed} of {len(guards)} failed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
