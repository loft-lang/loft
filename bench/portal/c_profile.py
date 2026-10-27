#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""c_profile.py — where a bench routine's time goes in the C99 backend's build.

    python3 bench/portal/c_profile.py 13 [--routine join,split] [--lane c|cship] [--n N]
                                         [--leaf] [--cpu K] [--keep DIR] [--c FILE]

Emits the bench (a number like `13` or a directory) as `stats.py`'s lane does — `cship`
under LOFT_RELEASE_PASS_PROBE=1 — compiles it `-O3 -g -fno-omit-frame-pointer
-fno-inline-functions-called-once` (the lane's code, with the frames a call graph needs and
the once-called runtime helpers kept apart so they show by name), runs it under `perf record
--call-graph dwarf` (glibc keeps no frame pointers) pinned to one core, and folds the samples per ROUTINE: the routine's share
of the run, then the functions its samples land in.  By default a sample is charged to the
innermost RUNTIME frame (`loft_*`, libc's memory and allocation calls) and otherwise to
`(generated)` — the question is usually "the runtime, or the code the backend wrote?";
`--leaf` charges the innermost frame of any kind instead.

`--c FILE` profiles a unit you edited (bench/portal/hand_price_c.sh's workflow) instead of
emitting one; `--keep DIR` keeps the unit, the binary and perf.data.  `--n` defaults to the
`--n` at which the slowest routine kept runs about two seconds.  Read the shares, not the
absolute numbers: `bench/stats.py` is the measurement, this is where to look.
"""
import argparse
import collections
import glob
import os
import re
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, os.path.dirname(HERE))
import stats  # noqa: E402

CFLAGS = ["-std=c99", "-O3", "-g", "-fno-omit-frame-pointer", "-fno-inline-functions-called-once"]
RUNTIME_CALLS = {"memcpy", "memmove", "memset", "memchr", "memcmp", "malloc", "free", "realloc",
                 "calloc", "strlen"}


def clean(sym):
    """A symbol as its function: no GCC clone suffix, and a `__fr` twin (the body a function's
    call-chain frame wraps, `LOFT_CALL_CHAIN`) as the loft function it is."""
    sym = re.sub(r"\.(constprop|isra|part|cold)(\.\d+)?", "", sym)
    return sym[:-4] if sym.endswith("__fr") else sym


def fold(script_lines, leaf_mode):
    """perf script `-F ip,sym` stacks → {routine: Counter(function)}.  A routine is the
    outermost loft function (`n_*`) below `n_main`; samples outside `n_main` are dropped."""
    per = collections.defaultdict(collections.Counter)
    frames = []

    def flush():
        names = [clean(s) for s, _ in frames]
        if "n_main" not in names:
            return
        i = names.index("n_main")
        # The outermost loft function below `n_main`, inlined or not (GCC folds small
        # routines into main); a sample in none of them is main's own.
        lofts = [x for x in names[:i] if x.startswith("n_")]
        routine = lofts[-1] if lofts else "n_main (itself)"
        if leaf_mode:
            where = names[0]
        else:
            where = next((n for n in names[:i] if n.startswith("loft_") or n.startswith("__")
                          or n in RUNTIME_CALLS), "(generated)")
        per[routine][where] += 1

    for line in script_lines:
        if not line.strip():
            if frames:
                flush()
            frames = []
            continue
        parts = line.split()
        if len(parts) >= 2:
            frames.append((parts[1].split("+")[0], "(inlined)" in line))
    if frames:
        flush()
    return per


def bench_dir(spec):
    if os.path.isdir(spec):
        return os.path.abspath(spec)
    hits = sorted(glob.glob(os.path.join(ROOT, "bench", f"{spec}_*")))
    if not hits:
        stats.fail(f"no bench directory matches {spec!r}")
    return hits[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("bench")
    ap.add_argument("--routine", default="", help="comma-separated routines to report")
    ap.add_argument("--lane", choices=("c", "cship"), default="c")
    ap.add_argument("--n", type=int, default=0)
    ap.add_argument("--leaf", action="store_true", help="charge the innermost frame, any kind")
    ap.add_argument("--cpu", default="", help="the core to pin to (default: the fastest)")
    ap.add_argument("--keep", default="", help="a directory to keep the unit, binary and perf.data in")
    ap.add_argument("--c", default="", help="profile this C unit instead of emitting the bench")
    ap.add_argument("--cc", default="cc")
    ap.add_argument("--freq", type=int, default=1999)
    a = ap.parse_args()
    for tool in ("perf", a.cc):
        if not shutil.which(tool):
            stats.fail(f"{tool} is not on PATH")
    d = bench_dir(a.bench)
    if a.keep:
        work = os.path.abspath(a.keep)
    else:
        sys.path.insert(0, os.path.join(ROOT, "scripts"))
        from run_scratch import run_root  # removed at exit, and by the next run if this one is killed
        work = run_root("c-profile")
    os.makedirs(work, exist_ok=True)
    unit = os.path.join(work, "bench.c")
    exe = os.path.join(work, "bench")
    loft = os.path.join(ROOT, "target", "release", "loft")
    if a.c:
        shutil.copyfile(a.c, unit)
    else:
        env = {**os.environ, **({"LOFT_RELEASE_PASS_PROBE": "1"} if a.lane == "cship" else {})}
        r = subprocess.run([loft, "--emit-c", unit, "--path", ROOT + "/", os.path.join(d, "bench.loft")],
                           capture_output=True, text=True, env=env, cwd=d)
        if r.returncode != 0:
            stats.fail(f"loft --emit-c: {(r.stderr or r.stdout)[-2000:]}")
    r = subprocess.run([a.cc, *CFLAGS, "-o", exe, unit, "-lm"], capture_output=True, text=True)
    if r.returncode != 0:
        stats.fail(f"{a.cc}: {r.stderr[-2000:]}")
    keep_names = {x.strip() for x in a.routine.split(",") if x.strip()}
    keep = (lambda name: name in keep_names) if keep_names else None
    pin = ["taskset", "-c", a.cpu] if a.cpu else stats.pin_prefix(1, True)
    n = a.n or stats.calibrate([exe], pin, 2_000_000.0, "c_profile", d, keep, 8_000_000.0)
    data = os.path.join(work, "perf.data")
    r = subprocess.run(["perf", "record", "-q", "-F", str(a.freq), "--call-graph", "dwarf,16384", "-o", data,
                        "--", *pin, exe, "--n", str(n)], capture_output=True, text=True, cwd=d)
    if r.returncode != 0:
        stats.fail(f"perf record: {r.stderr[-2000:]}")
    rows = {}
    run_us = 0
    for line in r.stdout.splitlines():
        parts = line.split("\t")
        if len(parts) == 7 and parts[0] != "routine":
            rows[parts[0]] = int(parts[3])
            run_us += int(parts[2])
    s = subprocess.run(["perf", "script", "-i", data, "-F", "ip,sym"], capture_output=True, text=True)
    per = fold(s.stdout.splitlines(), a.leaf)
    total = sum(sum(c.values()) for c in per.values()) or 1
    print(f"# {os.path.relpath(d, ROOT)} [{a.lane}] --n {n}, {total} samples; unit, binary and "
          f"perf.data in {work}" + ("" if a.keep else " until the next run"))
    # perf drops whole buffers when its writer falls behind — a loaded box, or DWARF stack
    # copies outrunning the mlock-limited ring — and says so only without `-q`.  The loss is
    # not uniform: the routines that ran first keep their samples, so the shares lie.
    expected = run_us / 1e6 * a.freq
    if expected and total < 0.7 * expected:
        print(f"# warning: perf kept {total} of the ~{expected:.0f} samples the routines' own times "
              f"predict; the shares favour the routines that ran first — profile on an idle box")
    for routine, c in sorted(per.items(), key=lambda kv: -sum(kv[1].values())):
        # `n_` is the C backend's prefix; a bench names its routine functions with one more
        # word (`t_join`, `c_enum_match`), so the name is the row the program prints.
        fn = routine[2:] if routine.startswith("n_") else routine
        name = next((x for x in (fn, fn.split("_", 1)[-1]) if x in rows), fn)
        if keep and not keep(name):
            continue
        k = sum(c.values())
        if not keep and k / total < 0.01:
            continue
        ns = f" {rows[name]:,} ns/op" if name in rows else ""
        tops = ", ".join(f"{f} {100 * v / k:.0f}%" for f, v in c.most_common(9) if v / k >= 0.03)
        print(f"{100 * k / total:5.1f}%  {name}{ns}: {tops}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
