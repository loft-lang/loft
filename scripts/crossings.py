#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The rule-crossing ledger: which PAIRS of formal rules have already been verified together.

    python3 scripts/crossings.py list      every verified pair, one line each
    python3 scripts/crossings.py check     every row names two defined rules and a known verdict
                                           (exit 1 on a bad row)

The ledger is the table in doc/claude/RULE_CROSSINGS-history.md (doc/claude/RULE_CROSSINGS.md
says how a row is earned).  `rule_tags.py pairs` reads it through `verified_pairs()` and leaves
those pairs out of its queue, so a crossing that was built, hand-computed and found to hold on
both backends is not built again by the next walk.

A row counts when its verdict is `holds` or `fixed loft#N` (the composition was exercised and
answers what the rules say) or `disjoint — <why>` (no program can make both rules act at once,
and the row says which decline keeps them apart).  A row whose verdict is anything else
(`open loft#N`) is listed but NOT skipped — the pair still owes work.
"""

import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LEDGER = os.path.join(ROOT, "doc", "claude", "RULE_CROSSINGS-history.md")
HEADER = ["Rule A", "Rule B", "Verdict", "Validated", "How", "Axes crossed", "Guard"]
SKIP_VERDICT = re.compile(r"^(holds|fixed loft#\d+|disjoint\b)")
ANY_VERDICT = re.compile(r"^(holds|fixed loft#\d+|open loft#\d+|disjoint\b)")
# A `disjoint` verdict must say WHY: the decline that keeps the two rules apart.
DISJOINT_REASON = re.compile(r"^disjoint\s+[—-]+\s+\S")


def _tag(cell):
    return cell.strip().strip("`").removeprefix("@FR-").removeprefix("@")


def rows():
    """Every ledger row as a dict keyed by HEADER, in file order."""
    if not os.path.exists(LEDGER):
        return []
    out = []
    head = None
    with open(LEDGER, encoding="utf-8") as fh:
        for line in fh:
            if not line.startswith("|"):
                head = None if not line.strip() else head
                continue
            cells = [c.strip() for c in line.strip().strip("|").split("|")]
            if cells[:2] == HEADER[:2]:
                head = cells
                continue
            if head is None or set("".join(cells)) <= set("-: "):
                continue
            row = dict(zip(head, cells))
            row["A"], row["B"] = _tag(row.get("Rule A", "")), _tag(row.get("Rule B", ""))
            out.append(row)
    return out


def verified_pairs():
    """The (a, b) pairs, sorted and without the `@FR-` prefix, a walk may skip."""
    return {tuple(sorted((r["A"], r["B"]))) for r in rows()
            if SKIP_VERDICT.match(r.get("Verdict", ""))}


def main():
    cmd = sys.argv[1] if len(sys.argv) > 1 else "list"
    if cmd == "list":
        for r in rows():
            print(f"@FR-{r['A']:<22} @FR-{r['B']:<22} {r.get('Verdict', '')}  "
                  f"({r.get('Validated', '')})")
        return 0
    if cmd == "check":
        sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
        from rule_tags import defined_rules
        rules = defined_rules()
        bad = 0
        for r in rows():
            for t in (r["A"], r["B"]):
                if t not in rules:
                    print(f"crossings: @FR-{t} is not a defined rule", file=sys.stderr)
                    bad += 1
            verdict = r.get("Verdict", "")
            if not ANY_VERDICT.match(verdict):
                print(f"crossings: @FR-{r['A']} x @FR-{r['B']}: verdict "
                      f"'{verdict}' is not holds / fixed loft#N / open loft#N / disjoint",
                      file=sys.stderr)
                bad += 1
            elif verdict.startswith("disjoint") and not DISJOINT_REASON.match(verdict):
                print(f"crossings: @FR-{r['A']} x @FR-{r['B']}: a disjoint verdict names "
                      f"the decline that keeps the rules apart (`disjoint — <why>`)",
                      file=sys.stderr)
                bad += 1
        return 1 if bad else 0
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main())
