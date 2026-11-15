#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""`tests/scripts/` keeps only the programs this repository NAMES — the gate and its ratchet.

TESTING.md § `tests/scripts/`: the comprehensive language suite is `loft-lang/loft-suite`, one
cell per program under the formal rule it cites; a script stays in this repository only when
something here names it — a Rust test, a script, a Makefile target, a workflow, the worked-
example registry (`examples-index.tsv`), or its own `@ARGS --lib` / `--project` line that needs
this tree.  Everything else is a cell.  Measured 2026-11-15: of the 189 scripts added since the
suite became its own repository, 144 were named by nothing but their falsification receipt or a
plan page, and 111 of those cite a rule — cells by the suite's own definition.

    scripts/scripts_named.py list [--at REF]   every tests/scripts/*.loft nothing names (REF: a
                                               commit; default the working tree), one per line
    scripts/scripts_named.py check             exit 1 on a script not in the baseline (name it
                                               here, or make it a cell), or a baseline line
                                               whose script is named now or gone (the ratchet
                                               only shrinks — delete the line)
    scripts/scripts_named.py bless             rewrite tests/scripts_unpinned.baseline from `list`

What counts as NAMING: an occurrence of the file's basename in a tracked file of the kinds that
can pin it (below).  A receipt in `tests/falsified/`, a dump, a plan page and prose do not pin:
they are written FROM the script, not reasons for it to live here.  The function `unpinned(ref)`
is what `scripts/stability_read.py` reads for its weekly column.
"""
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SCRIPTS = "tests/scripts"
BASELINE = "tests/scripts_unpinned.baseline"
# The files whose mention of a script keeps it here.  Listed, not "everything but", because a
# grep over the whole tree (the rendered doc pages are large) takes minutes; these take seconds.
PINNERS = ["tests/*.rs", "tests/common", "tests/golden", "tests/fixtures", "tests/lib",
           "tests/docs", "tests/comparisons", "tests/c_split", "scripts", "Makefile", ".github",
           "examples-index.tsv", "src", "bench", "selfhost", "default", "lib", "tools",
           "Cargo.toml", "loft.toml"]
NAME = r"[A-Za-z0-9][A-Za-z0-9_.-]*\.loft"
SELF_PIN = re.compile(r"^//\s*@ARGS\b.*--(lib|project)\b", re.M)


def git(*args):
    r = subprocess.run(["git", *args], cwd=ROOT, capture_output=True, text=True)
    return r.stdout


def scripts(ref):
    # The working tree includes a file not yet added, so a probe written a minute ago is seen.
    out = (git("ls-tree", "-r", "--name-only", ref, SCRIPTS) if ref
           else git("ls-files", "--cached", "--others", "--exclude-standard", SCRIPTS))
    return sorted(p for p in out.split("\n") if p.endswith(".loft"))


def named(ref):
    # Options before the pattern: git grep reads one after it as a revision or a path.
    args = ["grep", "-ohIE"] + ([] if ref else ["--untracked"]) + [NAME]  # a pinning file
    if ref:                                                              # written a minute ago
        args.append(ref)                                                 # counts too
    out = git(*args, "--", *PINNERS)
    return set(out.split())


def head(path, ref):
    text = git("show", f"{ref}:{path}") if ref else open(os.path.join(ROOT, path), encoding="utf-8",
                                                           errors="replace").read()
    return text[:2000]


def unpinned(ref=None):
    """The scripts nothing names at `ref` (a commit) or in the working tree (None)."""
    names = named(ref)
    out = []
    for p in scripts(ref):
        if os.path.basename(p) in names:
            continue
        if SELF_PIN.search(head(p, ref)):
            continue  # its @ARGS line reaches this tree: it cannot leave
        out.append(p)
    return out


def rule_of(path):
    m = re.search(r"@FR-([A-Za-z0-9-]+)", open(os.path.join(ROOT, path), encoding="utf-8",
                                                errors="replace").read())
    return m.group(1) if m else None


def read_baseline():
    try:
        lines = open(os.path.join(ROOT, BASELINE), encoding="utf-8").read().split("\n")
    except FileNotFoundError:
        return set()
    return {l.strip() for l in lines if l.strip() and not l.startswith("#")}


def main(argv):
    cmd = argv[1] if len(argv) > 1 else "check"
    ref = argv[argv.index("--at") + 1] if "--at" in argv else None
    if cmd == "list":
        print("\n".join(unpinned(ref)))
        return 0
    if cmd == "bless":
        files = unpinned(None)
        with open(os.path.join(ROOT, BASELINE), "w", encoding="utf-8") as f:
            f.write("# tests/scripts/ files nothing in this repository names — the backlog to move into the\n"
                    "# language suite as cells (TESTING.md § tests/scripts/).  A RATCHET: it only shrinks.\n"
                    "# `scripts/scripts_named.py check` gates it; `bless` rewrites it after a move.\n")
            f.write("\n".join(files) + ("\n" if files else ""))
        print(f"{len(files)} unpinned script(s) recorded in {BASELINE}")
        return 0
    if cmd != "check":
        sys.exit(__doc__)
    now, base = set(unpinned(None)), read_baseline()
    added, gone = sorted(now - base), sorted(base - now)
    for p in added:
        rule = rule_of(p)
        where = (f"a cell under @FR-{rule} in the language suite (rules/<chapter>/{rule}/, "
                 f"`make suite ARGS=\"@FR-{rule}\"`)" if rule
                 else "a cell under the rule it exercises in the language suite")
        print(f"{p}\tnothing in this repository names it — make it {where}, or name it here")
    for p in gone:
        print(f"{p}\tnamed now, moved or gone — delete its line from {BASELINE} (the ratchet only shrinks)")
    return 1 if added or gone else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
