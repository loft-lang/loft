#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Did this change move a routine?  Measure the lanes it touched and compare with the last
measurement on this machine — before committing.

    scripts/perf_check.py                   # the programs whose rewrite ADMISSIONS moved
                                            #   against the census baseline (the compiler
                                            #   change touched them), measured and compared
    scripts/perf_check.py --only 16,17,18   # these lanes (bench/stats.py's --only syntax)
    scripts/perf_check.py --package ../loft-bench-libs/loft-libs-graphics/drawing=drawing
    scripts/perf_check.py --routine check_request,cbor/decode   # these routines only
    scripts/perf_check.py --baseline run.tsv    # compare with a saved `stats.py --tsv` run
    scripts/perf_check.py --record          # …and make this run the machine's baseline
                                            #   (bench/portal/results/<host>.tsv — commit it)

Why
---
A rewrite that fires at fewer sites shows in `make rewrite-census`; a routine that got
SLOWER while every admission stayed shows nowhere until the next portal run, days later
(`scripts/perf_trend.py` reads that history: `ease` +41 % between two joins, seen after the
fact).  This is the same comparison at the moment it is cheap to act on: the lanes this
change reached, against the rows the machine last committed.

What is compared is the RATIO to the Rust twin, measured interleaved on the same core in
the same minute, so the day's load cancels; the native time is shown beside it.  A move of
`--threshold` percent or more (15) in either direction is listed, a slowdown makes the exit
code 1.  A report for the committer's judgement, never a gate: a routine on a 200 ns twin
swings 10 % run to run (`bench/stats.py` flags those rows noisy), and a genuine move is
one that repeats.

Without a results file for this host and no `--baseline`, there is nothing to compare: the
run is measured and, with `--record`, becomes the baseline the next check reads.
"""
import argparse
import os
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "bench" / "portal"))
from portal import host_key  # noqa: E402
RESULTS = ROOT / "bench" / "portal" / "results"


def read_tsv(path):
    rows, header = {}, None
    for line in Path(path).read_text().splitlines():
        if line.startswith("#") or not line.strip():
            continue
        parts = line.split("\t")
        if header is None:
            header = parts
            continue
        r = dict(zip(header, parts))
        rows[(r.get("bench", ""), r.get("routine", ""))] = r
    return rows


def changed_programs():
    """The census's verdict on the current compiler: the programs whose admissions moved."""
    proc = subprocess.run([sys.executable, "scripts/rewrite_census.py", "--verbose"],
                          cwd=ROOT, capture_output=True, text=True, check=False)
    names = set()
    for line in proc.stdout.splitlines() + proc.stderr.splitlines():
        line = line.strip()
        if line.startswith("note:") or line.startswith("DROP") or " in " in line and ": " in line and "→" in line:
            # "note: R-Base in 16_consumer_shapes: 3 → 5" / "DROP R-X in drawing: 4 → 2"
            try:
                names.add(line.split(" in ", 1)[1].split(":", 1)[0].strip())
            except IndexError:
                pass
    return names


def lanes_for(names):
    """stats.py arguments for these census program names: in-repo lanes by number, library
    packages by their checkout (`portal.library_packages`)."""
    from portal import library_packages  # noqa: E402
    only, packages = [], []
    for d in sorted((ROOT / "bench").glob("[0-9][0-9]_*")):
        if d.name in names:
            only.append(d.name[:2])
    pkgs = library_packages()
    for spec in pkgs[1::2]:
        pkg, _, name = spec.rpartition("=")
        if name in names:
            packages += ["--package", spec]
    return only, packages


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--only", help="lanes, bench/stats.py's syntax")
    ap.add_argument("--package", action="append", default=[], help="a library bench DIR=NAME (repeatable)")
    ap.add_argument("--routine", action="append", default=[],
                    help="[BENCH/]NAME[,…]: only these routines, in the programs that hold them")
    ap.add_argument("--baseline", help="a saved stats.py --tsv run to compare with")
    ap.add_argument("--threshold", type=float, default=15.0)
    ap.add_argument("--record", action="store_true", help="merge this run into the machine's results file")
    ap.add_argument("--no-build", action="store_true", help="skip `cargo build --release --bin loft`")
    a = ap.parse_args()
    host = host_key()
    if not a.no_build:
        subprocess.run(["cargo", "build", "--release", "--bin", "loft", "-q"], cwd=ROOT, check=True)
    only, packages = a.only, []
    for p in a.package:
        packages += ["--package", p]
    if a.routine:
        pass  # stats.py places the routines in their programs itself
    elif not only and not packages:
        names = changed_programs()
        if not names:
            print("the census moved no program: nothing this change touched to measure")
            return 0
        lanes, packages = lanes_for(names)
        only = ",".join(lanes) if lanes else None
        print(f"measuring what the census says moved: {', '.join(sorted(names))}")
    with tempfile.NamedTemporaryFile(suffix=".tsv", delete=False) as tmp:
        out = tmp.name
    cmd = [sys.executable, "bench/stats.py", "--tsv", out]
    if only:
        cmd += ["--only", only]
    elif packages:
        cmd += ["--no-suite"]
    cmd += packages
    for r in a.routine:
        cmd += ["--routine", r]
    subprocess.run(cmd, cwd=ROOT, check=True, stdout=subprocess.DEVNULL)
    now = read_tsv(out)
    base_path = Path(a.baseline) if a.baseline else RESULTS / f"{host}.tsv"
    if not base_path.exists():
        print(f"no baseline to compare with ({base_path}); measured {len(now)} routine(s)")
        if a.record:
            record(cmd, out)
        return 0
    base = read_tsv(base_path)
    moved, missing = [], 0
    for key, r in sorted(now.items()):
        b = base.get(key)
        if not b:
            missing += 1
            continue
        try:
            r0, r1 = float(b["ratio"]), float(r["ratio"])
            n0, n1 = float(b["native_ns"]), float(r["native_ns"])
        except (KeyError, ValueError):
            continue
        d_ratio = (r1 - r0) / r0 * 100.0 if r0 else 0.0
        d_ns = (n1 - n0) / n0 * 100.0 if n0 else 0.0
        if abs(d_ratio) >= a.threshold:
            moved.append((d_ratio, d_ns, key, r0, r1, n0, n1, r.get("flags", ""), b.get("commit", "?"), b.get("date", "?")))
    print(f"{len(now)} routine(s) measured, {len(now) - missing} with a baseline row in {base_path.name}"
          + (f", {missing} new" if missing else ""))
    slower = [m for m in moved if m[0] > 0]
    if not moved:
        print(f"no routine moved {a.threshold:g} % or more against its baseline ratio")
    else:
        moved.sort(key=lambda m: -abs(m[0]))
        print(f"{'Δ ratio':>8} {'Δ native':>9}  {'routine':24} {'bench':16}  ratio before → after   native before → after   baseline")
        for d_ratio, d_ns, (bench, routine), r0, r1, n0, n1, flags, bc, bd in moved:
            tag = " SLOWER" if d_ratio > 0 else " faster"
            print(f"{d_ratio:+7.1f}% {d_ns:+8.1f}%  {routine:24} {bench:16}  {r0:5.2f} → {r1:5.2f}   {n0:12,.0f} → {n1:12,.0f}   {bc} ({bd}){tag}{' ' + flags if flags else ''}")
    if a.record:
        record(cmd, out)
    os.unlink(out)
    return 1 if slower else 0


def record(cmd, out):
    """This run becomes the machine's latest measurement, the way `make perf-portal` records
    one (`portal.measure` merges a partial run into `results/<host>.tsv`)."""
    from portal import merge_run  # noqa: E402
    merge_run(out)
    print(f"recorded into {RESULTS}/ — commit it with the change")


if __name__ == "__main__":
    sys.exit(main())
