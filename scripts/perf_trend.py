#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""How each measured routine has moved over time — read off the committed portal results.

    scripts/perf_trend.py                       # the MOVERS: every routine whose latest
                                                #   measurement differs from the one before
                                                #   it by 15 % or more, on any machine
    scripts/perf_trend.py --routine fill_polygon    # one routine's whole series
    scripts/perf_trend.py --threshold 25 --host laptop
    scripts/perf_trend.py --all                 # every routine's latest two points

Why
---
`bench/portal/results/<host>.tsv` holds ONE row per routine — the latest measurement on
that machine — and `make perf-portal` overwrites it.  The history is in git: every commit
that touched a results file is one measurement of the whole lane set on that machine.  A
routine that slowed by 40 % between two joins shows nowhere else: the portal page reads the
latest row, the speed gate reads test times, the census reads admissions.  This script walks
that history and reads each routine as a SERIES, so a move is named with the two commits
it sits between (`git log a..b -- src/` is then the bisect range).

A REPORT, never a gate: two measurements on one machine at two hours of one day differ by
the day's load, and the flag threshold is a coarse one on purpose.  Ratios compare across
time on ONE machine only; a machine's rows are never compared with another's.
"""
import argparse
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RESULTS = "bench/portal/results"


def git(*args):
    return subprocess.run(["git", *args], cwd=ROOT, capture_output=True, text=True, check=False).stdout


def history():
    """Every committed measurement: (git commit, git date, host, meta, rows) oldest first."""
    commits = git("log", "--format=%h %ad", "--date=short", "--", RESULTS).splitlines()
    out = []
    for line in reversed(commits):
        commit, date = line.split(maxsplit=1)
        files = git("ls-tree", "--name-only", commit, "--", RESULTS + "/").splitlines()
        for f in files:
            if not f.endswith(".tsv"):
                continue
            text = git("show", f"{commit}:{f}")
            meta, header, rows = {}, None, []
            for l in text.splitlines():
                if l.startswith("#"):
                    if "=" in l:
                        k, v = l[1:].strip().split("=", 1)
                        meta[k.strip()] = v.strip()
                    continue
                parts = l.rstrip("\n").split("\t")
                if header is None:
                    header = parts
                    continue
                rows.append(dict(zip(header, parts)))
            host = meta.get("host") or Path(f).stem
            out.append((commit, date, host, meta, rows))
    return out


def series(hist, host_filter=None):
    """(host, bench, routine) -> [(measured date, loft commit, native ns, rust ns, ratio, git commit)],
    one point per DISTINCT measurement: a re-commit of the same row is not a new point."""
    s = defaultdict(list)
    for commit, _date, host, meta, rows in hist:
        if host_filter and host != host_filter:
            continue
        for r in rows:
            try:
                nat = float(r.get("native_ns") or "nan")
                rust = float(r.get("rust_ns") or "nan")
                ratio = float(r.get("ratio") or "nan")
            except ValueError:
                continue
            key = (host, r.get("bench", ""), r.get("routine", ""))
            point = (r.get("date") or meta.get("date", ""), r.get("commit") or meta.get("commit", ""), nat, rust, ratio, commit)
            if s[key] and s[key][-1][:5] == point[:5]:
                continue
            s[key].append(point)
    return s


def pct(a, b):
    return (b - a) / a * 100.0 if a else float("nan")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--routine", help="one routine's series")
    ap.add_argument("--host", help="one machine's rows only")
    ap.add_argument("--threshold", type=float, default=15.0, help="flag a move of this many percent (15)")
    ap.add_argument("--all", action="store_true", help="every routine's latest two points")
    a = ap.parse_args()
    s = series(history(), a.host)
    if not s:
        print("no committed measurements under", RESULTS)
        return 2
    if a.routine:
        found = False
        for (host, bench, routine), pts in sorted(s.items()):
            if routine != a.routine:
                continue
            found = True
            print(f"{routine} · {bench} · {host}")
            print(f"  {'measured':10} {'loft':10} {'native ns':>12} {'rust ns':>10} {'ratio':>7} {'Δ native':>9}  committed")
            prev = None
            for date, loft, nat, rust, ratio, commit in pts:
                d = f"{pct(prev, nat):+8.1f}%" if prev else f"{'':9}"
                print(f"  {date:10} {loft:10} {nat:12,.0f} {rust:10,.0f} {ratio:7.2f} {d}  {commit}")
                prev = nat
        if not found:
            print(f"no routine named {a.routine!r} in the committed results")
            return 2
        return 0
    rows = []
    for (host, bench, routine), pts in sorted(s.items()):
        if len(pts) < 2:
            continue
        p, q = pts[-2], pts[-1]
        dn = pct(p[2], q[2])
        if a.all or abs(dn) >= a.threshold:
            rows.append((dn, host, bench, routine, p, q))
    if not rows:
        print(f"no routine moved {a.threshold:g} % or more between its last two measurements")
        return 0
    rows.sort(key=lambda r: -abs(r[0]))
    print(f"{'Δ native':>9}  {'routine':24} {'bench':14} {'host':8} {'before':>12} {'after':>12}  ratio before → after   between")
    for dn, host, bench, routine, p, q in rows:
        print(f"{dn:+8.1f}%  {routine:24} {bench:14} {host:8} {p[2]:12,.0f} {q[2]:12,.0f}  {p[4]:5.2f} → {q[4]:5.2f}   {p[1]} ({p[0]}) → {q[1]} ({q[0]})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
