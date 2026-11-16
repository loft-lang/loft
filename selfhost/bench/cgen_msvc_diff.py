#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Diff two C units with the Rust writer's `_MSC_VER` split copy left out of the first.

@PLN91 strand 12.  The Rust writer wraps a unit's thread-local and struct declarations in an
`#if defined(_MSC_VER) … #else … #endif` pair; the loft generator does not write it yet, so
every big unit differs there and nowhere else.  This reads WANT (the Rust unit), keeps only the
`#else` side of each such block, and prints the unified diff against GOT (the loft unit), so
whatever else differs is what is left.

    python3 selfhost/bench/cgen_msvc_diff.py WANT.c GOT.c [LINES]

Prints `N diff lines`, then the first LINES (default 40) of the diff; exit 1 when they differ.
"""
import difflib
import sys


def without_msvc_split(lines):
    out, i = [], 0
    while i < len(lines):
        if (lines[i] == "#if defined(_MSC_VER)" and i + 1 < len(lines)
                and lines[i + 1].startswith(("static LOFT_TLS ", "struct "))):
            # the split copy itself — not the runtime's own `#define LOFT_TLS` block
            j, depth = i + 1, 1
            while j < len(lines):
                if lines[j].startswith("#if"):
                    depth += 1
                if lines[j].startswith("#endif"):
                    depth -= 1
                    if depth == 0:
                        break
                if lines[j] == "#else" and depth == 1:
                    break
                j += 1
            if j < len(lines) and lines[j] == "#else":
                # keep the #else side, without its #endif
                k, depth = j + 1, 1
                while k < len(lines):
                    if lines[k].startswith("#if"):
                        depth += 1
                    if lines[k].startswith("#endif"):
                        depth -= 1
                        if depth == 0:
                            break
                    out.append(lines[k])
                    k += 1
                i = k + 1
                continue
            i = j + 1
            continue
        out.append(lines[i])
        i += 1
    return out


def main():
    if len(sys.argv) < 3 or sys.argv[1] in ("-h", "--help"):
        sys.exit(__doc__)
    with open(sys.argv[1], encoding="utf-8", errors="replace") as fh:
        want = without_msvc_split(fh.read().split("\n"))
    with open(sys.argv[2], encoding="utf-8", errors="replace") as fh:
        got = fh.read().split("\n")
    limit = int(sys.argv[3]) if len(sys.argv) > 3 else 40
    diff = [x for x in difflib.unified_diff(want, got, lineterm="", n=0) if not x.startswith("@@")]
    print(f"{len(diff)} diff lines")
    for line in diff[2:2 + limit]:
        print(line[:200])
    return 1 if diff else 0


if __name__ == "__main__":
    sys.exit(main())
