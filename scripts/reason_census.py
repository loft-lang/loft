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

Three states per rule that fires, because a zero alone cannot tell them apart:

  reports   it calls `declined` somewhere: its refusals are readable;
  nothing   it CANNOT refuse a site, and says so where it fires —

                // @no-decline: fires wherever the shape occurs; no condition to refuse on
                crate::rewrite_census::fired("R-Example", 1);

            the marker sits in the comment lines directly above a `fired` call of the rule
            and speaks for the rule; its reason is required, so the claim can be disputed;
  silent    it fires, it may refuse, and nothing says why: the backlog `--missing` lists.

A marker with no reason, one above no `fired` call, and one on a rule that also calls
`declined` are each refused by `--check`.

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
CALL = re.compile(r"rewrite_census::(fired|declined|admitted)\(\s*([^,)]+)")
CONST = re.compile(r'const\s+([A-Z_][A-Z0-9_]*)\s*:\s*&(?:\'static\s+)?str\s*=\s*"([^"]+)"')
LITERAL = re.compile(r'"([A-Z][A-Za-z]*-[A-Za-z0-9/-]+)"')
MARK = re.compile(r"//\s*@no-decline:[ \t]*(.*)")
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
    """The `@no-decline` marker in the comment lines directly above line index `at`."""
    k = at - 1
    while k >= 0 and lines[k].lstrip().startswith("//"):
        m = MARK.search(lines[k])
        if m:
            return k, m.group(1).strip()
        k -= 1
    return None


def scan_text(text, path, counts, marks, unread, orphans):
    """One file's census calls and markers, added to the four collections."""
    consts = dict(CONST.findall(text))
    lines = text.split("\n")
    attached = set()
    for m in CALL.finditer(text):
        kind = m.group(1)
        at = text.count("\n", 0, m.start())
        rule = rule_of(text, m, consts)
        if rule is None:
            unread.append((path, at + 1, kind, m.group(2).strip()))
            continue
        base = rule.partition("/")[0]
        counts.setdefault(base, dict.fromkeys(KINDS, 0))[kind] += 1
        if kind == "fired" and (found := marker_above(lines, at)):
            attached.add(found[0])
            marks.setdefault(base, []).append((path, found[0] + 1, found[1]))
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


def state(fired, declined, marked):
    """`reports`, `nothing` (to report), `silent`, or `-` for a rule that does not fire."""
    if declined:
        return "reports"
    if not fired:
        return "-"
    return "nothing" if marked else "silent"


def marker_faults(counts, marks, orphans):
    """What `--check` refuses about the markers themselves."""
    out = []
    for rule, sites in sorted(marks.items()):
        for path, line, why in sites:
            if not why:
                out.append(f"{path}:{line}: `@no-decline` on {rule} gives no reason")
        if counts.get(rule, {}).get("declined"):
            path, line, _ = sites[0]
            out.append(f"{path}:{line}: {rule} is marked `@no-decline` and also calls `declined`")
    for path, line in orphans:
        out.append(f"{path}:{line}: `@no-decline` is not directly above a `rewrite_census::fired` call")
    return out


def rows(defined, counts, marks):
    """One row per defined rule, then the names the source reports that the chapter lacks:
    `(rule, fired, declined, admitted, known, state)`."""
    extra = sorted(r for r in counts if r not in defined)
    for rule in list(defined) + extra:
        c = counts.get(rule, dict.fromkeys(KINDS, 0))
        st = state(c["fired"], c["declined"], rule in marks)
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
    // @no-decline: above nothing
    let x = 1;
    crate::rewrite_census::fired(name_at_run_time, 1);
}
'''


def self_test():
    counts, marks, unread, orphans = {}, {}, [], []
    scan_text(SAMPLE, "sample.rs", counts, marks, unread, orphans)
    got = {r: state(c["fired"], c["declined"], r in marks) for r, c in counts.items()}
    want = {"R-Silent": "silent", "R-Const": "reports", "R-Nothing": "nothing",
            "R-NoWhy": "nothing", "R-Both": "reports"}
    faults = marker_faults(counts, marks, orphans)
    ok = (got == want and len(unread) == 1 and len(orphans) == 1 and len(faults) == 3
          and any("gives no reason" in f for f in faults)
          and any("also calls `declined`" in f for f in faults)
          and any("not directly above" in f for f in faults))
    if not ok:
        print("reason_census self-test FAILED")
        print("  states:", got)
        print("  unread:", unread, "orphans:", orphans)
        for f in faults:
            print("  fault:", f)
        return 1
    print("reason_census self-test: ok (3 states, 3 refused markers, 1 unread call)")
    return 0


def main():
    args = sys.argv[1:]
    if "--self-test" in args:
        return self_test()
    defined = rules()
    counts, marks, unread, orphans = scan()
    table = list(rows(defined, counts, marks))
    faults = marker_faults(counts, marks, orphans)
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
                if was in ("reports", "nothing") and now.get(rule, "-") in ("silent", "-")]
        if lost or faults:
            print("the reason census refuses (scripts/reason_census.py):")
            for l in lost + faults:
                print("  " + l)
            return 1
        return 0
    fires = [r for r in table if r[1] and r[4]]
    by_state = {st: [r for r in fires if r[5] == st] for st in ("reports", "nothing", "silent")}
    with_admit = [r for r in fires if r[3]]
    only_missing = "--missing" in args
    print(f"{'rule':<24} {'fires':>5} {'refusal reasons':>15} {'admitted sites':>14}  state")
    for rule, f, d, a, known, st in table:
        if not (f or d or a):
            continue
        if only_missing and st != "silent":
            continue
        shown = {"nothing": "nothing to report", "-": "does not fire"}.get(st, st)
        mark = "" if known else "   (a rule of another chapter)"
        print(f"{rule:<24} {f:>5} {d:>15} {a:>14}  {shown}{mark}")
    for rule, sites in sorted(marks.items()):
        if not only_missing:
            path, line, why = sites[0]
            print(f"  {rule}: nothing to report — {why or '(no reason given)'}  [{path}:{line}]")
    quiet = sum(1 for r in table if r[4] and not (r[1] or r[2] or r[3]))
    print()
    print(f"{len(defined)} rewrite rules defined; {len(fires)} report where they fire, "
          f"{quiet} report to no census")
    print(f"of the {len(fires)}: {len(by_state['reports'])} report why they refuse a site, "
          f"{len(by_state['nothing'])} state they have nothing to report, "
          f"{len(by_state['silent'])} are silent")
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
