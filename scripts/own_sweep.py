#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Does the static ownership checker see every store fault the runtime sees?

Runs language-suite cells on the interpreter with the static checker on
(`LOFT_OWN_ORACLE=check`) and the runtime fault detectors armed
(`LOFT_LEAK_CHECK`, `LOFT_STRICT_STORES`, `LOFT_POISON`), once per lowering
switch asked for.  A `LOFT_NO_*` switch turns one of the compiler's ownership
lowerings off, so it re-creates a class of defect on today's build.

Each run gets one verdict:

  MISS    the runtime found a leak, a use after free or a refused wrong free,
          and the checker reported nothing: a hole in the checker
  CAUGHT  both found something
  STATIC  only the checker found something (a false finding, or a fault this
          run did not reach)
  CLEAN   neither found anything

  scripts/own_sweep.py --chapter ownership
  scripts/own_sweep.py --switch '' --switch LOFT_NO_JOIN_OWN --out sweep.tsv
  scripts/own_sweep.py --cell path/to/prog.loft     # any program, not only cells

The table goes to --out (TSV); the summary and every MISS go to stdout.
"""

import argparse
import concurrent.futures
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

FAULTS = [
    ("leak", re.compile(r"store\(s\) not freed at exit|stores not freed at program exit")),
    ("uaf", re.compile(r"USE AFTER FREE|use-after-free \(LOFT_UAF\)")),
    ("wrong-free", re.compile(r"loft: BUG \(#(?:306|405)\)")),
]
RED = re.compile(r"^RED ", re.M)
ARMED = re.compile(r"@ARMED:(.*)")
ARMED_SWITCH = re.compile(r"\b(LOFT_[A-Z0-9_]+)=([^\s`.,;)]+)")


def cells_of(suite, chapter):
    base = os.path.join(suite, "rules", chapter) if chapter else os.path.join(suite, "rules")
    out = []
    for d, _, fs in os.walk(base):
        out += [os.path.join(d, f) for f in fs if f.endswith(".loft")]
    return sorted(out)


def run(loft, cell, switch, timeout):
    env = dict(os.environ)
    env.update(
        LOFT_TIMEOUT=str(timeout),
        LOFT_LEAK_CHECK="1",
        LOFT_STRICT_STORES="1",
        LOFT_POISON="1",
        LOFT_OWN_ORACLE="check",
    )
    try:
        with open(cell, encoding="utf-8", errors="replace") as fh:
            for line in ARMED.findall(fh.read()):
                env.update(ARMED_SWITCH.findall(line))
    except OSError:
        pass
    if switch:
        env[switch] = "1"
    try:
        p = subprocess.run([loft, "--tests", cell], env=env, capture_output=True, text=True,
                           errors="replace", timeout=timeout + 10, cwd=os.path.dirname(cell))
        out = p.stdout + p.stderr
    except subprocess.TimeoutExpired:
        return cell, switch, "timeout", 0, ""
    kinds = [k for k, rx in FAULTS if rx.search(out)]
    reds = len(RED.findall(out))
    first_red = next((ln for ln in out.splitlines() if ln.startswith("RED ")), "")
    return cell, switch, ",".join(kinds), reds, first_red


def verdict(kinds, reds):
    if kinds == "timeout":
        return "TIMEOUT"
    if kinds and reds:
        return "CAUGHT"
    if kinds:
        return "MISS"
    if reds:
        return "STATIC"
    return "CLEAN"


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--loft", default=os.path.join(ROOT, "target", "release", "loft"))
    ap.add_argument("--suite", default=os.environ.get("LOFT_SUITE_DIR", os.path.join(ROOT, "..", "loft-suite")))
    ap.add_argument("--chapter", default="")
    ap.add_argument("--cell", action="append", default=[], help="a program to run (repeatable)")
    ap.add_argument("--switch", action="append", default=None,
                    help="a LOFT_NO_* switch to set ('' = none); repeatable, default: none")
    ap.add_argument("--jobs", type=int, default=max(1, (os.cpu_count() or 2) // 2))
    ap.add_argument("--timeout", type=int, default=60)
    ap.add_argument("--out", default="")
    a = ap.parse_args()
    cells = [os.path.abspath(c) for c in a.cell] or cells_of(os.path.abspath(a.suite), a.chapter)
    switches = a.switch if a.switch is not None else [""]
    jobs = [(c, s) for s in switches for c in cells]
    rows = []
    with concurrent.futures.ThreadPoolExecutor(a.jobs) as ex:
        for r in ex.map(lambda j: run(a.loft, j[0], j[1], a.timeout), jobs):
            rows.append(r + (verdict(r[2], r[3]),))
    if a.out:
        with open(a.out, "w", encoding="utf-8") as fh:
            fh.write("verdict\tswitch\tfaults\treds\tcell\tfirst_red\n")
            for cell, sw, kinds, reds, first, v in rows:
                fh.write(f"{v}\t{sw}\t{kinds}\t{reds}\t{os.path.relpath(cell, ROOT)}\t{first}\n")
    tally = {}
    for *_, v in rows:
        tally[v] = tally.get(v, 0) + 1
    print("  ".join(f"{k}={tally[k]}" for k in sorted(tally)) + f"  over {len(rows)} run(s)")
    for cell, sw, kinds, reds, _, v in rows:
        if v in ("MISS", "TIMEOUT"):
            print(f"{v:7} {sw or '-':28} {kinds:16} {os.path.relpath(cell, ROOT)}")
    return 1 if tally.get("MISS") else 0


if __name__ == "__main__":
    sys.exit(main())
