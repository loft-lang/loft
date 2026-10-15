# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The corpus a sweep walks — the ONE place that says what "every program" means now that the
language suite lives beside this repository (@PLN193).  `scripts/corpus_files.sh` is the same
list for shell; the two must agree.

    from corpus import corpus_files, suite_dir
    for path in corpus_files():            # tests/scripts + tests/docs + the suite
    for path in corpus_files(docs=False):  # what the corpus RUNNERS cover
    for path in corpus_files(examples=True)

`suite_dir()` is `$LOFT_SUITE_DIR`, else `../loft-suite` beside the repository root, resolved
against the root when relative.  An absent checkout contributes nothing and is SAID on stderr
once, so a sweep over 431 programs never reads as the sweep over 2300.
"""
import glob
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
_said = False


def suite_dir():
    d = os.environ.get("LOFT_SUITE_DIR") or os.path.join(ROOT, "..", "loft-suite")
    if not os.path.isabs(d):
        d = os.path.join(ROOT, d)
    return os.path.normpath(d)


def suite_files():
    """Every program of the suite: the rule cells, then the attic.  Empty, and said, without
    the checkout."""
    global _said
    d = suite_dir()
    if not os.path.isdir(os.path.join(d, "rules")):
        if not _said:
            print(f"corpus.py: no language suite at {d} — its programs are NOT in this sweep "
                  "(LOFT_SUITE_DIR)", file=sys.stderr)
            _said = True
        return []
    return sorted(glob.glob(os.path.join(d, "rules", "*", "*", "*.loft"))
                  + glob.glob(os.path.join(d, "attic", "scripts", "*.loft")))


def corpus_files(scripts=True, docs=True, suite=True, examples=False):
    """Absolute paths, sorted within each group: tests/scripts, tests/docs, examples/, the suite."""
    out = []
    if scripts:
        out += sorted(glob.glob(os.path.join(ROOT, "tests", "scripts", "*.loft")))
    if docs:
        out += sorted(glob.glob(os.path.join(ROOT, "tests", "docs", "*.loft")))
    if examples:
        out += sorted(glob.glob(os.path.join(ROOT, "examples", "**", "*.loft"), recursive=True))
    if suite:
        out += suite_files()
    return [p for p in out if os.path.isfile(p)]


def rel(path):
    """`path` relative to the repository root where it is under it (a suite path beside the
    root reads `../loft-suite/...`), else unchanged."""
    try:
        return os.path.relpath(path, ROOT)
    except ValueError:
        return path
