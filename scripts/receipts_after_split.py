#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Re-derive the patch receipts a file split moved code out from under.

    scripts/receipts_after_split.py <original.rs> <part.rs>…            # every receipt naming it
    scripts/receipts_after_split.py <original.rs> <part.rs>… --only P…  # these patch files only
    scripts/receipts_after_split.py … --dry-run                          # say, write nothing

A split moves code under `tests/falsified/*.patch` receipts that name the ORIGINAL file, so they
stop applying (the receipts job's three-way merge conflicts on a split).  For each receipt that
names the original, each of its hunks there is placed where its context now applies — the
original or one of the parts — by `git apply --check` on a scratch copy, trying the exact
context first, then less (`-C2`, `-C1`, `-C0`), then the split's one allowed edit inside moved
code (`pub(super)` on a method or field whose name has it in the tree).  The receipt is then
regenerated from that placement, with full fresh context, and checked to apply to the tree.
A hunk that lands nowhere is reported for a hand re-derivation; its receipt is left as it was.

A receipt placed with REDUCED context applies but is not yet proof: re-score it with
`scripts/falsify.sh <guard> --patch <file>` (the tool lists them).  RELEASE.md § File split, the
split-file skill.
"""
import glob
import os
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = subprocess.run(["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True).stdout.strip()


def run(args, cwd, inp=None):
    return subprocess.run(args, cwd=cwd, input=inp, capture_output=True, text=True)


def public_names(parts):
    """Names the split made `pub(super)` — methods and fields — read from the tree."""
    names = set()
    for f in parts:
        text = open(os.path.join(ROOT, f), encoding="utf-8").read()
        names |= set(re.findall(r"^\s+pub\(super\) (?:const )?fn ([a-z_0-9]+)", text, re.M))
        names |= set(re.findall(r"^\s+pub\(super\) ([a-z_][a-z_0-9]*):", text, re.M))
    return names


def with_visibility(hunk, names):
    out = []
    for line in hunk.split("\n"):
        if line[:1] in (" ", "-", "+"):
            m = re.match(r"^(\s{4})((?:const )?fn ([a-z_0-9]+)|([a-z_][a-z_0-9]*):)", line[1:])
            if m and (m.group(3) or m.group(4)) in names:
                line = line[0] + m.group(1) + "pub(super) " + line[1 + len(m.group(1)):]
        out.append(line)
    return "\n".join(out)


def rederive(path, original, cands, names, scratch):
    raw = open(path, encoding="utf-8").read()
    pre = ""
    if not raw.startswith(("diff --git", "--- ")):
        i = min(x for x in (raw.find("\ndiff --git "), raw.find("\n--- ")) if x >= 0)
        pre, raw = raw[: i + 1], raw[i + 1:]
    work = tempfile.mkdtemp(dir=scratch)
    touched, unplaced, reduced = set(), [], False
    for sec in re.split(r"(?m)^(?=--- )", raw):
        m = re.match(r"--- (?:[ab]/)?(\S+)\n\+\+\+ (?:[ab]/)?(\S+)\n", sec)
        if not m or "\n@@" not in sec:
            continue
        f = m.group(2)
        sec = re.sub(r"(?m)^diff --git .*\n(?:index .*\n)?\Z", "", sec)
        hunks = [h for h in re.split(r"(?m)^(?=@@ )", sec[sec.find("\n@@") + 1:]) if h]
        targets = cands if f == original else [f]
        for c in targets:
            dst = os.path.join(work, c)
            if not os.path.exists(dst):
                os.makedirs(os.path.dirname(dst), exist_ok=True)
                shutil.copy(os.path.join(ROOT, c), dst)
        for h0 in hunks:
            placed = False
            for h in (h0, with_visibility(h0, names)):
                for ctx in ("3", "2", "1", "0"):
                    for c in targets:
                        if run(["git", "apply", "-C" + ctx, "--recount", "-"], work,
                               f"--- a/{c}\n+++ b/{c}\n" + h).returncode == 0:
                            touched.add(c)
                            reduced |= ctx != "3"
                            placed = True
                            break
                    if placed:
                        break
                if placed:
                    break
            if not placed:
                unplaced.append(h0.split("\n")[0])
    if unplaced:
        return "hand", unplaced, None
    out = pre
    for c in sorted(touched):
        d = run(["git", "diff", "--no-index", "--", os.path.join(ROOT, c), os.path.join(work, c)], ROOT).stdout
        d = re.sub(r"^diff --git .*$", f"diff --git a/{c} b/{c}", d, count=1, flags=re.M)
        d = re.sub(r"^index .*\n", "", d, flags=re.M)
        d = re.sub(r"^--- .*$", f"--- a/{c}", d, count=1, flags=re.M)
        d = re.sub(r"^\+\+\+ .*$", f"+++ b/{c}", d, count=1, flags=re.M)
        out += d
    ok = run(["git", "apply", "--check", "-"], ROOT, out[len(pre):]).returncode == 0
    return ("reduced" if reduced else "exact") if ok else "fail", [], out if ok else None


def main():
    args = sys.argv[1:]
    if len(args) < 2 or args[0] in ("-h", "--help"):
        print(__doc__)
        return 0
    dry = "--dry-run" in args
    args = [a for a in args if a != "--dry-run"]
    only = None
    if "--only" in args:
        i = args.index("--only")
        only, args = args[i + 1:], args[:i]
    original, parts = args[0], args[1:]
    cands = [original] + parts
    names = public_names(cands)
    patches = only or sorted(glob.glob(os.path.join(ROOT, "tests/falsified/*.patch")))
    patches = [p if os.path.isabs(p) else os.path.join(ROOT, p) for p in patches]
    scratch = tempfile.mkdtemp(prefix="receipts_after_split_")
    # A receipt already on the known-stale list went stale before this split; it is that list's
    # question (GUARDS.md § The patch receipt), not this tool's.
    known = set()
    base = os.path.join(ROOT, "tests/falsified_patches.baseline")
    if os.path.exists(base):
        known = {os.path.basename(line.split()[0]) for line in open(base, encoding="utf-8")
                 if line.strip() and not line.startswith("#")}
    stale = [p for p in patches
             if os.path.basename(p) not in known
             and original in open(p, encoding="utf-8").read()
             and run(["git", "apply", "--check", "--3way", "-"], ROOT,
                     re.sub(r"^# regenerate.*\n", "", open(p, encoding="utf-8").read(), flags=re.M)).returncode != 0]
    counts, rescore, hand = {}, [], []
    for p in stale:
        verdict, unplaced, text = rederive(p, original, cands, names, scratch)
        counts[verdict] = counts.get(verdict, 0) + 1
        name = os.path.basename(p)
        if verdict == "hand":
            hand.append(f"{name}: {len(unplaced)} hunk(s) land nowhere ({', '.join(u[:30] for u in unplaced)})")
        elif text is not None:
            if not dry:
                open(p, "w", encoding="utf-8").write(text)
            if verdict == "reduced":
                rescore.append(name)
    shutil.rmtree(scratch, ignore_errors=True)
    print(f"{len(stale)} receipt(s) naming {original} no longer applied: " +
          ", ".join(f"{n} {v}" for v, n in sorted(counts.items())) + (" (dry run)" if dry else ""))
    for r in rescore:
        print(f"  re-score (placed with reduced context): {r}")
    for h in hand:
        print(f"  by hand: {h}")
    return 1 if hand else 0


if __name__ == "__main__":
    sys.exit(main())
