#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Which rewrites say WHY — the reasons a rewrite's decision can be read by, per rule.

    scripts/reason_census.py              # the table: per rule, where it fires, refuses, admits
    scripts/reason_census.py --missing    # only the SILENT rules: they fire and say nothing of a refusal
    scripts/reason_census.py --check      # fail on a rule that fell silent, and on a bad marker
    scripts/reason_census.py --record     # write tests/reason_census.baseline
    scripts/reason_census.py --self-test  # the scanner on a sample: each state, each refused marker

A rewrite's EFFECT is readable from outside by its `LOFT_NO_*` switch and by the rewrite
census (`rewrite_census::fired`).  Its REASONS are readable only where the rewrite reports
them (`formal/facts.md`, `(Fact-Oracle)` in words): `rewrite_census::declined(rule, site,
why)` names why a site it is shaped for was refused, and `rewrite_census::admitted(rule,
site)` names the site it took, so the two can be set against each other.  This counts the
call sites of the three in `src/`, by rule — a property of the source, so it needs no build
and no program.  An admission carries a site and no reason: the column says where a reason
could be attached once the call takes one.

Five states per rule that fires, because a zero alone cannot tell them apart:

  reports   it calls `declined` somewhere: its refusals are readable;
  partly    it calls `declined` at some of its gates and carries a `@decline-owed` marker
            naming the ones that say nothing yet: it stays on the `--owed` worklist;
  nothing   it CANNOT refuse a site, and says so where it fires —

                // @no-decline: fires wherever the shape occurs; no condition to refuse on
                crate::rewrite_census::fired("R-Example", 1);

            the marker sits in the comment lines directly above a `fired` call of the rule
            and speaks for the rule; its reason is required, so the claim can be disputed;
  owes      it CAN refuse, somebody read it and wrote down on what, and it does not report
            it yet — `// @decline-owed: <the conditions it refuses on>`, in the same place;
            the instrumentation backlog, with its work already named;
  silent    it fires and nobody has classified it: the reading backlog.

`--missing` lists the silent rules, `--owed` the ones that owe, in whole or in part.  A marker
with no text, one above no `fired` call, a `@no-decline` on a rule that also calls `declined`,
and both markers on one rule are each refused by `--check`.

A rule that reports to NO census is read from the formal chapter, which says why in its
phase table (`formal/rewrites.md`: `not counted — <kind>: <why>`, the kinds `executor`,
`runtime`, `condition`, `meta` and `unbuilt`); one the table does not explain is listed as
unexplained and refused by `--check`.

A rule's name is the first argument: a literal, or a `const … : &str` of the same file.  A
clause (`R-PushFill/loop`) counts under its rule.  A call whose rule this cannot read is
listed, never guessed.
"""
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SRC = os.path.join(ROOT, "src")
BASELINE = os.path.join(ROOT, "tests", "reason_census.baseline")
CALL = re.compile(r"rewrite_census::(fired|declined_in|declined|admitted)\(\s*([^,)]+)")
# The same call after `use crate::rewrite_census::fired;`: bare, its rule a literal.
BARE = re.compile(r'(?<![A-Za-z0-9_:.])(fired|declined_in|declined|admitted)\(\s*("[A-Z][A-Za-z]*-[^"]+")')
NOT_COUNTED = re.compile(r"^\| `\((R-[A-Za-z0-9]+)\)` \| not counted — ([a-z]+): (.*?) \|\s*$")
CONST = re.compile(r'const\s+([A-Z_][A-Z0-9_]*)\s*:\s*&(?:\'static\s+)?str\s*=\s*"([^"]+)"')
LITERAL = re.compile(r'"([A-Z][A-Za-z]*-[A-Za-z0-9/-]+)"')
MARK = re.compile(r"//\s*@(no-decline|decline-owed):[ \t]*(.*)")
KINDS = ("fired", "declined", "admitted")


def rules():
    """The rewrite rules the formal chapter defines (`R-…`), in its order."""
    out = subprocess.run(
        [sys.executable, os.path.join(ROOT, "scripts", "rule_tags.py"), "list"],
        capture_output=True, text=True, check=True, encoding="utf-8",
    ).stdout
    return [l.split()[0][len("@FR-"):] for l in out.splitlines() if l.startswith("@FR-R-")]


def rule_of(text, m, consts):
    """The rule a census call names, or `None` where the argument is not a readable name."""
    arg = m.group(2).strip()
    if arg.startswith('"'):
        return arg.strip('"')
    if arg in consts:
        return consts[arg]
    if arg.startswith("if ") and (lit := LITERAL.search(text, m.start(2), m.start(2) + 200)):
        # `fired(if clause { "R-X/clause" } else { "R-X" }, n)`: one rule either way.
        return lit.group(1)
    return None


def marker_above(lines, at):
    """The marker in the comment lines directly above line index `at`: `(line, kind, text)`,
    the text carried over the comment lines that continue it (indented further than `//`)."""
    k = at - 1
    while k >= 0 and lines[k].lstrip().startswith("//"):
        m = MARK.search(lines[k])
        if m:
            text = m.group(2).strip()
            j = k + 1
            while j < at and lines[j].lstrip().startswith("//   "):
                text += " " + lines[j].lstrip()[2:].strip()
                j += 1
            return k, m.group(1), text
        k -= 1
    return None


def scan_text(text, path, counts, marks, unread, orphans):
    """One file's census calls and markers, added to the four collections."""
    consts = dict(CONST.findall(text))
    lines = text.split("\n")
    attached = set()
    calls = sorted(list(CALL.finditer(text)) + list(BARE.finditer(text)), key=lambda m: m.start())
    for m in calls:
        # `declined_in` is `declined` with the site key built from a function and a local.
        kind = "declined" if m.group(1) == "declined_in" else m.group(1)
        at = text.count("\n", 0, m.start())
        rule = rule_of(text, m, consts)
        if rule is None:
            unread.append((path, at + 1, kind, m.group(2).strip()))
            continue
        base = rule.partition("/")[0]
        counts.setdefault(base, dict.fromkeys(KINDS, 0))[kind] += 1
        if kind == "fired" and (found := marker_above(lines, at)):
            attached.add(found[0])
            marks.setdefault(base, []).append((path, found[0] + 1, found[2], found[1]))
    for k, line in enumerate(lines):
        if MARK.search(line) and k not in attached:
            orphans.append((path, k + 1))


def scan():
    """Over `src/`: `{rule: {kind: count}}`, `{rule: [(path, line, why)]}` for the markers,
    the calls whose rule is not a readable name, and the markers above no `fired` call."""
    counts, marks, unread, orphans = {}, {}, [], []
    for folder, _, files in os.walk(SRC):
        for name in sorted(files):
            if not name.endswith(".rs") or name == "rewrite_census.rs":
                continue
            path = os.path.join(folder, name)
            with open(path, encoding="utf-8") as fh:
                text = fh.read()
            scan_text(text, os.path.relpath(path, ROOT), counts, marks, unread, orphans)
    return counts, marks, unread, orphans


def not_counted():
    """`{rule: (kind, why)}` — the rules the formal chapter's phase table says no census counts."""
    out = {}
    path = os.path.join(ROOT, "doc", "claude", "formal", "rewrites.md")
    with open(path, encoding="utf-8") as fh:
        for line in fh:
            m = NOT_COUNTED.match(line)
            if m:
                out[m.group(1)] = (m.group(2), m.group(3))
    return out


def state(fired, declined, marked):
    """`reports`, `partly`, `nothing` (to report), `owes`, `silent`, or `-` for a rule that
    does not fire.  `marked` is the rule's marker sites, or a false value."""
    owed = any(kind == "decline-owed" for _, _, _, kind in marked or [])
    if declined:
        return "partly" if owed else "reports"
    if not fired:
        return "-"
    if not marked:
        return "silent"
    return "owes" if owed else "nothing"


def marker_faults(counts, marks, orphans):
    """What `--check` refuses about the markers themselves."""
    out = []
    for rule, sites in sorted(marks.items()):
        for path, line, why, kind in sites:
            if not why:
                out.append(f"{path}:{line}: `@{kind}` on {rule} gives no reason")
        path, line, _, kind = sites[0]
        if counts.get(rule, {}).get("declined") and any(k == "no-decline" for _, _, _, k in sites):
            out.append(f"{path}:{line}: {rule} is marked `@no-decline` and also calls `declined`")
        if len({k for _, _, _, k in sites}) > 1:
            out.append(f"{path}:{line}: {rule} is marked both `@no-decline` and `@decline-owed`")
    for path, line in orphans:
        out.append(f"{path}:{line}: a marker is not directly above a `rewrite_census::fired` call")
    return out


def rows(defined, counts, marks):
    """One row per defined rule, then the names the source reports that the chapter lacks:
    `(rule, fired, declined, admitted, known, state)`."""
    extra = sorted(r for r in counts if r not in defined)
    for rule in list(defined) + extra:
        c = counts.get(rule, dict.fromkeys(KINDS, 0))
        st = state(c["fired"], c["declined"], marks.get(rule))
        yield rule, c["fired"], c["declined"], c["admitted"], rule in defined, st


def read_baseline():
    """`{rule: state}` as recorded; a row of the older form (counts only) reads its state off them."""
    base = {}
    if os.path.exists(BASELINE):
        with open(BASELINE, encoding="utf-8") as fh:
            for line in fh:
                if line.startswith("#") or not line.strip():
                    continue
                cols = line.rstrip("\n").split("\t")
                fired, declined = int(cols[1]), int(cols[2])
                base[cols[0]] = cols[4] if len(cols) > 4 else state(fired, declined, False)
    return base


SAMPLE = '''
const RULE: &str = "R-Const/clause";
fn a() {
    crate::rewrite_census::fired("R-Silent", 1);
    crate::rewrite_census::fired(RULE, n);
    crate::rewrite_census::declined(RULE, site, why);
    // The shape is its own condition.
    // @no-decline: fires wherever the shape occurs
    crate::rewrite_census::fired("R-Nothing", 1);
    // @no-decline:
    crate::rewrite_census::fired("R-NoWhy", 1);
    // @no-decline: but it does refuse
    crate::rewrite_census::fired("R-Both", 1);
    crate::rewrite_census::declined("R-Both", site, why);
    // @decline-owed: refuses a place written between the two reads,
    //   and one its callee may free
    crate::rewrite_census::fired("R-Owes", 1);
    // @no-decline: one thing
    crate::rewrite_census::fired("R-Twice", 1);
    // @decline-owed: and its opposite
    crate::rewrite_census::fired("R-Twice", 1);
    // @no-decline: above nothing
    let x = 1;
    crate::rewrite_census::fired(name_at_run_time, 1);
    use crate::rewrite_census::fired;
    fired("R-Bare", 1);
    crate::rewrite_census::fired("R-In", 1);
    crate::rewrite_census::declined_in("R-In", function, local, why);
    not_fired("R-Other", 1);
    // @decline-owed: its second gate
    crate::rewrite_census::fired("R-Partly", 1);
    crate::rewrite_census::declined("R-Partly", site, why);
}
'''


def self_test():
    counts, marks, unread, orphans = {}, {}, [], []
    scan_text(SAMPLE, "sample.rs", counts, marks, unread, orphans)
    got = {r: state(c["fired"], c["declined"], marks.get(r)) for r, c in counts.items()}
    want = {"R-Silent": "silent", "R-Const": "reports", "R-Nothing": "nothing",
            "R-NoWhy": "nothing", "R-Both": "reports", "R-Owes": "owes", "R-Twice": "owes",
            "R-Bare": "silent", "R-In": "reports", "R-Partly": "partly"}
    faults = marker_faults(counts, marks, orphans)
    owed = marks.get("R-Owes", [("", 0, "", "")])[0][2]
    ok = (got == want and len(unread) == 1 and len(orphans) == 1 and len(faults) == 4
          and owed == "refuses a place written between the two reads, and one its callee may free"
          and any("gives no reason" in f for f in faults)
          and any("also calls `declined`" in f for f in faults)
          and any("marked both" in f for f in faults)
          and any("not directly above" in f for f in faults))
    if not ok:
        print("reason_census self-test FAILED")
        print("  states:", got)
        print("  unread:", unread, "orphans:", orphans)
        for f in faults:
            print("  fault:", f)
        return 1
    print("reason_census self-test: ok (5 states, 4 refused markers, 1 unread call)")
    return 0


def main():
    args = sys.argv[1:]
    if "--self-test" in args:
        return self_test()
    defined = rules()
    counts, marks, unread, orphans = scan()
    table = list(rows(defined, counts, marks))
    faults = marker_faults(counts, marks, orphans)
    uncounted = not_counted()
    quiet_rules = [r for r, f, d, a, known, _ in table if known and not (f or d or a)]
    for r in quiet_rules:
        if r not in uncounted:
            faults.append(f"{r}: reports to no census and formal/rewrites.md's phase table "
                          "does not say why (`not counted — <kind>: <why>`)")
    if "--record" in args:
        with open(BASELINE, "w", encoding="utf-8", newline="\n") as fh:
            fh.write("# Per rewrite rule: the call sites in src/ that report it fired, why it refused a\n")
            fh.write("# site, and which site it admitted, and its state (scripts/reason_census.py;\n")
            fh.write("# --check: a rule never falls silent).  rule<TAB>fired<TAB>declined<TAB>admitted<TAB>state\n")
            for rule, f, d, a, _, st in table:
                if f or d or a:
                    fh.write(f"{rule}\t{f}\t{d}\t{a}\t{st}\n")
        print(f"recorded {os.path.relpath(BASELINE, ROOT)}")
        return 0
    if "--check" in args:
        base = read_baseline()
        now = {rule: st for rule, _, _, _, _, st in table}
        lost = [f"{rule}: was `{was}`, now says nothing of its refusals"
                for rule, was in sorted(base.items())
                if was in ("reports", "partly", "nothing", "owes")
                and now.get(rule, "-") in ("silent", "-")]
        if lost or faults:
            print("the reason census refuses (scripts/reason_census.py):")
            for l in lost + faults:
                print("  " + l)
            return 1
        return 0
    fires = [r for r in table if r[1] and r[4]]
    by_state = {st: [r for r in fires if r[5] == st]
                for st in ("reports", "partly", "nothing", "owes", "silent")}
    with_admit = [r for r in fires if r[3]]
    only_missing = "--missing" in args
    only_owed = "--owed" in args
    print(f"{'rule':<24} {'fires':>5} {'refusal reasons':>15} {'admitted sites':>14}  state")
    for rule, f, d, a, known, st in table:
        if not (f or d or a):
            continue
        if (only_missing and st != "silent") or (only_owed and st not in ("owes", "partly")):
            continue
        shown = {"nothing": "nothing to report", "-": "does not fire",
                 "owes": "owes its reasons", "partly": "reports in part"}.get(st, st)
        mark = "" if known else "   (a rule of another chapter)"
        print(f"{rule:<24} {f:>5} {d:>15} {a:>14}  {shown}{mark}")
    for rule, sites in sorted(marks.items()):
        path, line, why, kind = sites[0]
        owes = kind == "decline-owed"
        if only_missing or (only_owed and not owes):
            continue
        partly = owes and counts.get(rule, {}).get("declined")
        label = "still owes" if partly else "refuses on" if owes else "nothing to report"
        print(f"  {rule}: {label} — {why or '(no reason given)'}  [{path}:{line}]")
    kinds = {}
    for r in quiet_rules:
        kind, why = uncounted.get(r, ("unexplained", ""))
        kinds[kind] = kinds.get(kind, 0) + 1
        if not only_missing and not only_owed:
            print(f"{r:<24} {'-':>5} {'-':>15} {'-':>14}  not counted — {kind}: {why}")
    print()
    split = ", ".join(f"{n} {k}" for k, n in sorted(kinds.items()))
    print(f"{len(defined)} rewrite rules defined; {len(fires)} report where they fire, "
          f"{len(quiet_rules)} are counted by no census ({split})")
    print(f"of the {len(fires)}: {len(by_state['reports'])} report why they refuse a site, "
          f"{len(by_state['partly'])} report in part, "
          f"{len(by_state['nothing'])} state they have nothing to report, "
          f"{len(by_state['owes'])} owe their reasons, {len(by_state['silent'])} are unread")
    print(f"{len(with_admit)} name the site they admit; an admission carries no reason: "
          "`rewrite_census::admitted` takes none")
    for f in faults:
        print("  refused marker: " + f)
    if unread:
        print()
        print(f"{len(unread)} call(s) whose rule is not a literal or a const of the file:")
        for path, line, kind, arg in unread:
            print(f"  {path}:{line}  {kind}({arg}, …)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
