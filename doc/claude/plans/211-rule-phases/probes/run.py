#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""@PLN211 B2's probes: what a rule moved into the IR would save the INTERPRETER.

    python3 doc/claude/plans/211-rule-phases/probes/run.py            # every case
    python3 doc/claude/plans/211-rule-phases/probes/run.py push_fill  # some cases

Each case is a directory with `before.loft` (the shape as a program writes it, which a
compiled writer rewrites) and `after.loft` (the same work written by hand in the form the IR
rule would leave), both taking the iteration count as their argument and printing a checksum.
The header of `before.loft` names the census rule the case models and the two counts:

    // probe-rule: R-PushFill
    // probe-n: 20000 60000
    // probe-fn: n_main          (the function the rule fires in; n_main when absent)

The runner checks the case before it measures it: the two forms print the same at each count,
the rule is ADMITTED in the case's function for `before.loft` by a compiled writer
(`LOFT_REWRITE_CENSUS_FN`, the C and the rustc emission), and in that function for
`after.loft` by neither — so the hand form is the shape the rule produces.  The standard
library's own admissions are left out: they are the same in both forms.  Then it counts user-space instructions on the interpreter, pinned to one
core, at both counts: (I(n2) - I(n1)) / (n2 - n1) is the cost of one iteration with start-up
removed.  Instructions are deterministic (repeat runs agree to 0.01 %), so no timing is taken
and no layout pin is needed: one binary runs both forms.
"""

import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[4]
LOFT = os.environ.get("LOFT_BIN", str(ROOT / "target" / "release" / "loft"))
CORE = os.environ.get("PROBE_CORE", "2")


def header(path, key):
    m = re.search(rf"//\s*probe-{key}:\s*(.+)", path.read_text())
    return m.group(1).strip() if m else None


def instructions(src, n):
    p = subprocess.run(["taskset", "-c", CORE, "perf", "stat", "-x,", "-e", "instructions:u",
                        LOFT, "--interpret", str(src), str(n)],
                       capture_output=True, text=True, timeout=600)
    count = next((int(line.split(",")[0]) for line in p.stderr.splitlines()
                  if "cpu_core/instructions" in line and line.split(",")[0].isdigit()), None)
    if count is None:  # a machine without hybrid cores names the counter plainly
        count = next((int(line.split(",")[0]) for line in p.stderr.splitlines()
                      if "instructions" in line and line.split(",")[0].isdigit()), None)
    return count, p.stdout.strip(), p.returncode


def admitted(src, rule, fn):
    """{writer: admissions of `rule` (its name exactly) in function `fn` when `src` is emitted}."""
    out = {}
    for writer, flags in (("c", ["--emit-c"]), ("rust", ["--native-emit"])):
        with tempfile.TemporaryDirectory() as tmp:
            tsv = Path(tmp) / "census.tsv"
            target = Path(tmp) / ("o.c" if writer == "c" else "o.rs")
            extra = [] if writer == "c" else ["--lean"]
            subprocess.run([LOFT, *flags, str(target), *extra, str(src)], capture_output=True,
                           timeout=300, env={**os.environ, "LOFT_REWRITE_CENSUS_FN": str(tsv)})
            rows = [line.split("\t") for line in tsv.read_text().splitlines()] if tsv.exists() else []
            out[writer] = sum(int(n) for _phase, f, r, n in rows if r == rule and f == fn)
    return out


def case(d):
    before, after = d / "before.loft", d / "after.loft"
    rule = header(before, "rule")
    fn = header(before, "fn") or "n_main"
    n1, n2 = (int(x) for x in header(before, "n").split())
    notes = []
    adm_b, adm_a = admitted(before, rule, fn), admitted(after, rule, fn)
    if not any(adm_b.values()):
        notes.append(f"{rule} is admitted in {fn} by neither writer on before.loft: the probe does not model it")
    if any(adm_a.values()):
        notes.append(f"{rule} is still admitted in {fn} on after.loft ({adm_a}): the hand form is not the rewritten shape")
    cost = {}
    for name, src in (("before", before), ("after", after)):
        got = [instructions(src, n) for n in (n1, n2)]
        if any(rc != 0 or c is None for c, _, rc in got):
            return rule, None, None, adm_b, [f"{name}.loft failed to run"] + notes
        cost[name] = ((got[1][0] - got[0][0]) / (n2 - n1), [o for _, o, _ in got])
    if cost["before"][1] != cost["after"][1]:
        notes.append(f"the answers differ: {cost['before'][1]} against {cost['after'][1]}")
    return rule, cost["before"][0], cost["after"][0], adm_b, notes


def main():
    only = sys.argv[1:]
    cases = sorted(d for d in HERE.iterdir() if (d / "before.loft").is_file() and (d / "after.loft").is_file()
                   and (not only or d.name in only))
    print(f"{'case':<14} {'rule':<22} {'before':>9} {'after':>9} {'change':>8}  admitted (c, rust)")
    bad = 0
    for d in cases:
        rule, b, a, adm, notes = case(d)
        if b is None:
            print(f"{d.name:<14} {rule:<22} {'-':>9} {'-':>9} {'-':>8}  {adm.get('c')}, {adm.get('rust')}")
        else:
            print(f"{d.name:<14} {rule:<22} {b:>9.1f} {a:>9.1f} {100 * (a - b) / b:>7.1f}%  "
                  f"{adm.get('c')}, {adm.get('rust')}")
        for note in notes:
            print(f"  ! {note}")
        bad += bool(notes)
    print("instructions per iteration on --interpret (user space, one core)")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
