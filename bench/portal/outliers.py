#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Which slow routines are BUGS, and which belong to a slow CLASS.

    bench/portal/outliers.py <results.tsv>          # the report
    bench/portal/outliers.py <results.tsv> --json   # the same, machine-readable
    bench/portal/outliers.py <results.tsv> --routines <routines.tsv>
                                                    # classes from another registry (a frozen
                                                    # copy pins a historical run's report)

A routine over the bar is an OUTLIER when its mechanism class's median is at or under the
bar — one program is slow where its kind is not, which is an ordinary `performance` issue.  It
is SYSTEMIC when the class median itself is over the bar: the mechanism is slow, and filing its
routines one by one would be noise; the class's plan owns them (PERFORMANCE.md § What makes a
slow routine a bug).  The bar is 3× the Rust twin (C136).

A report, never a gate.  The classes come from routines.tsv, as on the portal page.
"""

import json
import os
import statistics
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from portal import HERE, read_run, read_tsv  # noqa: E402

BAR = 3.0  # @C136


def classify(results_path, routines_path=os.path.join(HERE, "routines.tsv")):
    """Per-class medians and the over-bar routines split SYSTEMIC / OUTLIER, from one run."""
    registry = {(r[0], r[1]): r[2] for r in read_tsv(routines_path, 5)}
    meta, rows = read_run(results_path)
    judged, unclassified = [], []
    for r in rows:
        if not r.get("ratio"):
            continue
        key = (r["bench"], r["routine"])
        if key not in registry:
            unclassified.append(f"{key[0]}/{key[1]}")
            continue
        judged.append(dict(bench=key[0], routine=key[1], cls=registry[key], ratio=float(r["ratio"])))
    by_class = {}
    for r in judged:
        by_class.setdefault(r["cls"], []).append(r["ratio"])
    medians = {c: statistics.median(xs) for c, xs in by_class.items()}
    systemic_classes = sorted((c for c, m in medians.items() if m > BAR), key=lambda c: -medians[c])
    over = sorted((r for r in judged if r["ratio"] > BAR), key=lambda r: -r["ratio"])
    return dict(
        host=meta.get("host", "?"),
        date=meta.get("date", "?"),
        commit=meta.get("commit", "?"),
        bar=BAR,
        routines=len(judged),
        over=len(over),
        medians=medians,
        systemic_classes=systemic_classes,
        systemic=[r for r in over if medians[r["cls"]] > BAR],
        outliers=[r for r in over if medians[r["cls"]] <= BAR],
        unclassified=unclassified,
    )


def report(c):
    out = [f"{c['host']} · {c['date']} · commit {c['commit']} — bar {c['bar']:.0f}× the Rust twin",
           f"{c['routines']} routines, {c['over']} over the bar: "
           f"{len(c['systemic'])} systemic, {len(c['outliers'])} outliers", ""]
    out.append("Classes by median (SYSTEMIC = median over the bar):")
    for cls in sorted(c["medians"], key=lambda k: -c["medians"][k]):
        mark = "SYSTEMIC" if cls in c["systemic_classes"] else ""
        out.append(f"  {c['medians'][cls]:6.2f}×  {cls:<14} {mark}")
    out.append("")
    out.append("OUTLIERS — over the bar in a class that is not (each an ordinary issue):")
    for r in c["outliers"]:
        out.append(f"  {r['ratio']:6.2f}×  {r['bench']}/{r['routine']}  ({r['cls']})")
    if c["unclassified"]:
        out.append("")
        out.append("Not in routines.tsv, so not judged: " + ", ".join(c["unclassified"]))
    return "\n".join(out)


def main(argv):
    if not argv or argv[0] in ("-h", "--help"):
        print(__doc__.strip())
        return 0 if argv else 2
    rest = argv[1:]
    routines = os.path.join(HERE, "routines.tsv")
    if "--routines" in rest:
        routines = rest[rest.index("--routines") + 1]
    c = classify(argv[0], routines)
    print(json.dumps(c, indent=1) if "--json" in rest else report(c))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
