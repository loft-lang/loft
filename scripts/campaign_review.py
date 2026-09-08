#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Which mechanism class earns a CAMPAIGN next — the four gates, as one report.

The question *"which family is worth a plan?"* is asked every cycle and was answered by
reading four instruments by hand and holding the answers in one head.  This joins them:
one row per mechanism class, one column per gate (@PLN155 arc A).

  1. TREND       is the class rising, or flat at its own peak?   `bug-review.py` § 2.
  2. SPELLINGS   is one fact carried in two or more representations, with no predicate
                 answering both?  `rule_predicate_audit.py` (a `Type` list hand-spelled at
                 N sites) and `rule_tags.py dups` (one rule cited from N files).
  3. CHOKEPOINT  is there a home every site goes through, or does the majority resolve the
                 shape by naming variants?  `ir_walker_audit.py former <F>`'s opacity
                 screen, one type former at a time.
  4. WALK        has a rule-led walk already been tried on this class, and did the class's
                 share fall afterwards?  The QUALITY.md walk records, measured against the
                 bug population rather than believed.

A class passing all four earns a PLAN; passing only 1-3 earns a rule-led WALK; passing only
gate 2 earns a QUEUE entry.

**Gate 4 is the one that must stay loud.**  A class whose keystone has landed but whose
payoff row cannot be judged yet is not a candidate — it is an unfinished MEASUREMENT, and a
report that ranked it would be reading three gates out of four.  Such a class prints
`MEASURING` and is held back whatever the other three say.

A REPORT, never a gate.  Every input is heuristic, the class->rule map below is a stated
judgement rather than a derivation, and an unmeasured gate is printed as `-` and counts as
NOT passed — never as a pass.  Read the numbers beside the ticks: within one verdict the
ranking is theirs, not this script's.

Usage:
    scripts/campaign_review.py                      # fetch from gh
    scripts/campaign_review.py --cache issues.json  # re-run offline
    scripts/campaign_review.py --before 2026-08-01  # the population as of a past date
    scripts/campaign_review.py --verbose            # + the evidence behind each gate
"""
import argparse
import importlib.util
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "scripts"))

import ir_walker_audit as walker  # noqa: E402 — after the path insert above
import rule_predicate_audit as predicates  # noqa: E402
import rule_tags  # noqa: E402


def _bug_review():
    """`bug-review.py` under its hyphenated name — the home of the class signatures, the
    band arithmetic and the payoff verdict, all three of which this report reads rather
    than respells."""
    path = os.path.join(ROOT, "scripts", "bug-review.py")
    spec = importlib.util.spec_from_file_location("bug_review", path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


review = _bug_review()

# ── the class -> formal-rule map ──────────────────────────────────────────────
# A STATED judgement, printed with the report so it can be argued with.  Keyed by
# `(rule family, defining doc)` because neither half identifies an area on its own: `F` is
# `calls.md` and `formatting.md`, `L` is `closures.md` and `layout.md`, `G` is three docs.
#
# A rule whose own TAG matches a class signature overrides this map — `@FR-L-Null-Tag`
# lives in `layout.md` and is a null rule, and its name says so.  That refinement is
# derived, so the declared table only has to be right about the families whose names are
# silent about their subject.
FAMILY_HOME = {
    ("N", "types.md"): "null/sentinel",
    ("O", "ownership.md"): "ownership/free",
    ("H", "heap.md"): "ownership/free",
    ("B", "binding.md"): "ownership/free",
    ("Col", "collections.md"): "keyed collections",
    ("Slice", "collections.md"): "keyed collections",
    ("T", "tuples.md"): "tuple",
    ("G", "interfaces.md"): "generic/monomorph",
    ("G", "coroutines.md"): "par/coroutine",
    ("C", "concurrency.md"): "par/coroutine",
    ("M", "matching.md"): "enum/variant",
    ("P", "matching.md"): "enum/variant",
    ("L", "layout.md"): "narrow-int/width",
    ("I", "iteration.md"): "traversal/reach",
}

# The `Type` / `Parts` variants each class's facts are spelled over, for gate 2's
# hand-spelled-list half.  A list is attributed to a class when it names TWO or more of
# them: one shared variant is a coincidence (`Type::Null` is in almost every list), two is
# the class's own shape.
CLASS_VARIANTS = {
    "keyed collections": {"Hash", "Index", "Sorted", "Radix", "Trie", "Ordered"},
    "tuple": {"Tuple"},
    "null/sentinel": {"Null", "Optional"},
    "narrow-int/width": {"Integer", "Float", "Single", "Character", "Boolean"},
    "ownership/free": {"Reference", "Enum", "RefVar", "Vector", "Text"},
    "enum/variant": {"Enum"},
}

# The type FORMER whose opacity screen answers gate 3 for each class.  A class with no
# former has no number, and the plan that says so is explicit about the consequence:
# without one, condition 3 is an opinion, so the gate reads `-` and does not pass.
CLASS_FORMER = {
    "null/sentinel": "Optional",
    "ownership/free": "RefVar",
    "tuple": "Tuple",
}

# A rule DEFINITION line inside a rules block: `  (Name)  prose`.  Same shape
# `rule_tags.py` parses, read here for the doc each rule lives in.
RULE_DEF = re.compile(r"^\s*\(([A-Z][A-Za-z0-9-]{1,40})\)\s", re.M)
# A walk record: `#### B7x — `@FR-Rule` walked: … (YYYY-MM-DD)`.
WALK = re.compile(
    r"^#### (B7[a-z]+) — .?`?@FR-([A-Za-z0-9-]+)`?.{0,300}?walked.*?\((\d{4}-\d\d-\d\d)\)\s*$",
    re.M,
)
ISSUE_REF = re.compile(r"loft#(\d+)")
# A class passes gate 2 at the plan's own threshold: one fact in TWO representations.
# The magnitudes are printed beside the tick because the ranking within a verdict is
# theirs — a list spelled at 26 sites and one spelled at 2 both pass.
SPELLING_MIN = 2
# Gate 3 passes when the MAJORITY of shape-resolving sites cannot see the former's
# wrapper: that is what "no chokepoint by construction" means as a number.  Sites that
# descend via the `Type` keystone are chokepoint users and count on the seeing side.
CHOKEPOINT_OPAQUE_SHARE = 0.5
# The negative control.  Fed the bug population as it stood before 2026-08, this report must
# not name generic/monomorph a campaign: that class's keystone went on to PAY OFF, so a
# report that ranked it for a plan would be reading gates it had not measured.  Run it with
# `--control` after touching any gate — a mis-wire that lets a paid-off class through is the
# failure this instrument cannot afford, because the whole point is to stop a cycle being
# spent on work already done.
CONTROL_DATE = "2026-08-01"
CONTROL_CLASS = "generic/monomorph"


def rules_by_doc():
    """`{tag: doc}` for every rule the formal docs define."""
    out = {}
    formal = os.path.join(ROOT, "doc", "claude", "formal")
    for name in sorted(os.listdir(formal)):
        if not name.endswith(".md"):
            continue
        text = open(os.path.join(formal, name), encoding="utf-8").read()
        for m in RULE_DEF.finditer(text):
            out.setdefault(m.group(1), name)
    return out


def rule_class(tag, doc):
    """Which mechanism class does one rule belong to?

    The rule's own NAME first — a signature match on the tag is derived evidence and beats
    the table — then the `(family, doc)` map.  `None` where neither answers, which the
    report prints rather than hides: an unmapped rule is a hole in the map, and a silent
    drop is how a walk comes to be invisible to the gate that must stay loud.
    """
    for cls, pat in review.CLASSES.items():
        if re.search(pat, tag, re.I):
            return cls
    # A rule that has moved to its chapter's `-history` doc is the same rule in the same
    # area; keying on the history file would drop it, which is how `@FR-O-Complete` — one
    # of the five ownership walks this gate exists to count — read as unmapped.
    return FAMILY_HOME.get((tag.split("-")[0], doc.replace("-history.md", ".md")))


def walks():
    """The rule-led walks on record: `(id, tag, date, class, own_finds)`.

    `own_finds` is the issue numbers the walk's own section names — the population that
    must come OUT of the after-window before a share is compared, for the reason the bug
    review states about a screen that files into the window that scores it.
    """
    path = os.path.join(ROOT, "doc", "claude", "QUALITY.md")
    text = open(path, encoding="utf-8").read()
    docs = rules_by_doc()
    found, spans = [], [(m.start(), m.group(1), m.group(2), m.group(3)) for m in WALK.finditer(text)]
    heads = [m.start() for m in re.finditer(r"^#### ", text, re.M)]
    for start, wid, tag, date in spans:
        end = next((h for h in heads if h > start), len(text))
        own = {int(n) for n in ISSUE_REF.findall(text[start:end])}
        found.append((wid, tag, date, rule_class(tag, docs.get(tag, "?")), own))
    return found


def spelling_evidence():
    """Gate 2, both halves: `{class: (max_list_sites, list, max_rule_files, rule)}`."""
    out = {}
    lists = predicates.collect(3)
    cites = rule_tags.citations()
    docs = rules_by_doc()
    for cls in review.CLASSES:
        vs = CLASS_VARIANTS.get(cls, set())
        best_l, which_l = 0, None
        for names, where in lists.items():
            if len(set(names) & vs) >= 2 and len(where) > best_l:
                best_l, which_l = len(where), names
        best_r, which_r = 0, None
        for tag, where in cites.items():
            if rule_class(tag, docs.get(tag, "?")) != cls:
                continue
            files = len({f for f, _ in where})
            if files > best_r:
                best_r, which_r = files, tag
        out[cls] = (best_l, which_l, best_r, which_r)
    return out


def chokepoint_evidence(verbose):
    """Gate 3: `{class: (opaque, seeing, share)}` from the opacity screen, per former."""
    import contextlib
    import io

    out = {}
    for cls, former in CLASS_FORMER.items():
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            walker.audit_optional(walker.Former(former))
        text = buf.getvalue()
        nums = {}
        for key, pat in (("all", r"discriminating on a `Type` variant : (\d+)"),
                         ("sees", r"see through the wrapper \(peel or arm\)\s*: (\d+)"),
                         ("desc", r"descend via the `Type` keystone\s*: (\d+)"),
                         ("opaque", r"opaque to a wrapped shape\s*: (\d+)")):
            m = re.search(pat, text)
            nums[key] = int(m.group(1)) if m else 0
        seeing = nums["sees"] + nums["desc"]
        share = nums["opaque"] / nums["all"] if nums["all"] else 0.0
        out[cls] = (nums["opaque"], seeing, share, former)
        if verbose:
            print(f"    gate 3  {cls}: Type::{former} — {nums['opaque']} opaque, "
                  f"{nums['sees']} peel/arm, {nums['desc']} descend, of {nums['all']}")
    return out


def share_at(bugs, cls_nums, lo, hi, exclude):
    """The class's share of the bugs numbered in `[lo, hi)`, minus `exclude`."""
    pop = [i["number"] for i in bugs
           if lo <= i["number"] < hi and i["number"] not in exclude]
    if not pop:
        return 0.0, 0
    hits = sum(1 for n in pop if n in cls_nums)
    return 100 * hits / len(pop), hits


def walk_evidence(bugs, hits, verbose):
    """Gate 4: `{class: (verdict, detail)}` from the walk records, measured.

    A walk lands on a DAY, so the split is a watermark issue number — the highest bug
    filed before that day — rather than a band edge.  The walk's own finds are removed
    from the after-window: a walk files the bugs it discovers, and counting them as the
    class still firing scores the instrument instead of the code.
    """
    out, unmapped = {}, []
    by_class = {}
    # A walk dated after this population's newest bug has not happened yet as far as these
    # measurements go, and must read `none yet` rather than `cannot judge`.  Without this
    # the `--before` control scored every class against walks from its own future and
    # reported them all as unjudgeable, which reads like a broken gate and is not one.
    horizon = max(i.get("createdAt", "")[:10] for i in bugs)
    for wid, tag, date, cls, own in walks():
        if date > horizon:
            continue
        if cls is None:
            unmapped.append((wid, tag))
            continue
        by_class.setdefault(cls, []).append((wid, tag, date, own))
    for cls, ws in by_class.items():
        cls_nums = {i["number"] for i in hits.get(cls, [])}
        ws.sort(key=lambda w: w[2])
        first, last = ws[0], ws[-1]
        # Split at the FIRST walk on this class, not the last.  The question gate 4 asks is
        # *has walking this class moved it*, and the campaign starts at its first walk; the
        # last walk's date gives the shortest possible after-window, which for the seven
        # ownership walks is three days and cannot separate a fall from two unfiled bugs.
        mark = max((i["number"] for i in bugs if i.get("createdAt", "")[:10] < first[2]),
                   default=0)
        own = set().union(*(w[3] for w in ws))
        hi = max(i["number"] for i in bugs) + 1
        # TWO splits, and the gate needs both — the bug review's own convention, applied to
        # a walk instead of a keystone.  The RAW split is what every earlier pass was scored
        # on and stays the headline; the SCREENED one takes the walks' own finds out of both
        # windows, because a walk's job is to file the bugs it discovers and counting those
        # as the class still firing scores the instrument rather than the code.
        raw_before, nb = share_at(bugs, cls_nums, 0, mark, set())
        raw_after, _ = share_at(bugs, cls_nums, mark, hi, set())
        scr_before, snb = share_at(bugs, cls_nums, 0, mark, own)
        scr_after, _ = share_at(bugs, cls_nums, mark, hi, own)
        pop_after = sum(1 for i in bugs if i["number"] >= mark and i["number"] not in own)
        pop_raw = sum(1 for i in bugs if i["number"] >= mark)
        raw = review.payoff_verdict(raw_before, nb, raw_after, pop_raw)
        scr = review.payoff_verdict(scr_before, snb, scr_after, pop_after)
        if raw.startswith("NO EFFECT") and scr.startswith("NO EFFECT"):
            verdict = raw
        elif raw.startswith("cannot") or scr.startswith("cannot"):
            # An unjudgeable reading is not a pass.  Both of the ownership walks' readings
            # land here on a three-day window, which is the honest answer: the walk may
            # have worked and may not, and the class has not earned a campaign either way.
            verdict = scr if scr.startswith("cannot") else raw
        else:
            # Either reading showing a fall is enough to hold the class back.  Gate 4 is the
            # loud one: a class whose walk MAY have worked has not earned a campaign.
            verdict = "PAID OFF"
        out[cls] = (verdict,
                    f"{len(ws)} walk(s), {first[0]} @FR-{first[1]} {first[2]} .. "
                    f"{last[0]} @FR-{last[1]} {last[2]}, split at #{mark}: raw {raw_before:.1f}% -> {raw_after:.1f}% ({raw[:9]}), "
                    f"minus its own {len(own & {i['number'] for i in bugs})} finds "
                    f"{scr_before:.1f}% -> {scr_after:.1f}% ({scr[:9]}), {pop_after} bugs after")
        if verbose:
            print(f"    gate 4  {cls}: " + ", ".join(f"{w[0]} @FR-{w[1]} {w[2]}" for w in ws))
    return out, unmapped


def measuring(bugs, hits, bands, counts, tot):
    """Which classes are an unfinished MEASUREMENT — a landed keystone with no payoff row.

    The gate that must stay loud.  A keystone landing inside the last band has no band
    starting above it, so its class has not been scored yet; ranking it would read three
    gates out of four.
    """
    out = {}
    newest = max(i["number"] for i in bugs)
    for keystone, cls, landed, *_plan in review.KEYSTONES:
        if cls not in counts or landed > newest:
            # A keystone that has not landed within this population is not an unfinished
            # measurement — it is no measurement at all.  Only the `--before` control
            # reaches this, and without it every future keystone held its class back.
            continue
        after = [b for b in bands if b[0] >= landed]
        before = [b for b in bands if b[1] <= landed]
        if not after or not before:
            out[cls] = f"{keystone} landed at #{landed}, no band above it yet"
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cache", help="issues JSON from a previous gh call")
    ap.add_argument("--bands", type=int, default=4, help="time slices (default 4)")
    ap.add_argument("--before", help="measure the population as of YYYY-MM-DD")
    ap.add_argument("--verbose", action="store_true", help="print each gate's evidence")
    ap.add_argument("--control", action="store_true",
                    help="the negative control: fed the pre-2026-08 population, this must "
                         "NOT name generic/monomorph a campaign")
    a = ap.parse_args()
    if a.control:
        a.before = CONTROL_DATE

    issues = review.load(a.cache)
    bugs = [i for i in issues if any(lb["name"] == "bug" for lb in i["labels"])]
    if a.before:
        bugs = [i for i in bugs if i.get("createdAt", "")[:10] < a.before]
    if len(bugs) < 20:
        sys.exit(f"only {len(bugs)} bug issues in this population — nothing to rank")

    bands = review.band_edges(bugs, a.bands)
    rows, counts, tot, hits = review.class_trends(bugs, bands)
    print("== campaign review (@PLN155 arc A) — which class earns a campaign next? ==\n")
    print(f"  {len(bugs)} bug issues #{bands[0][0]}-{bands[-1][1] - 1}, {a.bands} bands"
          + (f", filed before {a.before}" if a.before else ""))
    if a.verbose:
        print()
    spell = spelling_evidence()
    choke = chokepoint_evidence(a.verbose)
    walked, unmapped = walk_evidence(bugs, hits, a.verbose)
    held = measuring(bugs, hits, bands, counts, tot)

    print()
    hdr = (f"  {'class':<20}{'1 trend':<24}{'2 spellings':<22}"
           f"{'3 chokepoint':<24}{'4 walk tried':<28}verdict")
    print(hdr + "\n  " + "-" * (len(hdr) - 2))
    out = []
    for delta, cls, sh, _c, peak in rows:
        g1 = review.trend_mark(delta) != "falling"
        c1 = f"{review.trend_mark(delta)} {delta:+.1f}pp"
        n_l, _wl, n_r, _wr = spell[cls]
        g2 = max(n_l, n_r) >= SPELLING_MIN
        c2 = f"list {n_l}x  rule {n_r}f"
        if cls in choke:
            opaque, seeing, share, former = choke[cls]
            g3 = share >= CHOKEPOINT_OPAQUE_SHARE
            c3 = f"{former} {100 * share:.0f}% opaque"
        else:
            g3, c3 = False, "-"
        if cls in walked:
            verdict4 = walked[cls][0]
            g4 = verdict4.startswith("NO EFFECT")
            c4 = ("NO EFFECT" if g4 else
                  "PAID OFF" if verdict4.startswith("PAID") else "unjudged (thin window)")
        else:
            g4, c4 = False, "none yet"
        passed = sum((g1, g2, g3, g4))
        if cls in held:
            verdict = "MEASURING"
        elif passed == 4:
            verdict = "PLAN"
        elif g2 and passed == 1:
            verdict = "queue"
        elif passed >= 1:
            verdict = "walk"
        else:
            verdict = "-"
        tick = lambda ok: "v" if ok else " "  # noqa: E731 — a column marker, not logic
        out.append((passed, sh[-1], cls,
                    f"  {cls:<20}{c1 + ' ' + tick(g1):<24}{c2 + ' ' + tick(g2):<22}"
                    f"{c3 + ' ' + tick(g3):<24}{c4 + ' ' + tick(g4):<28}{verdict}"))
    for _p, _s, _c, line in sorted(out, reverse=True):
        print(line)

    if a.control:
        named = [c for _p, _s, c, line in out if line.rstrip().endswith("PLAN")]
        ok = CONTROL_CLASS not in named
        print(f"\n  CONTROL: fed the population before {CONTROL_DATE}, the report names "
              f"{named or ['no class']} a PLAN.")
        print(f"  {CONTROL_CLASS} must NOT be among them — its keystone PAID OFF, so a")
        print("  report that ranked it would be reading a gate it has not measured.")
        print("  " + ("PASS" if ok else "FAIL — a gate is mis-wired"))
        return 0 if ok else 1
    print("\n  v = the gate passes.  A `-` gate is UNMEASURED and does not pass.")
    print("  4 passes = PLAN · 1-3 = a rule-led walk · gate 2 alone = a queue entry.")
    for cls, why in sorted(held.items()):
        print(f"\n  ! {cls} is HELD as an unfinished measurement: {why}.")
        print("    Not a candidate whatever the other gates say — its keystone has not")
        print("    been scored yet, and ranking it would read three gates out of four.")
    if unmapped:
        print(f"\n  {len(unmapped)} walk(s) map to no mechanism class — a hole in FAMILY_HOME,")
        print("  not an absence of walks: " + ", ".join(f"{w} @FR-{t}" for w, t in unmapped))
    if a.verbose:
        print("\n  gate 2 evidence — the widest-spread fact per class:")
        for cls in review.CLASSES:
            n_l, wl, n_r, wr = spell[cls]
            if not (n_l or n_r):
                continue
            print(f"    {cls:<20} list {n_l}x {'+'.join(sorted(wl)) if wl else '-'}"
                  f"   rule {n_r} files @FR-{wr}")
        print("\n  gate 4 evidence:")
        for cls, (_v, detail) in sorted(walked.items()):
            print(f"    {cls:<20} {detail}")
    print()


if __name__ == "__main__":
    sys.exit(main() or 0)
