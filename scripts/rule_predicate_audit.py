#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# rule_predicate_audit.py — where is one RULE spelled as a type-list more than once?
#
# Many rules in doc/claude/formal/ are implemented as a membership test over `Type`
# variants: "is this a scalar", "is this a keyed collection", "does this own a store".
# Written inline at each site, the copies drift — which is what loft#1006 was, and what
# `ref_tuple_element_ok` / `tuple_carries_fn_ref` / `write_absent_value` were each created
# to stop.  This finds the lists that are still spelled more than once, and the pairs that
# differ by exactly ONE variant (the drift that is already there).
#
# A REPORT, never a gate: some repeats are genuinely different questions that happen to
# share a list today, and merging those would couple two rules that must be free to differ.
# The checklist of verdicts is doc/claude/formal/IMPLEMENTATIONS.md.
#
# Usage:  python3 scripts/rule_predicate_audit.py [--near] [--min N]

import collections
import glob
import itertools
import os
import re
import sys

ROOT = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "src")
MATCHES = re.compile(r"matches!\s*\(([^;]{0,600}?)\)\s*(?:\{|,|;|\)|$)", re.S)
# `Type` is the surface type; `Parts` is the LAYOUT view of it (the narrow integer widths
# live there, not in `Type`).  A rule can be spelled over either, so both are scanned —
# reading only `Type::` made the whole narrow-width family invisible to this instrument.
VARIANT = re.compile(r"\b(?:Type|Parts)::([A-Za-z_][A-Za-z0-9_]*)")


def collect(min_len):
    sites = collections.defaultdict(list)
    # Sorted: glob answers in readdir order, so the same tree printed sites in a
    # different order on every filesystem — a report nobody can diff.
    for path in sorted(glob.glob(ROOT + "/**/*.rs", recursive=True)):
        src = open(path, encoding="utf-8", errors="replace").read()
        for m in MATCHES.finditer(src):
            names = frozenset(VARIANT.findall(m.group(1)))
            if len(names) >= min_len:
                line = src[: m.start()].count("\n") + 1
                sites[names].append((os.path.relpath(path, ROOT), line))
    return sites


def main():
    near = "--near" in sys.argv
    min_len = 3
    if "--min" in sys.argv:
        min_len = int(sys.argv[sys.argv.index("--min") + 1])
    sites = collect(min_len)

    if not near:
        print(f"{len(sites)} distinct Type:: lists of {min_len}+ variants\n")
        for names, where in sorted(sites.items(), key=lambda kv: -len(kv[1])):
            if len(where) < 2:
                continue
            print(f"[{len(where):2d}x] {' | '.join(sorted(names))}")
            for s, ln in where[:6]:
                print(f"       {s}:{ln}")
            if len(where) > 6:
                print(f"       … and {len(where) - 6} more")
        return 0

    print("=== lists differing by exactly ONE variant — drift already present ===")
    seen = set()
    for a, b in itertools.combinations(list(sites), 2):
        diff = a ^ b
        if len(diff) != 1 or min(len(a), len(b)) < min_len:
            continue
        key = tuple(sorted([tuple(sorted(a)), tuple(sorted(b))]))
        if key in seen:
            continue
        seen.add(key)
        big, small = (a, b) if len(a) > len(b) else (b, a)
        print(f"\n  differ by: {next(iter(diff))}")
        for label, s in (("WITH   ", big), ("WITHOUT", small)):
            print(f"    {label} ({len(sites[s])}x): {' | '.join(sorted(s))}")
            for f, ln in sites[s][:3]:
                print(f"             {f}:{ln}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
