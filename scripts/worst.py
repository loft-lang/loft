#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The worst routines of the last complete run, re-timed quickly — `make worst`.

Two full measurements answer where loft stands, and both are slow: `make interp-gap` (the
interpreter against native code, every bench routine, with censuses) and `make perf-portal`
(native code against its Rust twin, every bench and library routine).  Between two of them the
question is narrower — did this change move the routines that are worst?  This takes a WORKING
SET from the last complete run on each axis and re-times only the lanes that hold it:

    interp   interpreter / native, from target/interp-gap/report.json (the last `make
             interp-gap`): the routines at or above the 100x cliff, at least --min of them
    native   native / Rust, from bench/portal/results/<host>.tsv (the last portal run on this
             machine): the --min worst, a library's routine through its own bench (--package)

A new full run is what redefines the set; nothing is kept by hand.  The set does not have to
be exactly the worst — a good working set is the point — so it is never re-derived from the
quick runs.  Each row shows the full run's ratio beside the one just measured.

    make worst                         # both axes
    make worst ARGS="--axis interp"    # one axis
    make worst ARGS="--list"           # the sets only, nothing timed

A report, never a gate.  A bench program times all its routines, so a lane is run whole and
only the set's routines are shown.
"""
import argparse
import json
import os
import platform
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
HOST = platform.node() or "unknown"
# axis -> (stats.py lanes, numerator column, denominator column, default cut, default min)
AXES = {
    "interp": ("native,interp", "interp_ns", "native_ns", 100.0, 10),
    "native": ("native,rust", "native_ns", "rust_ns", 1e9, 20),
}


def read_tsv(path):
    rows, head = [], None
    for line in open(path):
        if line.startswith("#") or not line.strip():
            continue
        parts = line.rstrip("\n").split("\t")
        if head is None:
            head = parts
            continue
        rows.append(dict(zip(head, parts)))
    return rows


def last_run(axis):
    """(bench, routine, ratio) of every routine the last complete run measured, worst first,
    and where it was read from — or None when there is no run."""
    if axis == "interp":
        src = ROOT / "target/interp-gap/report.json"
        if not src.exists():
            return None
        rows = [(r["bench"], r["routine"], r["ratio"]) for r in json.load(open(src))["routines"]
                if r.get("ratio")]
    else:
        src = ROOT / f"bench/portal/results/{HOST}.tsv"
        if not src.exists():
            return None
        latest = {}
        for r in read_tsv(src):  # chronological: a routine's last row is its latest
            try:
                latest[(r["bench"], r["routine"])] = float(r["ratio"])
            except (KeyError, ValueError):
                pass
        rows = [(b, n, v) for (b, n), v in latest.items()]
    rows.sort(key=lambda r: -r[2])
    return rows, src


def working_set(rows, cut, least):
    over = [r for r in rows if r[2] >= cut]
    return over if len(over) >= least else rows[:least]


def library_packages(names):
    """The portal's `--package` arguments for the libraries in `names` (its one home)."""
    sys.path.insert(0, str(ROOT / "bench" / "portal"))
    import portal  # noqa: E402

    specs = portal.library_packages()
    out = []
    for flag, spec in zip(specs[::2], specs[1::2]):
        if spec.rsplit("=", 1)[-1] in names:
            out += [flag, spec]
    return out


def measure(axis, picked, samples, loft):
    lanes, num, den, _, _ = AXES[axis]
    suite = sorted({b for b, _, _ in picked if (ROOT / "bench" / b).is_dir()})
    libs = {b for b, _, _ in picked} - set(suite)
    fd, out = tempfile.mkstemp(suffix=".tsv")
    os.close(fd)
    cmd = [sys.executable, str(ROOT / "bench/stats.py"), "--lanes", lanes, "--samples", str(samples),
           "--tsv", out, "--loft", loft]
    cmd += ["--only", ",".join(suite)] if suite else ["--no-suite"]
    cmd += library_packages(libs) if libs else []
    print(f"# {axis}: re-timing {', '.join(suite + sorted(libs))}", flush=True)
    if subprocess.run(cmd, stdout=subprocess.DEVNULL).returncode != 0:
        print(f"# {axis}: stats.py reported a failure; the rows it measured are used", flush=True)
    now = {}
    for r in read_tsv(out) if os.path.getsize(out) else []:
        try:
            now[(r["bench"], r["routine"])] = (float(r[num]), float(r[den]))
        except (KeyError, ValueError):
            pass
    os.remove(out)
    return now


def fmt_ns(ns):
    for unit, scale in (("s", 1e9), ("ms", 1e6), ("us", 1e3)):
        if ns >= scale:
            return f"{ns / scale:,.2f} {unit}"
    return f"{ns:,.0f} ns"


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0],
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--axis", choices=list(AXES))
    ap.add_argument("--cut", type=float, help="routines at or above this ratio (interp default 100)")
    ap.add_argument("--min", type=int, help="at least this many routines (interp 10, native 20)")
    ap.add_argument("--samples", type=int, default=3, help="stats.py samples per lane")
    ap.add_argument("--loft", default=str(ROOT / "target/release/loft"))
    ap.add_argument("--list", action="store_true", help="print the sets; time nothing")
    a = ap.parse_args()

    for axis in [a.axis] if a.axis else list(AXES):
        lanes, num, den, cut, least = AXES[axis]
        run = last_run(axis)
        if run is None:
            print(f"# {axis}: no complete run to take a set from "
                  f"({'make interp-gap' if axis == 'interp' else 'make perf-portal'})")
            continue
        rows, src = run
        picked = working_set(rows, a.cut if a.cut is not None else cut, a.min if a.min is not None else least)
        stamp = time.strftime("%Y-%m-%d %H:%M", time.localtime(src.stat().st_mtime))
        print(f"\n## {axis} — {num.split('_')[0]} / {den.split('_')[0]}: {len(picked)} of {len(rows)} routines "
              f"from {src.relative_to(ROOT)} ({stamp})\n")
        now = {} if a.list else measure(axis, picked, a.samples, a.loft)
        print(f"| bench | routine | full run | now | change | {num.split('_')[0]} now | {den.split('_')[0]} now |")
        print("|---|---|--:|--:|--:|--:|--:|")
        out = []
        for b, n, v in picked:
            if (b, n) in now and now[(b, n)][1] > 0:
                x, y = now[(b, n)]
                r = x / y
                out.append((r, b, n, v, f"{r:,.1f}x", f"{(r / v - 1) * 100:+.0f} %", fmt_ns(x), fmt_ns(y)))
            else:
                out.append((v, b, n, v, "—", "", "", ""))
        out.sort(key=lambda o: -o[0])
        for _, b, n, v, r, ch, x, y in out:
            print(f"| {b} | {n} | {v:,.1f}x | {r} | {ch} | {x} | {y} |")
        timed = [o for o in out if o[4] != "—"]
        if timed:
            gm = 1.0
            for o in timed:
                gm *= o[0] / o[3]
            gm **= 1 / len(timed)
            print(f"\n{axis}: {len(timed)} routines re-timed; geometric mean {(gm - 1) * 100:+.1f} % against the full run.")


if __name__ == "__main__":
    main()
