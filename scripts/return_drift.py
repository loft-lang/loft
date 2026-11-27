#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The return-drift census (@PLN233 phase 0): functions whose return protocol pass 2 changed.

Each corpus program is parsed with `LOFT_RETURN_DRIFT=1`, which makes the parser report every
function whose return dependencies differ between the two passes (`Parser::report_return_drift`).
HARMFUL is pass 1 recording a borrow of a visible parameter that pass 2 drops: a caller typed on
pass 1 keeps the borrow and never frees what it owns (loft#2660).

    scripts/return_drift.py [--loft LOFT] [--jobs N] [--list]   # the census, per program
    scripts/return_drift.py --check                              # fail when HARMFUL rose
    scripts/return_drift.py --write-baseline                     # record today's counts

The baseline (`index/return_drift.tsv`) is a ratchet: the harmful count may fall and may not
rise.  The corpus is `scripts/corpus_files.sh` (tests/scripts, tests/docs, examples, the suite).
"""
import concurrent.futures as cf
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BASELINE = os.path.join(ROOT, "index", "return_drift.tsv")


def corpus():
    r = subprocess.run(["bash", os.path.join(ROOT, "scripts", "corpus_files.sh")], cwd=ROOT,
                       capture_output=True, text=True)
    return [l for l in r.stdout.splitlines() if l.strip()]


def drift_of(loft, path):
    env = dict(os.environ, LOFT_RETURN_DRIFT="1", LOFT_NO_CACHE="1")
    full = os.path.join(ROOT, path)
    try:
        r = subprocess.run([loft, "introspect", os.path.basename(full)], cwd=os.path.dirname(full),
                           env=env, capture_output=True, text=True, timeout=120)
        err = r.stderr
    except subprocess.TimeoutExpired:
        return path, None
    rows = []
    for line in err.splitlines():
        if line.startswith("return-drift\t"):
            parts = line.split("\t")
            if len(parts) >= 6:
                rows.append((parts[1], parts[2], parts[3], parts[4], parts[5]))
    return path, rows


def measure(loft, jobs):
    files = corpus()
    out = {}
    with cf.ThreadPoolExecutor(max_workers=jobs) as pool:
        for path, rows in pool.map(lambda p: drift_of(loft, p), files):
            out[path] = rows
    return files, out


def counts(out):
    names, programs, harmful, hnames, hprograms = set(), set(), 0, set(), set()
    for path, rows in out.items():
        for name, at, p1, p2, kind in rows or []:
            names.add(name)
            programs.add(path)
            if kind == "harmful":
                harmful += 1
                hnames.add(name)
                hprograms.add(path)
    return {"drift-names": len(names), "drift-programs": len(programs), "harmful": harmful,
            "harmful-names": len(hnames), "harmful-programs": len(hprograms)}


def main():
    args = sys.argv[1:]
    if "-h" in args or "--help" in args:
        print(__doc__)
        return 0
    loft = os.path.join(ROOT, "target", "debug", "loft")
    if "--loft" in args:
        loft = os.path.abspath(args[args.index("--loft") + 1])
    jobs = int(args[args.index("--jobs") + 1]) if "--jobs" in args else 6
    files, out = measure(loft, jobs)
    c = counts(out)
    timed = sum(1 for v in out.values() if v is None)
    print(f"return drift over {len(files)} programs ({timed} timed out): "
          f"{c['drift-names']} function(s) in {c['drift-programs']} program(s) change their return "
          f"between the passes; HARMFUL {c['harmful']} instance(s), {c['harmful-names']} name(s), "
          f"{c['harmful-programs']} program(s)")
    if "--list" in args:
        for path in sorted(out):
            for name, at, p1, p2, kind in out[path] or []:
                if kind == "harmful":
                    print(f"  {path}\t{name}\t{p1} -> {p2}")
    if "--write-baseline" in args:
        with open(BASELINE, "w", encoding="utf-8") as fh:
            fh.write("# The return-drift census (scripts/return_drift.py, @PLN233): counts over the corpus.\n"
                     "# A ratchet: `harmful` may fall and may not rise.\n")
            for k, v in c.items():
                fh.write(f"{k}\t{v}\n")
        print(f"wrote {os.path.relpath(BASELINE, ROOT)}")
    if "--check" in args:
        base = {}
        if os.path.exists(BASELINE):
            for line in open(BASELINE, encoding="utf-8"):
                if line.startswith("#") or "\t" not in line:
                    continue
                k, v = line.rstrip("\n").split("\t")
                base[k] = int(v)
        was = base.get("harmful")
        if was is not None and c["harmful"] > was:
            print(f"return drift: HARMFUL rose {was} -> {c['harmful']} — a function's pass-1 return "
                  "borrows a parameter pass 2 no longer does (--list names them)")
            return 1
        if was is not None and c["harmful"] < was:
            print(f"return drift: HARMFUL fell {was} -> {c['harmful']} — record it: --write-baseline")
    return 0


if __name__ == "__main__":
    sys.exit(main())
