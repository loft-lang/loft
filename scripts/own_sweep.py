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
  MASKED  a switched run whose checker findings are all ones the same cell
          already has with no switch: the fault is new, the checker said
          nothing new — a MISS hidden behind a false finding
  STATIC  only the checker found something (a false finding, or a fault this
          run did not reach)
  CLEAN   neither found anything

A switched run is judged against the cell's no-switch run when the sweep has one
(`--switch ''` beside the others): only its NEW findings answer its faults.

  scripts/own_sweep.py --chapter ownership
  scripts/own_sweep.py --switch '' --switch LOFT_NO_JOIN_OWN --out sweep.tsv
  scripts/own_sweep.py --cell path/to/prog.loft     # any program, not only cells
  scripts/own_sweep.py --oracle check-records       # judge the store records instead

The table goes to --out (TSV); the summary and every MISS and MASKED go to stdout.
"""

import argparse
import collections
import concurrent.futures
import os
import re
import subprocess
import sys
import tempfile

try:  # POSIX only: Windows has no `resource`, and runs without the memory cap
    import resource
except ImportError:
    resource = None
import json

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

FAULTS = [
    ("leak", re.compile(r"store\(s\) not freed at exit|stores not freed at program exit")),
    # A read through a freed store: named by strict stores, or caught by the poisoned
    # record number (`LOFT_POISON` fills a freed record, so its reference reads as corrupt).
    # The strict child-record check (`Stores::ref_from_child_rec`) is the same fault reached
    # through a child's number: under `LOFT_POISON` it can fire before any access report.
    ("uaf", re.compile(r"USE AFTER FREE|use-after-free \(LOFT_UAF\)|the reference is corrupt"
                       r"|strict stores: the child-record field")),
    ("wrong-free", re.compile(r"loft: BUG \(#(?:306|405)\)")),
    # A free of an already-freed store (`LOFT_DOUBLE_FREE`'s census): harmless only while
    # nothing takes the slot in between, so the compiler removes it (STORE_RECORDS.md).
    ("re-free", re.compile(r"\[double-free\] [1-9][0-9]* re-free")),
]
RED = re.compile(r"^RED ", re.M)
# The kind of each checker finding (`RED <fn>: store-<kind> …`).
RED_KIND = re.compile(r"^RED [^:]*: store-([a-z-]+)", re.M)
# One finding's identity across two builds of one program: its line, with the numbers a
# lowering switch can shift (store numbers, compiler temporaries) taken out.
RED_LINE = re.compile(r"^RED .*$", re.M)
RED_VOLATILE = re.compile(r"#\d+|\b(_{1,3}[a-z][a-z0-9]*_)\d+\b")


def red_identity(line):
    return RED_VOLATILE.sub(lambda m: m.group(1) + "N" if m.group(1) else "#N", line)
# Which checker kinds answer a runtime fault kind: a CAUGHT whose findings answer none of the
# run's faults is a coincidence, and `matched` says so.
ANSWERS = {"uaf": {"use-after-end", "wrong-end", "re-free", "double-end"},
           "re-free": {"re-free", "double-end", "wrong-end"},
           "leak": {"leak"},
           "wrong-free": {"wrong-end"}}


def matched(faults, red_kinds):
    """Does every runtime fault kind have a finding of a kind that answers it?"""
    rk = set(filter(None, red_kinds.split(",")))
    return all(ANSWERS.get(f, set()) & rk for f in filter(None, faults.split(",")))
ARMED = re.compile(r"@ARMED:(.*)")
# The checker mode: `check` (the shipped checks) or `check-records` (the store records).
ORACLE = ["check"]
ARMED_SWITCH = re.compile(r"\b(LOFT_[A-Z0-9_]+)=([^\s`.,;)]+)")


def cells_of(suite, chapter):
    base = os.path.join(suite, "rules", chapter) if chapter else os.path.join(suite, "rules")
    out = []
    for d, _, fs in os.walk(base):
        out += [os.path.join(d, f) for f in fs if f.endswith(".loft")]
    return sorted(out)


def cap_memory():
    """Each run gets 8 GB of address space: a runaway analysis fails its own run instead of
    pushing the machine into the out-of-memory killer."""
    if resource is None:
        return
    limit = 8 * 1024 ** 3
    resource.setrlimit(resource.RLIMIT_AS, (limit, limit))


def run(loft, cell, switch, timeout):
    env = dict(os.environ)
    env.update(
        LOFT_TIMEOUT=str(timeout),
        LOFT_LEAK_CHECK="1",
        LOFT_STRICT_STORES="1",
        LOFT_POISON="1",
        LOFT_DOUBLE_FREE="1",
        LOFT_OWN_ORACLE=ORACLE[0],
        LOFT_NO_CACHE="1",
    )
    # The faults as data (`LOFT_FAULT_LOG`, src/fault_log.rs): one JSON line per fault, read
    # by kind.  The text patterns below stay as the fallback for a binary that predates it.
    fd, fault_log = tempfile.mkstemp(prefix="own_sweep_", suffix=".jsonl")
    os.close(fd)
    env["LOFT_FAULT_LOG"] = fault_log
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
                           errors="replace", timeout=timeout + 10, cwd=os.path.dirname(cell),
                           preexec_fn=cap_memory if resource is not None else None)
        out = p.stdout + p.stderr
    except subprocess.TimeoutExpired:
        os.unlink(fault_log)
        return cell, switch, "timeout", 0, "", "", ()
    kinds = [k for k, rx in FAULTS if rx.search(out)]
    for k in logged_kinds(fault_log):
        if k not in kinds:
            kinds.append(k)
    os.unlink(fault_log)
    reds = len(RED.findall(out))
    first_red = next((ln for ln in out.splitlines() if ln.startswith("RED ")), "")
    red_kinds = ",".join(sorted(set(RED_KIND.findall(out))))
    red_lines = tuple(red_identity(ln) for ln in RED_LINE.findall(out))
    return cell, switch, ",".join(kinds), reds, first_red, red_kinds, red_lines


def new_findings(red_lines, baseline):
    """The findings in `red_lines` that `baseline` (the same cell, no switch) lacks."""
    left = collections.Counter(baseline)
    out = []
    for ln in red_lines:
        if left[ln]:
            left[ln] -= 1
        else:
            out.append(ln)
    return out


# A fault-log kind as this sweep names it.
LOGGED = {"use-after-free": "uaf", "corrupt-reference": "uaf", "leak": "leak",
          "wrong-free": "wrong-free", "re-free": "re-free"}


def logged_kinds(path):
    """The fault kinds a run wrote to its `LOFT_FAULT_LOG`, in this sweep's names."""
    kinds = []
    try:
        with open(path, encoding="utf-8", errors="replace") as fh:
            for line in fh:
                try:
                    k = LOGGED.get(json.loads(line).get("kind", ""), "")
                except ValueError:
                    continue
                if k and k not in kinds:
                    kinds.append(k)
    except OSError:
        pass
    return kinds


def compare(before_path, rows):
    """Print every cell × switch whose verdict or red count differs from `before_path`."""
    before = {}
    with open(before_path, encoding="utf-8") as fh:
        for line in fh:
            f = line.rstrip("\n").split("\t")
            if len(f) >= 5 and f[0] != "verdict":
                before[(f[4], f[1])] = (f[0], f[3])
    worse = better = 0
    rank = {"CLEAN": 0, "CAUGHT": 1, "STATIC": 2, "MASKED": 3, "MISS": 3, "TIMEOUT": 4}
    for cell, sw, _kinds, reds, _first, v, _rk in rows:
        key = (os.path.relpath(cell, ROOT), sw)
        old = before.get(key)
        if old is None or old == (v, str(reds)):
            continue
        was = rank.get(old[0], 9), int(old[1] or 0)
        now = rank.get(v, 9), reds
        mark = "better" if now < was else "WORSE"
        worse += mark == "WORSE"
        better += mark == "better"
        print(f"{mark:7} {old[0]}({old[1]}) -> {v}({reds})  {sw or '-'}  {key[0]}")
    print(f"compare: {better} better, {worse} worse against {before_path}")
    return worse


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
    ap.add_argument("--compare", default="",
                    help="a previous --out table: print each run whose verdict or red count changed")
    ap.add_argument("--oracle", default="check", choices=["check", "check-records"],
                    help="the static checker: the shipped checks, or the store records")
    a = ap.parse_args()
    ORACLE[0] = a.oracle
    # Each cell runs from its own directory, so a relative binary path is resolved here.
    a.loft = os.path.abspath(a.loft)
    cells = [os.path.abspath(c) for c in a.cell] or cells_of(os.path.abspath(a.suite), a.chapter)
    switches = a.switch if a.switch is not None else [""]
    jobs = [(c, s) for s in switches for c in cells]
    results = []
    with concurrent.futures.ThreadPoolExecutor(a.jobs) as ex:
        results = list(ex.map(lambda j: run(a.loft, j[0], j[1], a.timeout), jobs))
    # A switched run answers its faults only with findings its cell's no-switch run lacks.
    baseline = {r[0]: r[6] for r in results if r[1] == "" and r[2] != "timeout"}
    rows = []
    for r in results:
        v, rk = verdict(r[2], r[3]), r[5]
        if v == "CAUGHT" and r[1] and r[0] in baseline:
            fresh = new_findings(r[6], baseline[r[0]])
            rk = ",".join(sorted({k for ln in fresh for k in RED_KIND.findall(ln)}))
            if not fresh:
                v = "MASKED"
        rows.append(r[:5] + (v, rk))
    if a.out:
        with open(a.out, "w", encoding="utf-8") as fh:
            fh.write("verdict\tswitch\tfaults\treds\tcell\tfirst_red\tred_kinds\tmatched\n")
            for cell, sw, kinds, reds, first, v, rk in rows:
                m = "" if v != "CAUGHT" else ("yes" if matched(kinds, rk) else "no")
                fh.write(f"{v}\t{sw}\t{kinds}\t{reds}\t{os.path.relpath(cell, ROOT)}\t{first}\t{rk}\t{m}\n")
    tally = {}
    for _c, _s, kinds, _r, _f, v, rk in rows:
        tally[v] = tally.get(v, 0) + 1
        if v == "CAUGHT" and not matched(kinds, rk):
            tally["UNMATCHED"] = tally.get("UNMATCHED", 0) + 1
    print("  ".join(f"{k}={tally[k]}" for k in sorted(tally)) + f"  over {len(rows)} run(s)")
    for cell, sw, kinds, reds, _, v, _rk in rows:
        if v in ("MISS", "MASKED", "TIMEOUT"):
            print(f"{v:7} {sw or '-':28} {kinds:16} {os.path.relpath(cell, ROOT)}")
    worse = compare(a.compare, rows) if a.compare else 0
    return 1 if tally.get("MISS") or tally.get("MASKED") or worse else 0


if __name__ == "__main__":
    sys.exit(main())
