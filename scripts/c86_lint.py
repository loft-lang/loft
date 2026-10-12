#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# c86_lint.py — flag loft code that relies on pre-@C86 vector alias-by-default.
#
# Under loft's @C86 rule (@C86; OWNERSHIP_MODEL.md; regression
# test tests/scripts/503-vector-reference-alias.loft) a whole-value heap bind
# COPIES; `&` is the opt-in for shared mutation.  So this pattern silently loses
# its writes on current loft:
#
#     d = self.data      # plain field bind -> COPIES the vector
#     d[i] = color       # writes the copy; self.data never changes
#
# The fix is `d = &self.data`.  This is the exact shape that broke every Canvas
# drawing method and hex_terrain's world generation.  A plain bind whose write
# happens to reach the store today only does so via last-use copy-elision, which
# is a compiler heuristic, not a guarantee — so it is worth making explicit too.
#
# Detection (function-scoped, high-signal): a local PLAIN-bound from a heap field
# (`x = a.b`, no `&`) that is then INDEX-written (`x[..] =`), FIELD-written
# (`x.f =`), or VECTOR-grown (`x += [..]`).  Scalar `+=` and direct field
# mutation through a `&`-parameter (`m.kinds += ..`) are NOT flagged.
#
# Advisory by design: exits 0 and just reports, so it can be a REPORT-level check
# (lib_audit.sh folds it in).  `--strict` exits 1 when anything is flagged, for a
# standalone pre-commit gate.
#
# Usage:
#   scripts/c86_lint.py <path>...     # files and/or dirs (dirs scanned for *.loft)
#   scripts/c86_lint.py --strict pkg/ # exit 1 if any finding
#
# Known limits (advisory, not a type checker): misses a 2-hop alias
# (`a = o.f; b = a; b[i]=x`) and a bind from a heap-returning call.  The audit's
# per-package test run is the behavioural backstop; this covers the static +
# EMPTY-suite gap.

import os
import re
import sys

# A bind: `name = [&] rhs ;` with an optional trailing comment.
BIND = re.compile(r"^\s*([a-z_]\w*)\s*=\s*(&?)\s*(.+?)\s*;\s*(?://.*)?$")
# Mutations of a bare local that imply a HEAP value (never a scalar):
IDX_WRITE = re.compile(r"^\s*([a-z_]\w*)\s*\[[^\]]*\]\s*=(?!=)")      # x[..] =
FIELD_WRITE = re.compile(r"^\s*([a-z_]\w*)\s*\.[a-z_]\w*\s*=(?!=)")   # x.f  =
VEC_GROW = re.compile(r"^\s*([a-z_]\w*)\s*\+=\s*\[")                  # x += [..]
FN = re.compile(r"^\s*(?:pub\s+)?fn\s+(\w+)")
# A plain field-access chain (`a.b`, `a.b.c`) — a heap read that COPIES under @C86.
# Excludes calls, indexing, and expressions (those are not the aliasing trap).
FIELD_RHS = re.compile(r"^[a-z_]\w*(?:\.[a-z_]\w*)+$")


def scan_file(path):
    """Return a list of (lineno, funcname, local, rhs) C86-lag findings."""
    try:
        with open(path, encoding="utf-8", errors="replace") as fh:
            lines = fh.readlines()
    except OSError:
        return []

    # Split into functions so a bind and its mutation must share a scope.
    funcs, cur = [], ("<top>", [])
    for i, line in enumerate(lines):
        m = FN.match(line)
        if m:
            if cur[1]:
                funcs.append(cur)
            cur = (m.group(1), [])
        cur[1].append((i + 1, line.rstrip("\n")))
    if cur[1]:
        funcs.append(cur)

    findings = []
    for fname, body in funcs:
        binds = {}   # name -> (lineno, rhs)  — plain (no &) field binds only
        mutated = set()
        for ln, line in body:
            mb = BIND.match(line)
            if mb and not IDX_WRITE.match(line) and not FIELD_WRITE.match(line):
                name, amp, rhs = mb.group(1), mb.group(2), mb.group(3).strip()
                if not amp and FIELD_RHS.match(rhs):
                    binds[name] = (ln, rhs)
            for rx in (IDX_WRITE, FIELD_WRITE, VEC_GROW):
                mm = rx.match(line)
                if mm:
                    mutated.add(mm.group(1))
        for name in mutated:
            if name in binds:
                ln, rhs = binds[name]
                findings.append((ln, fname, name, rhs))
    return sorted(findings)


def iter_loft(paths):
    for p in paths:
        if os.path.isdir(p):
            for root, _, files in os.walk(p):
                for f in sorted(files):
                    if f.endswith(".loft"):
                        yield os.path.join(root, f)
        elif p.endswith(".loft"):
            yield p


def main(argv):
    strict = False
    paths = []
    for a in argv:
        if a in ("-h", "--help"):
            print(
                "c86_lint.py — flag loft code relying on pre-C86 vector "
                "alias-by-default.\n\n"
                "Flags a local PLAIN-bound from a heap field (`x = a.b`, no `&`) that is\n"
                "then index-written (`x[..]=`), field-written (`x.f=`), or vector-grown\n"
                "(`x += [..]`) — the write is lost under C86; the fix is `x = &a.b`.\n\n"
                "usage: c86_lint.py [--strict] <path>...   (dirs scanned for *.loft)\n"
                "  --strict   exit 1 when anything is flagged (default: advisory, exit 0)\n"
            )
            return 0
        if a == "--strict":
            strict = True
        else:
            paths.append(a)
    if not paths:
        print("usage: c86_lint.py [--strict] <path>...", file=sys.stderr)
        return 2

    total = 0
    for f in iter_loft(paths):
        for ln, fname, name, rhs in scan_file(f):
            total += 1
            print(f"{f}:{ln}: fn {fname}: `{name} = {rhs}` — plain bind of a heap "
                  f"field then mutated; use `{name} = &{rhs}` (C86)")
    if total:
        print(f"c86_lint: {total} write-through bind(s) may need `&`", file=sys.stderr)
    return 1 if (strict and total) else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
