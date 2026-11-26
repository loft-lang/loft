#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Does every suite cell that says it catches a cure's defect get caught by the store records?

A language-suite cell that declares `// @catches: LOFT_NO_<CURE>` promises its output moves when
the cure is switched off; `scripts/switch_ab.sh` holds the RUNTIME half of that promise.  This
holds the STATIC half: with the cure off, the store records (the compile-time lifetime check)
must report the defect the cell was written for — `scripts/own_sweep.py` reads such a run as
CAUGHT.  A cure whose defect the records cannot see (a stale value rather than a store's
lifetime) is listed in the baseline with the verdict it reads today, and the list only shrinks.

    scripts/catches_records.py                # check every pair against the baseline
    scripts/catches_records.py --update       # write today's non-CAUGHT verdicts as the baseline
    scripts/catches_records.py --loft BIN     # the loft binary (default target/release/loft)

Fails when a pair reads worse than the baseline says (a CAUGHT pair that no longer is, a new
`@catches` cell the records do not catch), and names the pairs the baseline can drop.
"""
import collections
import os
import re
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BASELINE = os.path.join(HERE, "tests", "catches_records.baseline")
CATCHES = re.compile(r"^// @catches: (LOFT_NO_[A-Z0-9_]+)", re.M)


def suite_dir():
    return os.environ.get("LOFT_SUITE_DIR", os.path.join(HERE, "..", "loft-suite"))


def pairs(suite):
    """(cell relative to the suite, switch) for every `@catches` line under rules/."""
    out = []
    for root, _, files in os.walk(os.path.join(suite, "rules")):
        for f in sorted(files):
            if not f.endswith(".loft"):
                continue
            path = os.path.join(root, f)
            with open(path, encoding="utf-8") as fh:
                for sw in CATCHES.findall(fh.read()):
                    out.append((os.path.relpath(path, suite), sw))
    return sorted(set(out))


def read_baseline():
    base = {}
    if os.path.exists(BASELINE):
        with open(BASELINE, encoding="utf-8") as fh:
            for line in fh:
                if line.strip() and not line.startswith("#"):
                    cell, sw, verdict = line.rstrip("\n").split("\t")[:3]
                    base[(cell, sw)] = verdict
    return base


def measure(loft, suite, todo):
    """The own_sweep verdict per pair, one run per switch over that switch's cells."""
    by_switch = collections.defaultdict(list)
    for cell, sw in todo:
        by_switch[sw].append(cell)
    verdicts = {}
    for sw, cells in sorted(by_switch.items()):
        with tempfile.NamedTemporaryFile(suffix=".tsv", delete=False) as tmp:
            out = tmp.name
        cmd = [sys.executable, os.path.join(HERE, "scripts", "own_sweep.py"), "--loft", loft,
               "--suite", suite, "--switch", sw, "--out", out]
        for c in cells:
            cmd += ["--cell", c]
        subprocess.run(cmd, cwd=HERE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        with open(out, encoding="utf-8") as fh:
            next(fh, None)
            for line in fh:
                cols = line.rstrip("\n").split("\t")
                if len(cols) >= 5:
                    verdicts[(os.path.relpath(os.path.abspath(cols[4]), suite), cols[1])] = cols[0]
        os.unlink(out)
    return verdicts


def main():
    args = sys.argv[1:]
    if "-h" in args or "--help" in args:
        print(__doc__)
        return 0
    loft = os.path.join(HERE, "target", "release", "loft")
    if "--loft" in args:
        loft = os.path.abspath(args[args.index("--loft") + 1])
    update = "--update" in args
    suite = os.path.abspath(suite_dir())
    todo = pairs(suite)
    now = measure(loft, suite, todo)
    if update:
        with open(BASELINE, "w", encoding="utf-8") as fh:
            fh.write("# Suite cells whose @catches switch the store records do not read as CAUGHT today:\n"
                     "# <cell>\\t<switch>\\t<verdict>.  Written by scripts/catches_records.py --update; it only\n"
                     "# shrinks (a cure whose defect is a stale value, not a store's lifetime, stays here).\n")
            for key in todo:
                v = now.get(key, "NOT-RUN")
                if v != "CAUGHT":
                    fh.write(f"{key[0]}\t{key[1]}\t{v}\n")
        print(f"baseline written: {BASELINE}")
        return 0
    base = read_baseline()
    worse, better = [], []
    for key in todo:
        v = now.get(key, "NOT-RUN")
        was = base.get(key, "CAUGHT")
        if v != "CAUGHT" and was == "CAUGHT":
            worse.append((key, v))
        elif v == "CAUGHT" and was != "CAUGHT":
            better.append(key)
    caught = sum(1 for k in todo if now.get(k) == "CAUGHT")
    print(f"catches_records: {len(todo)} @catches pair(s), {caught} CAUGHT by the store records, "
          f"{len(todo) - caught} listed in the baseline")
    for (cell, sw), v in worse:
        print(f"  NOT CAUGHT  {sw}  {cell}  ({v}) — the records no longer report the defect this cell catches")
    for cell, sw in better:
        print(f"  now CAUGHT  {sw}  {cell} — drop it from {os.path.relpath(BASELINE, HERE)}")
    return 1 if worse or better else 0


if __name__ == "__main__":
    sys.exit(main())
