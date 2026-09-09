#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Monthly bug-review aid: which MECHANISM classes are still producing bugs.

Reports five things and judges none of them (see BUG_REVIEW.md):

  1. the population this cycle reviews;
  2. each mechanism class's share of bugs over time, so a class that is still
     firing is visible next to one that has gone quiet;
  3. the payoff check — for every keystone already landed, whether its class's
     bug share actually fell afterwards;
  4. enumeration exposure — how often each child-bearing `Value` variant is
     omitted from a hand-written walker, beside how many bugs it has carried;
  5. contract pressure — of the bugs we FIXED, how many needed the written standard
     to move.  That is the convergence signal the contract-1 decision reads, and it
     is not the bug count: finding bugs is the audits working, while a moving
     standard is the only one of the two that can make a freeze premature.

Bugs are bucketed by ISSUE NUMBER, not close date: the tracker is young and a
release-month close-out lands hundreds of old issues at once, which makes any
calendar window read as "everything is recent".

Usage:
    scripts/bug-review.py                      # fetch from gh
    scripts/bug-review.py --cache issues.json  # re-run offline
    scripts/bug-review.py --bands 4            # coarser/finer time slicing
"""
import argparse, json, pathlib, re, subprocess, sys
from collections import defaultdict

# Mechanism signatures, matched against the issue TITLE.  loft issue titles state
# a mechanism, which is what makes title-matching usable here; the classes overlap
# on purpose (one bug can belong to several) so shares are read per class, never
# summed.
CLASSES = {
    "generic/monomorph": r"generic|monomorph|type variable|template|instantiat",
    "tuple":             r"\btuple",
    "null/sentinel":     r"\bnull|sentinel|nullable",
    "narrow-int/width":  r"narrow|width|\bbyte|u8|u16|i32",
    "ownership/free":    r"\bfree|leak|use-after|UAF|owner|borrow|double-",
    "enum/variant":      r"\benum\b|variant|discriminant",
    "keyed collections": r"\bhash\b|sorted|radix|trie|spatial|keyed",
    "wasm/browser":      r"wasm|browser|html",
    "packages/registry": r"registr|package|loft\.toml|install|publish",
    "par/coroutine":     r"\bpar\b|parallel|worker|coroutine|yield|generator",
    "traversal/reach":   r"reachab|walk|traver|descend|prune|unmarked|unreachable",
}

# Keystones already landed, and the class each was meant to retire.  The payoff
# check reads this: a keystone whose class did NOT fall afterwards is the useful
# finding, because it means the fact was not the one manufacturing the bugs.
KEYSTONES = [
    ("IntegerSpec::range_to_width",  "narrow-int/width",  700),
    ("Stores::for_each_owned_child", "keyed collections", 715),
    ("Value::for_each_child",        "traversal/reach",   700),
    # @PLN153 phase 3: the `(N-Store)` refusal folded onto ONE body at `convert`'s
    # `τ? ⤳ τ` arm — the generics-precedent shape, a refusal at the point every
    # escape ends up.  Landed 2026-09-05 with the #1366 guards.
    ("Parser::nstore_unwrap_report", "null/sentinel",    1366, "PLN153"),
    # The 2026-09 cycle's keystone, landed at the end of the #1124-#1259 window it was
    # picked from (BUG_REVIEW.md).  Listed with its row still unjudgeable on purpose: a
    # keystone the review cannot see is a keystone whose class reads as a fresh candidate,
    # which is the reading @PLN155 arc A exists to refuse.
    ("Scopes::owns_freeable_store",   "keyed collections", 1260),
    # @PLN155's own keystone: the @FR-O-Proxy / @FR-O-Override PAIR given one home, which six
    # free sites had been spelling by hand.  Registered the day the plan's phases landed, with
    # its payoff row necessarily blank — which is the POINT.  Without this row arc A reads the
    # class as a fresh candidate and names it a PLAN again, one day after a plan finished on
    # it; with it, the class reads MEASURING, which is what an unscored keystone means.
    ("Function::proxy_says_owned",     "ownership/free",   1479, "PLN155"),
]

# ⚠ **The fourth trap, and it is the one that reads as a verdict.**  A keystone whose plan
# also ran a SCREEN for its own class cannot be judged on the raw share, because the screen
# FILES that class's bugs into the very window that scores it.  Measured on @PLN153: 21 of
# the 36 null/sentinel bugs after its watermark are its own phase-4 finds, and the share
# reads 37.3 % with them and 20.8 % without — the difference between "re-open the premise"
# and "the first fall this class has had".
#
# Neither number is the verdict, which is why both are printed.  The raw line is what the
# earlier passes were scored on and stays the headline; the split line says how much of it
# is the screen looking at itself.  An issue counts as the plan's own find when its BODY
# names the plan — the convention every batch already follows when it files.
PLAN_TAG = re.compile(r"@?PLN(\d+)")

# Child-bearing IR variants — the set `IrNode::for_each_child` is exhaustive over.
CHILD_BEARING = ["Call", "CallRef", "Insert", "Tuple", "Parallel", "Block", "Loop",
                 "Set", "Return", "Drop", "Yield", "TuplePut", "Span",
                 "If", "Iter"]
FN_RE = re.compile(r"^\s*(pub(\([^)]*\))?\s+)?(const\s+)?fn\s+([a-z_0-9]+)")


def load(cache):
    if cache:
        return json.loads(pathlib.Path(cache).read_text())
    out = subprocess.run(
        ["gh", "issue", "list", "--state", "all", "--limit", "1200",
         "--json", "number,title,labels,state,closedAt,createdAt,body"],
        capture_output=True, text=True)
    if out.returncode != 0:
        sys.exit(f"gh failed: {out.stderr.strip()}\n"
                 f"(offline? re-run with --cache <file.json>)")
    return json.loads(out.stdout)


def classify(issues):
    hits = defaultdict(list)
    for i in issues:
        for name, pat in CLASSES.items():
            if re.search(pat, i["title"], re.I):
                hits[name].append(i)
    return hits


def walker_omissions():
    """How often each child-bearing variant is left out of a PARTIAL walker.

    Counts only walkers that recurse, carry a wildcard arm, and do not delegate
    to a keystone — a delegating or exhaustive walker cannot omit anything.
    """
    present, total = defaultdict(int), 0
    for p in sorted(pathlib.Path("src").rglob("*.rs")):
        lines = p.read_text(errors="replace").splitlines()
        cur, body, fns = None, [], []
        for line in lines:
            m = FN_RE.match(line)
            if m:
                if cur:
                    fns.append((cur, body))
                cur, body = m.group(4), []
            if cur is not None:
                body.append(line)
        if cur:
            fns.append((cur, body))
        for name, body in fns:
            txt = "\n".join(body)
            arms = set(re.findall(r"Value::([A-Z]\w+)", txt))
            if len(arms) < 4:
                continue
            if not re.search(r"\b" + re.escape(name) + r"\s*\(", txt[txt.find("{"):]):
                continue
            if "for_each_child" in txt:            # delegates — total by construction
                continue
            if not re.search(r"^\s*(_|other)\s*(\||=>)", txt, re.M):
                continue                            # exhaustive — cannot omit
            total += 1
            for v in CHILD_BEARING:
                if v in arms:
                    present[v] += 1
    return present, total


def band_edges(bugs, nbands):
    """Equal-WIDTH slices of the issue-number range — the population's own time axis.

    Bucketed by issue NUMBER, not close date: a release-month close-out lands hundreds of
    old issues at once, which makes any calendar window read as "everything is recent".
    """
    nums = sorted(i["number"] for i in bugs)
    lo, hi = nums[0], nums[-1]
    step = max(1, (hi - lo) // nbands)
    return [(lo + k * step, lo + (k + 1) * step if k < nbands - 1 else hi + 1)
            for k in range(nbands)]


def class_trends(bugs, bands):
    """One row per mechanism class: `(delta_vs_peak, name, shares, counts, peak)`.

    Measured against the PEAK, not band 0.  A class that did not exist in the first band,
    rose, and has since fallen is FALLING; comparing it to zero would call it rising and
    point the cycle at work already done.

    One home because two readers ask it — section 2 below, and `campaign_review.py`'s
    first gate (@PLN155 arc A).  A second spelling of this arithmetic is how a campaign
    comes to be picked off a trend the review itself never printed.
    """
    hits = classify(bugs)
    counts = {n: {b: 0 for b in bands} for n in CLASSES}
    tot = {b: 0 for b in bands}
    for i in bugs:
        for b in bands:
            if b[0] <= i["number"] < b[1]:
                tot[b] += 1
    for name, lst in hits.items():
        for i in lst:
            for b in bands:
                if b[0] <= i["number"] < b[1]:
                    counts[name][b] += 1
    rows = []
    for name in CLASSES:
        sh = [100 * counts[name][b] / tot[b] if tot[b] else 0.0 for b in bands]
        peak = max(sh[:-1]) if len(sh) > 1 else sh[0]
        rows.append((sh[-1] - peak, name, sh, [counts[name][b] for b in bands], peak))
    return rows, counts, tot, hits


def trend_mark(delta):
    """The three-way verdict on one class's share — one home, two readers."""
    return "RISING" if delta > 2 else ("falling" if delta < -2 else "flat")


# A fall has to be bigger than the noise of the window it is read in.  One percentage
# point of a 90-bug window is under one bug, so a 2pp "fall" there can be two issues that
# happened not to be filed — which is how a three-day window came to read PAID OFF for a
# class its own campaign was being written about.  A verdict therefore needs BOTH: a share
# that fell by more than a point, and a fall worth at least this many bugs against what the
# before-share predicted.
PAYOFF_MIN_BUGS = 3


def payoff_verdict(before_share, before_n, after_share, after_pop):
    """Did a landed keystone move its class's share?  One home, two readers.

    A class with (almost) no bugs BEFORE the keystone cannot show a fall after it — there
    was nothing to remove.  Abstain rather than print a verdict the data does not carry: a
    false "NO EFFECT" would send the cycle to re-open a premise that was never tested, and
    a false "PAID OFF" would close one that was never confirmed.

    The SPLIT is the caller's: section 3 below compares whole bands (a keystone landing
    inside the last band has no band above it and is not judged at all), while
    `campaign_review.py` splits at a watermark issue number, because a walk lands on a day
    and not on a band edge.  Those are two questions.  The VERDICT rule over a split is
    one, and it is here.
    """
    if before_n < 3:
        return f"cannot judge — only {before_n} bug(s) in this class before it landed"
    if after_share >= before_share - 1:
        return "NO EFFECT — re-open the premise"
    missing = (before_share - after_share) * after_pop / 100
    if missing < PAYOFF_MIN_BUGS:
        return (f"cannot judge — the fall is {missing:.1f} bug(s) "
                f"in a {after_pop}-bug window")
    return "PAID OFF"


def stated_fixed(issue):
    """Is this an issue we STATED WE FIXED — the population a contract verdict applies to?

    Closed, or carrying `fixed-pending-merge`.  One home because section 5 asks it twice
    (by month, then by mechanism class) and two spellings of a population predicate is how
    two tables come to disagree about their own denominator.

    NOT the `bug` label: sections 1-4 ask what KIND of defect it was, this one asks what
    the FIX needed.  Measured while building it — three of the first four judged issues
    (#1120, #1122, #1123) carry no `bug` label, so that filter counted one of four.
    """
    return (issue["state"] == "CLOSED"
            or any(l["name"] == "fixed-pending-merge" for l in issue["labels"]))


def verdict_of(issue):
    """`settled` / `strained` / None — None means NOT JUDGED, never "settled"."""
    names = {l["name"] for l in issue["labels"]}
    if "contract:strained" in names:
        return "strained"
    if "contract:settled" in names:
        return "settled"
    return None


def contract_pressure(issues, months):
    """Section 5 — of the bugs we FIXED, how many moved the written standard?

    Counted by the month the issue was FILED, which is the only date every issue has;
    the verdict itself is set when the fix lands (.github/LABELS.md `contract:`).  A
    lag between the two is expected and harmless — a month's ratio settles as its fixes
    land, and the UNJUDGED column is what says how much of it is still settling.

    Unjudged is printed, never folded into either side.  A count that read an
    unlabelled issue as settled would report convergence it never measured, which is
    exactly the reassurance this section exists to withhold.
    """
    from collections import Counter
    settled, strained, unjudged = Counter(), Counter(), Counter()
    for i in issues:
        if not stated_fixed(i):
            continue
        m = i.get("createdAt", "")[:7]
        if not m:
            continue
        v = verdict_of(i)
        (strained if v == "strained" else settled if v == "settled" else unjudged)[m] += 1
    seen = sorted(set(settled) | set(strained) | set(unjudged))[-months:]
    print("\n=== 5. Contract pressure — did fixing them MOVE the standard? ===")
    if not seen:
        print("  no dated bugs")
        return
    print("  month     settled  strained   judged-ratio   unjudged")
    for m in seen:
        s_, x_, u_ = settled[m], strained[m], unjudged[m]
        judged = s_ + x_
        ratio = f"{100 * x_ / judged:5.1f}% strained" if judged else "      —      "
        print(f"  {m}   {s_:6d}  {x_:7d}   {ratio}   {u_:7d}")
    tot_j = sum(settled[m] + strained[m] for m in seen)
    tot_u = sum(unjudged[m] for m in seen)
    if tot_j == 0:
        print("\n  Nothing judged yet in this window — the axis is new.  Every fix that")
        print("  writes a `Contract:` trailer adds one — the push labels the issue off")
        print("  it; `scripts/contract_labels.py` names the fixes that carried none.")
    elif tot_u > tot_j:
        print(f"\n  ⚠ {tot_u} unjudged against {tot_j} judged — the ratio above is drawn from")
        print("  a minority of the population and is not yet evidence either way.")

    # The cross-tab.  Same axis cut by mechanism class, because a rising class means two
    # different jobs depending on which way its fixes went, and the month view cannot tell
    # them apart:
    #
    #   mostly SETTLED  — the rules were right and the code kept missing them, so the
    #                     duplicated case analysis is the target (a code keystone);
    #   any STRAINED    — closing them had to MOVE the standard, so the formal spec is
    #                     incomplete there and a RULE is the target, not a refactor.
    #
    # Reported, not judged — which class is worth one generalization stays the pass's call
    # (BUG_REVIEW.md § The pass).  Sorted by strained first so an unsettled SPEC surfaces
    # above a merely busy class; ties by judged count, so the best-evidenced row leads.
    print("\n  by mechanism class — of the FIXED ones, which way did they go?")
    print("    class                 fixed  settled  strained   unjudged")
    # Classified from the STATED-FIXED population, not from section 2's `hits`.  That set
    # is built from `bug`-labelled issues, and the `bug` label is not reliably applied —
    # three of the first four judged issues lack it — so reusing it would drop exactly the
    # rows this table exists to show, and drop them silently.
    fixed_hits = classify([i for i in issues if stated_fixed(i)])
    rows = []
    for name in CLASSES:
        fixed = fixed_hits.get(name, [])
        if not fixed:
            continue
        v = [verdict_of(i) for i in fixed]
        rows.append((v.count("strained"), v.count("settled"), name, len(fixed),
                     v.count(None)))
    for x_, s_, name, n, u_ in sorted(rows, key=lambda r: (-r[0], -(r[0] + r[1]), r[2])):
        print(f"    {name:<20}{n:6d}{s_:9d}{x_:10d}{u_:11d}")
    if rows and not any(r[0] + r[1] for r in rows):
        print("\n    Every class is entirely unjudged, so this table says nothing yet —")
        print("    it is the shape the next month fills in, not a reading.")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cache", help="issues JSON from a previous gh call")
    ap.add_argument("--bands", type=int, default=4, help="time slices (default 4)")
    ap.add_argument("--months", type=int, default=6,
                    help="months of contract-pressure history (default 6)")
    a = ap.parse_args()

    issues = load(a.cache)
    bugs = [i for i in issues if any(l["name"] == "bug" for l in i["labels"])]
    if not bugs:
        sys.exit("no `bug`-labelled issues found")
    bands = band_edges(bugs, a.bands)
    lo, hi = bands[0][0], bands[-1][1] - 1

    print(f"\n=== 1. Population ===")
    print(f"  {len(bugs)} bug issues, #{lo}-#{hi}   "
          f"({sum(1 for i in issues if i['state'] == 'OPEN')} open overall)")
    print(f"  bucketed by issue number into {a.bands} bands of "
          f"~{bands[0][1] - bands[0][0]}")

    rows, counts, tot, hits = class_trends(bugs, bands)

    print(f"\n=== 2. Mechanism class share by band (RISING = still firing) ===")
    hdr = "  " + "class".ljust(20) + "".join(f"#{b[0]}-{b[1]}".rjust(13) for b in bands) + "   trend"
    print(hdr + "\n  " + "-" * (len(hdr) - 2))
    for delta, name, sh, c, peak in sorted(rows, reverse=True):
        cells = "".join(f"{c[j]:3d} ({sh[j]:4.1f}%)".rjust(13) for j in range(len(bands)))
        print(f"  {name:<20}{cells}   {trend_mark(delta)} {delta:+.1f}pp vs peak {peak:.1f}%")

    print(f"\n=== 3. Payoff check — did each landed keystone move its class? ===")
    for keystone, cls, landed, *plan in KEYSTONES:
        if cls not in counts:
            print(f"  {keystone:<32} {cls}: no class signature — add one to CLASSES")
            continue
        before = [b for b in bands if b[1] <= landed]
        after = [b for b in bands if b[0] >= landed]
        if not before or not after:
            # Bands are equal-WIDTH in issue number, so a keystone landing inside the last
            # band has no band starting above it and cannot be judged.  Say what to do about
            # it rather than only that it happened: a finer slicing may reach it, and if it
            # does not, the honest answer is that the window has not passed yet.
            print(f"  {keystone:<32} landed at #{landed}: no band starts above it — "
                  f'try `make bug-review ARGS="--bands {max(a.bands * 2, 14)}"`, '
                  f"or wait for the next cycle")
            continue
        nb = sum(counts[cls][b] for b in before)
        sb = 100 * nb / max(1, sum(tot[b] for b in before))
        sa = 100 * sum(counts[cls][b] for b in after) / max(1, sum(tot[b] for b in after))
        verdict = payoff_verdict(sb, nb, sa, sum(tot[b] for b in after))
        print(f"  {keystone:<32} {cls:<18} {sb:5.1f}% -> {sa:5.1f}%   {verdict}")
        if plan and plan[0]:
            tag = plan[0]
            own = {i["number"] for i in bugs if tag in (i.get("body") or "")}
            cls_nums = {i["number"] for i in hits.get(cls, [])}

            def share(bs, own=own, cls_nums=cls_nums):
                inb = [i["number"] for i in bugs
                       if any(b[0] <= i["number"] < b[1] for b in bs)
                       and i["number"] not in own]
                return 100 * sum(1 for n in inb if n in cls_nums) / max(1, len(inb))

            removed = sum(1 for n in own if any(b[0] <= n < b[1] for b in after))
            label = f"minus {tag}'s finds"
            print(f"  {'':<32} {label:<18} {share(before):5.1f}% -> {share(after):5.1f}%   "
                  f"({removed} of its own finds removed from the after window)")

    present, total = walker_omissions()
    print(f"\n=== 4. Enumeration exposure ({total} partial walkers scanned) ===")
    print("  a variant is dangerous when it is BOTH often-omitted and often-used")
    print(f"  {'variant':<12}{'omitted':>9}{'bugs':>7}")
    bugcount = {"Tuple": len(hits.get("tuple", [])),
                "Parallel": len(hits.get("par/coroutine", [])),
                "Yield": len(hits.get("par/coroutine", []))}
    for v in sorted(CHILD_BEARING, key=lambda v: -(total - present[v])):
        om = 100 * (total - present[v]) / total if total else 0
        bc = bugcount.get(v)
        print(f"  {v:<12}{om:8.1f}%{(str(bc) if bc is not None else '-'):>7}")
    print()

    contract_pressure(issues, a.months)


if __name__ == "__main__":
    main()
