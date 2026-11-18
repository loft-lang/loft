#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Which rewrites say WHY — the reasons a rewrite's decision can be read by, per rule.

    scripts/reason_census.py              # the table: per rule, where it fires, refuses, admits
    scripts/reason_census.py --missing    # only the rules that fire and give no refusal reason
    scripts/reason_census.py --check      # fail when a rule LOST a reason it had (the baseline)
    scripts/reason_census.py --record     # write tests/reason_census.baseline

A rewrite's EFFECT is readable from outside by its `LOFT_NO_*` switch and by the rewrite
census (`rewrite_census::fired`).  Its REASONS are readable only where the rewrite reports
them (`formal/facts.md`, `(Fact-Oracle)` in words): `rewrite_census::declined(rule, site,
why)` names why a site it is shaped for was refused, and `rewrite_census::admitted(rule,
site)` names the site it took, so the two can be set against each other.  This counts the
call sites of the three in `src/`, by rule — a property of the source, so it needs no build
and no program.  An admission carries a site and no reason: the column says where a reason
could be attached once the call takes one.

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
KINDS = ("fired", "declined", "admitted")


def rules():
    """The rewrite rules the formal chapter defines (`R-…`), in its order."""
    out = subprocess.run(
        [sys.executable, os.path.join(ROOT, "scripts", "rule_tags.py"), "list"],
        capture_output=True, text=True, check=True, encoding="utf-8",
    ).stdout
    return [l.split()[0][len("@FR-"):] for l in out.splitlines() if l.startswith("@FR-R-")]


def scan():
    """`{rule: {kind: count}}` over `src/`, and the calls whose rule is not a readable name."""
    counts, unread = {}, []
    for folder, _, files in os.walk(SRC):
        for name in sorted(files):
            if not name.endswith(".rs") or name == "rewrite_census.rs":
                continue
            path = os.path.join(folder, name)
            with open(path, encoding="utf-8") as fh:
                text = fh.read()
            consts = dict(CONST.findall(text))
            for m in CALL.finditer(text):
                kind, arg = m.group(1), m.group(2).strip()
                if arg.startswith('"'):
                    rule = arg.strip('"')
                elif arg in consts:
                    rule = consts[arg]
                elif arg.startswith("if ") and (lit := LITERAL.search(text, m.start(2), m.start(2) + 200)):
                    # `fired(if clause { "R-X/clause" } else { "R-X" }, n)`: one rule either way.
                    rule = lit.group(1)
                else:
                    line = text.count("\n", 0, m.start()) + 1
                    unread.append((os.path.relpath(path, ROOT), line, kind, arg))
                    continue
                base = rule.partition("/")[0]
                counts.setdefault(base, dict.fromkeys(KINDS, 0))[kind] += 1
    return counts, unread


def rows(defined, counts):
    """One row per defined rule, then the names the source reports that the chapter lacks."""
    extra = sorted(r for r in counts if r not in defined)
    for rule in list(defined) + extra:
        c = counts.get(rule, dict.fromkeys(KINDS, 0))
        yield rule, c["fired"], c["declined"], c["admitted"], rule in defined


def read_baseline():
    base = {}
    if os.path.exists(BASELINE):
        with open(BASELINE, encoding="utf-8") as fh:
            for line in fh:
                if line.startswith("#") or not line.strip():
                    continue
                rule, fired, declined, admitted = line.rstrip("\n").split("\t")[:4]
                base[rule] = (int(fired), int(declined), int(admitted))
    return base


def main():
    args = sys.argv[1:]
    defined = rules()
    counts, unread = scan()
    table = list(rows(defined, counts))
    if "--record" in args:
        with open(BASELINE, "w", encoding="utf-8", newline="\n") as fh:
            fh.write("# Per rewrite rule: the call sites in src/ that report it fired, why it refused a\n")
            fh.write("# site, and which site it admitted (scripts/reason_census.py; --check: a reason is\n")
            fh.write("# never lost).  rule<TAB>fired<TAB>declined<TAB>admitted\n")
            for rule, f, d, a, _ in table:
                if f or d or a:
                    fh.write(f"{rule}\t{f}\t{d}\t{a}\n")
        print(f"recorded {os.path.relpath(BASELINE, ROOT)}")
        return 0
    if "--check" in args:
        base = read_baseline()
        now = {rule: (f, d, a) for rule, f, d, a, _ in table}
        lost = []
        for rule, (_, d0, a0) in sorted(base.items()):
            _, d1, a1 = now.get(rule, (0, 0, 0))
            if d0 and not d1:
                lost.append(f"{rule}: its refusal reasons are gone ({d0} call site(s) -> 0)")
            if a0 and not a1:
                lost.append(f"{rule}: its admitted sites are gone ({a0} call site(s) -> 0)")
        if lost:
            print("a rewrite lost a reason it reported (scripts/reason_census.py):")
            for l in lost:
                print("  " + l)
            return 1
        return 0
    fires = [r for r in table if r[1] and r[4]]
    with_decline = [r for r in fires if r[2]]
    with_admit = [r for r in fires if r[3]]
    both = [r for r in fires if r[2] and r[3]]
    only_missing = "--missing" in args
    print(f"{'rule':<24} {'fires':>5} {'refusal reasons':>15} {'admitted sites':>14}")
    for rule, f, d, a, known in table:
        if not (f or d or a):
            continue
        if only_missing and (not f or d):
            continue
        mark = "" if known else "   (a rule of another chapter)"
        print(f"{rule:<24} {f:>5} {d:>15} {a:>14}{mark}")
    silent = sum(1 for r in table if r[4] and not (r[1] or r[2] or r[3]))
    print()
    print(f"{len(defined)} rewrite rules defined; {len(fires)} report where they fire, "
          f"{silent} report nothing")
    print(f"of the {len(fires)}: {len(with_decline)} give a reason for a refusal, "
          f"{len(with_admit)} name the site they admit, {len(both)} do both")
    print("an admission carries its site and no reason: `rewrite_census::admitted` takes none")
    if unread:
        print()
        print(f"{len(unread)} call(s) whose rule is not a literal or a const of the file:")
        for path, line, kind, arg in unread:
            print(f"  {path}:{line}  {kind}({arg}, …)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
