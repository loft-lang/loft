#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Read a formal rule's language-suite cells — READ-ONLY: it reads files and runs nothing, so it
can be allow-listed on its own (the curation pass's writing steps are `suite_curate.py`).

    scripts/suite_cells.py <rule>                 # the cells under rules/<chapter>/<rule>/: receipt,
                                                  # gate E verdict, lines, tests, headline
    scripts/suite_cells.py --head <cell>… [-n N]  # each cell's header comments (license lines off),
                                                  # at most N lines each (default 12)

Receipt kinds: `P` a patch receipt (`@falsified-by:`), `C` a commit receipt (`@falsified-at: <sha>`),
`switch` a switch A/B receipt, `none` a conformance guard (`@falsified-at: none`), `pending`, `-`
no receipt at all (asked whether it CAN fail — TESTING.md § Curating the suite).  The verdict is
gate E's baseline row, or `agree` when the baseline names the cell nowhere.

The suite checkout is `--suite DIR`, else `$LOFT_SUITE_DIR`, else `../loft-suite` beside this
repository.
"""
import glob
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SUITE = os.path.normpath(os.environ.get("LOFT_SUITE_DIR") or os.path.join(os.path.dirname(ROOT), "loft-suite"))
GATE_E = os.path.join(ROOT, "selfhost", "front", "elab_gate.baseline.tsv")


def receipt_kind(text):
    if "@falsified-by:" in text:
        return "P"
    if re.search(r"@falsified-at: *[0-9a-f]{7,} \(switch", text):
        return "switch"
    if re.search(r"@falsified-at: *[0-9a-f]{7,}", text):
        return "C"
    if "@falsified-at: none" in text:
        return "none"
    if "@falsified-at: pending" in text:
        return "pending"
    return "-"


def verdicts():
    out = {}
    if os.path.exists(GATE_E):
        for line in open(GATE_E, encoding="utf-8"):
            f = line.rstrip("\n").split("\t")
            if len(f) == 3 and f[2].startswith("loft-suite/"):
                out[f[2][len("loft-suite/"):]] = f[1]
    return out


def header(text, limit):
    """The comment block above the first code line, the copyright and SPDX lines dropped."""
    lines = []
    for line in text.split("\n"):
        if line.startswith("//"):
            lines.append(line)
        elif line.strip():
            break
    lines = [l for l in lines if "Copyright (c)" not in l and "SPDX-License-Identifier" not in l]
    while lines and lines[0].strip() == "//":
        lines.pop(0)
    return lines[:limit]


def headline(text):
    """The header's first sentence, on one line."""
    words = " ".join(l[2:].strip() for l in header(text, 12) if l.strip() != "//")
    m = re.match(r"(.+?[.!?])(\s|$)", words)
    return (m.group(1) if m else words)[:110]


def list_rule(rule):
    rule = rule.removeprefix("@FR-")
    dirs = glob.glob(os.path.join(SUITE, "rules", "*", rule))
    if not dirs:
        print(f"no rules/<chapter>/{rule}/ under {SUITE}")
        return 1
    v = verdicts()
    for d in dirs:
        rel_dir = os.path.relpath(d, SUITE)
        cells = sorted(glob.glob(os.path.join(d, "*.loft")))
        print(f"{rel_dir}: {len(cells)} cell(s)")
        for c in cells:
            text = open(c, encoding="utf-8", errors="replace").read()
            n_lines = text.count("\n")
            tests = len(re.findall(r"(?m)^fn test_", text))
            verdict = v.get(os.path.relpath(c, SUITE), "agree")
            print(f"  [{receipt_kind(text):<7}] {verdict:<8} {n_lines:>4}L {tests:>2}t  {os.path.basename(c)}")
            print(f"      {headline(text)}")
    return 0


def show_heads(cells, limit):
    for c in cells:
        text = open(c, encoding="utf-8", errors="replace").read()
        print(f"== {os.path.basename(c)}")
        for line in header(text, limit):
            print(line)
    return 0


def main():
    global SUITE
    a = sys.argv[1:]
    # `--suite DIR` in place of `$LOFT_SUITE_DIR`, so the command line starts with the tool's own
    # name (what an allow-list rule matches) rather than with an environment assignment.
    if "--suite" in a:
        i = a.index("--suite")
        SUITE = os.path.normpath(a[i + 1])
        a = a[:i] + a[i + 2:]
    if not a or a[0] in ("-h", "--help"):
        print(__doc__)
        return 0
    if a[0] == "--head":
        limit = 12
        if "-n" in a:
            i = a.index("-n")
            limit = int(a[i + 1])
            a = a[:i] + a[i + 2:]
        return show_heads(a[1:], limit)
    return list_rule(a[0])


if __name__ == "__main__":
    sys.exit(main())
