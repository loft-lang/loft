#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""How THOROUGHLY the tests match each formal rule — depth, where `rule_tags.py coverage` reads
presence.

    scripts/rule_depth.py                  # the summary and the shallowest guarded rules, by yield
    scripts/rule_depth.py --all            # every guarded rule
    scripts/rule_depth.py --family H       # one family (the tag's prefix up to the first dash)
    scripts/rule_depth.py --rule B-Copy    # one rule in full: cells, axes with their gaps, receipts, sites
    scripts/rule_depth.py --tsv            # the table as TSV, every guarded rule
    scripts/rule_depth.py selftest

Why
---
`coverage` counts a rule NAMED by a test and says so: the weaker claim.  Once nearly every rule
has a citing cell (97 % with the language suite beside the tree, 2026-10-21), the question the
freeze still owes is whether those cells would FAIL were the rule broken, and how much of what
the rule promises they reach.  A rule is a quantified statement — every value of some kinds, in
some contexts, on both backends, answers so — and one cell at one point of that domain is a
citation, not a verification.

How — four columns per rule, none a gate
----------------------------------------
cells     the files that cite the rule from a guard position: `.loft` cells (the language
          suite beside the tree, `tests/scripts`, `tests/docs`), `.rs` tests, and `#[cfg(test)]`
          modules of the source — what `rule_tags.guard_citations` collects.  A `.loft` cell
          runs on both backends by the runners' default; an `.rs` test counts as both only when
          its source spawns both `--interpret` and `--native`.
defects   the distinct `loft#N` issues the cells name (a cell written from a defect is one that
          failed once; a cell written from the rule's text has never been seen to fail).
receipt   a falsification: a `@falsified-at: <sha>` receipt in a cell (`none` and `INERT` do not
          count — they say the file CANNOT move a channel), a `tests/falsified/<cell>.patch`
          control, a rewrite-census admission (`make rewrite-census` fails when the rewrite
          stops firing), or a `LOFT_NO_*` switch A/B in a cell.  The column that says a test
          would fail; everything else says it looks at the right thing.
reach     for the axes the rule's chapter quantifies over (`scripts/rule_axes.tsv`, the
          by-hand input), the values the rule's `.loft` cells reach, as a union over the cells,
          against the language's domain (`scripts/matrix_axes.py`).  An axis with ONE value
          reached across every cell is PINNED — the shape of loft#1105, whose every cell built
          its container inside the calling function — and is named as such.  A floor: the
          census reads syntax, and ROUTE and ESCAPE (which decoder, whether a closure leaves
          its frame) are not composition axes and are not measured here.
sites     the enforcing sites in `src/` and `default/` that cite the rule — the targets of a
          site-mutation pass (perturb one, run the rule's cells, a site no cell catches is a
          vacuous guard), which this report does not run.

The shallow flags: C one cell file or none · B no cell on both backends · F no receipt ·
P an axis pinned at one value · X an axis under half reached.  Rules are ordered by the
defects their cells name (the walk's own yield order), then by flag count, then by reach.
"""

import collections
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import matrix_axes  # noqa: E402
import rule_tags  # noqa: E402

ROOT = rule_tags.ROOT
AXES_TSV = os.path.join(HERE, "rule_axes.tsv")
FALSIFIED_DIR = os.path.join(ROOT, "tests", "falsified")
RECEIPT = re.compile(r"@falsified-at:\s*([0-9a-f]{7,40})\b")
ISSUE = re.compile(r"loft#(\d+)")
ISSUE_STEM = re.compile(r"^(\d{3,5})[a-z]?[-_]")
SWITCH = re.compile(r"\bLOFT_NO_[A-Z0-9_]+\b")
LOFT_CELL = (".loft",)


def chapter_axes():
    """{chapter: [axis]} from `rule_axes.tsv`; an unknown axis name is a usage error."""
    out = {}
    for line in open(AXES_TSV, encoding="utf-8"):
        if line.startswith("#") or not line.strip():
            continue
        chapter, axes, _why = (line.rstrip("\n").split("\t") + ["", ""])[:3]
        names = axes.split()
        bad = [a for a in names if a not in matrix_axes.AXIS_DOMAIN]
        if bad:
            raise SystemExit(f"rule_axes.tsv: {chapter}: unknown axis {bad} (matrix_axes.py names "
                             f"{', '.join(matrix_axes.AXIS_DOMAIN)})")
        out[chapter] = names
    return out


def chapter_of(files):
    return sorted(files)[0].removesuffix(".md").removesuffix("-history")


def read(path):
    try:
        return open(path, encoding="utf-8", errors="replace").read()
    except OSError:
        return ""


def abs_path(rel):
    return rel if os.path.isabs(rel) else os.path.join(ROOT, rel)


class Depth:
    """One rule's measurements."""

    def __init__(self, tag, chapter, cites, axes, census_admitted, texts):
        self.tag, self.chapter, self.axes = tag, chapter, axes
        lines_of = collections.defaultdict(set)
        for f, n in cites:
            lines_of[f].add(n)
        self.cells = sorted(lines_of)
        self.loft = [f for f in self.cells if f.endswith(LOFT_CELL)]
        self.both = 0
        self.issues = set()
        self.receipts = []
        for f in self.cells:
            text = texts[f]
            if f.endswith(LOFT_CELL):
                self.both += 1
            elif "--interpret" in text and "--native" in text:
                self.both += 1
            # The defects a cell names: its file name's issue number, and a `loft#N` within a
            # few lines of the citation — not every issue the file mentions, or `tests/issues.rs`
            # would hand its hundreds to each rule one of its tests cites.
            m = ISSUE_STEM.match(os.path.basename(f))
            if m:
                self.issues.add(m.group(1))
            rows = text.split("\n")
            for n in lines_of[f]:
                near = "\n".join(rows[max(0, n - 6):n + 5])
                self.issues |= set(ISSUE.findall(near))
            for sha in RECEIPT.findall(text):
                self.receipts.append(f"@falsified-at {sha[:9]} in {os.path.basename(f)}")
            stem = os.path.basename(f).removesuffix(".loft")
            if os.path.exists(os.path.join(FALSIFIED_DIR, stem + ".patch")):
                self.receipts.append(f"tests/falsified/{stem}.patch")
            if SWITCH.search(text):
                self.receipts.append(f"switch A/B {SWITCH.search(text).group(0)} in {os.path.basename(f)}")
        if census_admitted:
            self.receipts.append("rewrite census admits it on a bench program")
        # The union of what the cells reach, per axis the chapter quantifies over.
        self.reach = {}
        for a in axes:
            hit = set()
            for f in self.loft:
                hit |= texts["census"][f][a]
            self.reach[a] = hit

    def reach_pct(self):
        if not self.axes:
            return None
        got = sum(len(self.reach[a]) for a in self.axes)
        dom = sum(len(matrix_axes.AXIS_DOMAIN[a]) for a in self.axes)
        return 100.0 * got / dom if dom else None

    def flags(self):
        out = ""
        if len(self.cells) <= 1:
            out += "C"
        if self.both == 0:
            out += "B"
        if not self.receipts:
            out += "F"
        if any(len(self.reach[a]) == 1 for a in self.axes):
            out += "P"
        if any(len(self.reach[a]) * 2 < len(matrix_axes.AXIS_DOMAIN[a]) for a in self.axes):
            out += "X"
        return out


def measure(rules=None):
    defined = rule_tags.defined_rules()
    rules = rules or set(defined)
    guard = rule_tags.guard_citations()
    census = rule_tags.census_rules()
    sites = rule_tags.citations_in([rule_tags.SRC, rule_tags.DEFAULT], rule_tags.CITE_EXTS)
    axes = chapter_axes()
    files_of = {t: {f for f, _ in guard.get(t, [])} for t in rules}
    texts = {"census": {}}
    for fs in files_of.values():
        for f in fs:
            if f not in texts:
                texts[f] = read(abs_path(f))
                if f.endswith(LOFT_CELL):
                    try:
                        texts["census"][f] = matrix_axes.census(abs_path(f))
                    except OSError:
                        texts["census"][f] = {a: set() for a in matrix_axes.AXIS_DOMAIN}
    out = []
    for tag in sorted(rules):
        if tag not in defined or not files_of[tag]:
            continue
        ch = chapter_of(defined[tag])
        d = Depth(tag, ch, guard[tag], axes.get(ch, []), tag in census, texts)
        d.sites = len(sites.get(tag, []))
        out.append(d)
    return out


def order(depths):
    return sorted(depths, key=lambda d: (-len(d.issues), -len(d.flags()),
                                         d.reach_pct() if d.reach_pct() is not None else 101.0,
                                         d.tag))


def row(d):
    pct = d.reach_pct()
    reach = "   -" if pct is None else f"{pct:3.0f}%"
    pinned = ",".join(a for a in d.axes if len(d.reach[a]) == 1)
    return (f"@FR-{d.tag:<24} {d.chapter:<12} {len(d.cells):>5} {d.both:>4} {len(d.issues):>7} "
            f"{'yes' if d.receipts else 'no':>7} {reach:>5} {d.sites:>5}  {d.flags():<5} {pinned}")


HEAD = (f"{'rule':<28} {'chapter':<12} {'cells':>5} {'both':>4} {'defects':>7} {'receipt':>7} "
        f"{'reach':>5} {'sites':>5}  flags pinned")


def summary(depths, total_rules):
    n = len(depths)
    print(f"{total_rules} defined rules, {n} with a citing guard — depth over those {n}:\n")
    print(f"  two or more cell files   {sum(len(d.cells) >= 2 for d in depths):>4} / {n}")
    print(f"  a cell on both backends  {sum(d.both > 0 for d in depths):>4} / {n}")
    print(f"  a falsification receipt  {sum(bool(d.receipts) for d in depths):>4} / {n}")
    measured = [d for d in depths if d.axes]
    print(f"  no axis pinned           {sum('P' not in d.flags() for d in measured):>4} / {len(measured)}"
          f"   (rules of a chapter rule_axes.tsv names)")
    print(f"  every axis half reached  {sum('X' not in d.flags() for d in measured):>4} / {len(measured)}")
    print(f"  no shallow flag at all   {sum(not d.flags() for d in depths):>4} / {n}\n")
    fam = collections.defaultdict(lambda: [0, 0, 0, 0])
    for d in depths:
        row_ = fam[rule_tags.family_of(d.tag)]
        row_[0] += 1
        row_[1] += bool(d.receipts)
        row_[2] += "P" not in d.flags() and "X" not in d.flags()
        row_[3] += not d.flags()
    print(f"  {'family':<8} {'guarded':>7} {'receipt':>7} {'reach ok':>8} {'deep':>5}")
    for f, (g, r, k, deep) in sorted(fam.items()):
        print(f"  {f:<8} {g:>7} {r:>7} {k:>8} {deep:>5}")
    print()


def detail(d):
    print(f"@FR-{d.tag}  chapter {d.chapter}  flags {d.flags() or '-'}")
    print(f"  cells ({len(d.cells)}, {d.both} on both backends):")
    for f in d.cells:
        print(f"    {f}")
    print(f"  defects named: {', '.join('loft#' + i for i in sorted(d.issues, key=int)) or 'none'}")
    print(f"  receipts: {'; '.join(d.receipts) or 'NONE — no cell is known to fail without the rule'}")
    print(f"  enforcing sites in src/ and default/: {d.sites}")
    if not d.axes:
        print(f"  axes: the chapter `{d.chapter}` is not in scripts/rule_axes.tsv — reach not measured")
        return
    print("  reach, per axis the chapter quantifies over (union over the .loft cells):")
    for a in d.axes:
        dom = matrix_axes.AXIS_DOMAIN[a]
        hit = sorted(d.reach[a])
        miss = [v for v in dom if v not in d.reach[a]]
        mark = "PINNED" if len(hit) == 1 else ("under half" if len(hit) * 2 < len(dom) else "")
        print(f"    {a} {matrix_axes.AXIS_TITLE[a]:<22} {len(hit)}/{len(dom)}  {mark:<10} "
              f"reaches {', '.join(hit) or '-'}" + (f"; missing {', '.join(miss)}" if miss else ""))


def selftest():
    axes = chapter_axes()
    assert "heap" in axes and "A1" in axes["heap"], axes.get("heap")
    assert RECEIPT.search("// @falsified-at: 9528ddc4 — interpret") and not RECEIPT.search("// @falsified-at: none")
    assert ISSUE_STEM.match("1017b-a-conditional.loft").group(1) == "1017"
    assert not ISSUE_STEM.match("an-omitted-field.loft")
    ds = measure()
    assert ds, "no guarded rule measured"
    d = next(x for x in ds if x.axes)
    assert all(d.reach[a] <= set(matrix_axes.AXIS_DOMAIN[a]) for a in d.axes)
    print(f"selftest: ok ({len(ds)} rules measured)")


def main():
    a = sys.argv[1:]
    if "-h" in a or "--help" in a:
        print(__doc__.strip())
        return 0
    if a[:1] == ["selftest"]:
        selftest()
        return 0
    total = len(rule_tags.defined_rules())
    if "--rule" in a:
        tag = a[a.index("--rule") + 1].removeprefix("@FR-")
        ds = measure({tag})
        if not ds:
            print(f"@FR-{tag}: not a defined rule, or no guard cites it")
            return 1
        detail(ds[0])
        return 0
    ds = measure()
    if "--family" in a:
        fam = a[a.index("--family") + 1]
        ds = [d for d in ds if rule_tags.family_of(d.tag) == fam]
    ds = order(ds)
    if "--tsv" in a:
        print("rule\tchapter\tcells\tboth\tdefects\treceipt\treach_pct\tsites\tflags\tpinned")
        for d in ds:
            pct = d.reach_pct()
            print(f"{d.tag}\t{d.chapter}\t{len(d.cells)}\t{d.both}\t{len(d.issues)}\t{int(bool(d.receipts))}\t"
                  f"{'' if pct is None else f'{pct:.0f}'}\t{d.sites}\t{d.flags()}\t"
                  f"{','.join(x for x in d.axes if len(d.reach[x]) == 1)}")
        return 0
    if "--family" not in a:
        summary(ds, total)
    shown = ds if ("--all" in a or "--family" in a) else [d for d in ds if d.flags()][:40]
    print(HEAD)
    for d in shown:
        print(row(d))
    if shown is not ds:
        print(f"\n{len([d for d in ds if d.flags()])} guarded rules carry a shallow flag; the 40 with the most "
              "defects behind them are shown (--all for every rule, --rule <tag> for one in full).")
    print("\nflags: C one cell file or none · B no both-backend cell · F no receipt · P an axis pinned "
          "at one value · X an axis under half reached.  A REPORT of reach, never a gate; what it "
          "cannot see: ROUTE and ESCAPE (matrix_axes.py caveats), and whether a site's mutation "
          "would be caught, which no column here runs.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
