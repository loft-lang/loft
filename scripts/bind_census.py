#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The lowering census (STORE_RECORDS.md § Phases, 2): which lowering each code generator takes
for the same assignment, tallied over a set of programs.

Every writer names the arm it lowers an assignment with — the interpreter under
`LOFT_TRACE_BIND`, the C writer under `LOFT_TRACE_BIND_C`, the rustc writer under
`LOFT_TRACE_BIND_NATIVE` — one row per assignment, `function⇥variable⇥first|re⇥L<line>⇥… arm`.
This runs each program once per writer, pairs the rows by site (function, variable, source line,
k-th assignment there) and counts the arm pairs. An interpreter arm met with more than one C arm, or
the other way round, is a place where the two writers decide differently for what the IR says is the
same assignment — where a store-lifetime defect of one backend lives.

    scripts/bind_census.py --loft target/debug/loft --suite ../loft-suite [--jobs 8]
        [--writer c|native] [--out census.tsv] [--cells file-with-paths]

Prints the pair table, most frequent first, and the disagreeing interpreter arms; `--out`
writes every paired row (cell, function, variable, L<line>#k, interpreter arm, writer arm).
"""

import argparse
import collections
import os
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor

WRITER_ENV = {"c": ("--native-c", "LOFT_TRACE_BIND_C"), "native": ("--native", "LOFT_TRACE_BIND_NATIVE")}


def rows(path):
    """(function, variable) -> [(first|re, line, arm)] in order, from one trace file.

    A row is `function⇥variable⇥first|re⇥L<line>⇥facts… arm`; the arm is the last word, and an
    interpreter rebind's `b2=` fact (the displaced free) is part of its lowering."""
    out = collections.defaultdict(list)
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            for line in f:
                parts = line.rstrip("\n").split("\t")
                if len(parts) < 5:
                    continue
                fn, var, kind, site, arm = parts[0], parts[1], parts[2], parts[3], parts[4]
                words = arm.split(" ")
                b2 = next((w for w in words if w.startswith("b2=") and "/" not in w), None)
                arm = words[-1] + (f"/{b2}" if b2 else "")
                out[(fn, var)].append((kind, site, arm))
    except OSError:
        pass
    return out


def by_site(seq):
    """The rows of one variable grouped by source line, each group in emission order."""
    groups = collections.defaultdict(list)
    for kind, site, arm in seq:
        groups[site].append((kind, arm))
    return groups


def run(loft, cell, mode, env_name, trace):
    env = dict(os.environ, LOFT_NO_CACHE="1", LOFT_TIMEOUT="200", LOFT_ALLOW_REFREE="1")
    env[env_name] = trace
    try:
        subprocess.run(
            [loft, mode, "--tests", os.path.basename(cell)],
            cwd=os.path.dirname(cell) or ".",
            env=env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=400,
            check=False,
        )
    except subprocess.TimeoutExpired:
        pass


def census(loft, cell, writer, tmp):
    mode, env_name = WRITER_ENV[writer]
    base = os.path.join(tmp, str(abs(hash(cell))))
    ti, tw = base + ".i", base + "." + writer
    run(loft, cell, "--interpret", "LOFT_TRACE_BIND", ti)
    run(loft, cell, mode, env_name, tw)
    ri, rw = rows(ti), rows(tw)
    paired = []
    # Rows pair by site — the same variable on the same source line, the k-th there on each
    # side — and only as many as the interpreter wrote: a writer that emits one assignment
    # more than once (an unrolled loop, a guarded twin) repeats its rows for that site.
    for key, wseq in rw.items():
        iseq = ri.get(key)
        if not iseq:
            continue
        isites, wsites = by_site(iseq), by_site(wseq)
        for site, irows in isites.items():
            for k, ((ik, ia), (wk, wa)) in enumerate(zip(irows, wsites.get(site, []))):
                if ik != wk:
                    break
                paired.append((cell, key[0], key[1], f"{site}#{k}", ik, ia, wa))
    return paired


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--loft", required=True)
    ap.add_argument("--suite")
    ap.add_argument("--cells")
    ap.add_argument("--writer", default="c", choices=sorted(WRITER_ENV))
    ap.add_argument("--jobs", type=int, default=8)
    ap.add_argument("--out")
    ap.add_argument("--examples", type=int, default=0, help="cells to list per split pair")
    a = ap.parse_args()
    loft = os.path.abspath(a.loft)
    if a.cells:
        with open(a.cells, encoding="utf-8") as f:
            cells = [os.path.abspath(l.strip()) for l in f if l.strip()]
    else:
        cells = sorted(
            os.path.abspath(os.path.join(d, n))
            for d, _, fs in os.walk(os.path.join(a.suite, "rules"))
            for n in fs
            if n.endswith(".loft")
        )
    with tempfile.TemporaryDirectory() as tmp, ThreadPoolExecutor(a.jobs) as pool:
        results = list(pool.map(lambda c: census(loft, c, a.writer, tmp), cells))
    allrows = [r for rs in results for r in rs]
    pairs = collections.Counter((r[4], r[5], r[6]) for r in allrows)
    by_interp = collections.defaultdict(collections.Counter)
    for (kind, ia, wa), n in pairs.items():
        by_interp[(kind, ia)][wa] += n
    print(f"{len(allrows)} paired assignments over {len(cells)} programs (writer: {a.writer})")
    print("\nkind\tinterpreter\twriter\tcount")
    for (kind, ia, wa), n in pairs.most_common():
        print(f"{kind}\t{ia}\t{wa}\t{n}")
    split = {k: v for k, v in by_interp.items() if len(v) > 1}
    print(f"\n{len(split)} interpreter arm(s) the writer lowers more than one way:")
    for (kind, ia), c in sorted(split.items(), key=lambda kv: -sum(kv[1].values())):
        print(f"  {kind} {ia}: " + ", ".join(f"{w}×{n}" for w, n in c.most_common()))
    if a.examples:
        ex = collections.defaultdict(list)
        for r in allrows:
            ex[(r[4], r[5], r[6])].append(r)
        print("\nexamples of each pair met under a split interpreter arm:")
        for (kind, ia), c in sorted(split.items(), key=lambda kv: -sum(kv[1].values())):
            for wa, _ in c.most_common():
                for r in ex[(kind, ia, wa)][: a.examples]:
                    print(f"  {kind} {ia} -> {wa}: {os.path.relpath(r[0])} {r[1]} {r[2]} {r[3]}")
    if a.out:
        with open(a.out, "w", encoding="utf-8") as f:
            for r in allrows:
                f.write("\t".join(str(x) for x in r) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
