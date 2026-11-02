#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""hand_forms.py — keep a hand-priced routine's forms in a plan directory (@PLN210).

A hand pricing (bench/portal/hand_price_c.sh) edits the emitted C unit step by step.  The
units are about a megabyte each and come back from `loft --emit-c` at the recorded commit,
so only what a person wrote is kept: each edited unit as a diff against its base, the step
scripts, the driver cut (bench.loft / bench.rs as diffs against the library's bench), and
the write-up.

  hand_forms.py export WORK_DIR PLAN_HAND_DIR LIBRARY_PKG_DIR [--commit SHA]
      WORK_DIR         the pricing directory (base*.c, step edits, *.py, RESULT.md, pkg*/)
      PLAN_HAND_DIR    doc/claude/plans/210-…/hand/<routine>
      LIBRARY_PKG_DIR  the library package the driver was cut from

  hand_forms.py restore PLAN_HAND_DIR BASE_DIR OUT_DIR
      re-applies every diff to the base unit its header names, found in BASE_DIR (emitted
      again, under the names FORMS.txt lists); a diff that no longer applies names the form
      that wants re-basing.
"""
import difflib
import os
import shutil
import subprocess
import sys

KEEP_SUFFIXES = (".py", ".sh", ".patch", ".md", ".txt", ".rs")
SKIP_PREFIXES = ("ann_", "census", "links", "nat")


def is_base(name):
    stem = name[:-2]
    return stem.startswith("base") or stem.endswith("base") or stem in ("b0",)


def read(path):
    with open(path, encoding="utf-8", errors="replace") as f:
        return f.readlines()


def best_base(unit, bases):
    """The base the edit started from: the one with the smallest diff."""
    lines = read(unit)
    best = None
    for b in bases:
        size = sum(1 for d in difflib.unified_diff(read(b), lines, n=0)
                   if d.startswith(("+", "-")))
        if best is None or size < best[0]:
            best = (size, b)
    return best[1]


def export(work, out, pkg, commit):
    os.makedirs(out, exist_ok=True)
    units = sorted(f for f in os.listdir(work) if f.endswith(".c"))
    bases = [os.path.join(work, f) for f in units if is_base(f)]
    if not bases:
        sys.exit(f"{work}: no base*.c to diff against")
    manifest = []
    seen = {}
    for f in units:
        if is_base(f):
            manifest.append(f"base  {f}")
            continue
        src = os.path.join(work, f)
        base = best_base(src, bases)
        diff = "".join(difflib.unified_diff(read(base), read(src),
                                            fromfile=os.path.basename(base), tofile=f, n=3))
        if not diff:
            manifest.append(f"same  {f}  as {os.path.basename(base)}")
            continue
        # A unit that is no edit of any base (an excerpt, another driver's unit) is scratch.
        if len(diff) > os.path.getsize(src) // 10:
            manifest.append(f"skip  {f}  (no edit of a kept base)")
            continue
        body = diff.split("\n", 2)[2] if diff.count("\n") > 2 else diff
        key = (base, body)
        if key in seen:
            manifest.append(f"same  {f}  as {seen[key]}")
            continue
        seen[key] = f
        with open(os.path.join(out, f + ".diff"), "w", encoding="utf-8") as o:
            o.write(diff)
        manifest.append(f"edit  {f}  from {os.path.basename(base)}")
    for f in sorted(os.listdir(work)):
        p = os.path.join(work, f)
        if os.path.isfile(p) and f.endswith(KEEP_SUFFIXES) and not f.startswith(SKIP_PREFIXES) \
                and os.path.getsize(p) < 200_000:
            # A step script is a record of the pricing, not a tool of this repo: kept as text.
            name = f + ".txt" if f.endswith((".py", ".sh")) else f
            shutil.copy2(p, os.path.join(out, name))
    for d in sorted(os.listdir(work)):
        drv = os.path.join(work, d, "bench")
        if not (d.startswith("pkg") and os.path.isdir(drv)):
            continue
        for b in ("bench.loft", "bench.rs"):
            mine, theirs = os.path.join(drv, b), os.path.join(pkg, "bench", b)
            if os.path.exists(mine) and os.path.exists(theirs):
                diff = list(difflib.unified_diff(read(theirs), read(mine),
                                                 fromfile=f"library/bench/{b}",
                                                 tofile=f"{d}/bench/{b}", n=2))
                if diff:
                    path = os.path.join(out, f"driver-{d}-{b}.diff")
                    with open(path, "w", encoding="utf-8") as o:
                        o.writelines(diff)
    lib_sha = subprocess.run(["git", "-C", pkg, "rev-parse", "--short", "HEAD"],
                             capture_output=True, text=True).stdout.strip()
    with open(os.path.join(out, "FORMS.txt"), "w", encoding="utf-8") as o:
        o.write(f"loft commit   {commit}\n")
        o.write(f"library       {os.path.basename(os.path.normpath(pkg))} @ {lib_sha}\n")
        o.write("emit          LOFT_RELEASE_PASS_PROBE=1 loft --emit-c base.c bench/bench.loft"
                " (in the driver package)\n\n")
        o.write("\n".join(manifest) + "\n")


def restore(hand, base_dir, out):
    """Apply each form to the base its header names (`--- base.c`), found in BASE_DIR."""
    os.makedirs(out, exist_ok=True)
    bad = 0
    for f in sorted(os.listdir(hand)):
        if not f.endswith(".c.diff"):
            continue
        with open(os.path.join(hand, f), encoding="utf-8") as d:
            base = d.readline()[4:].strip()
        target = os.path.join(out, f[:-5])
        r = subprocess.run(["patch", "-s", "-o", target, os.path.join(base_dir, base),
                            os.path.join(hand, f)], capture_output=True, text=True)
        if r.returncode != 0:
            bad += 1
            print(f"{f}: no longer applies to {base} — re-base this form\n{r.stdout}{r.stderr}")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    if len(sys.argv) == 2 and sys.argv[1] in ("-h", "--help"):
        print(__doc__)
        sys.exit(0)
    if len(sys.argv) >= 5 and sys.argv[1] == "export":
        commit = sys.argv[sys.argv.index("--commit") + 1] if "--commit" in sys.argv else "?"
        export(sys.argv[2], sys.argv[3], sys.argv[4], commit)
    elif len(sys.argv) == 5 and sys.argv[1] == "restore":
        restore(sys.argv[2], sys.argv[3], sys.argv[4])
    else:
        sys.exit(__doc__)
