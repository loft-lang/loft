#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""How far each rewrite REACHES — the sites it admitted against the sites it refused, and why.

    scripts/decline_census.py              # the benches: per rule, admitted / declined / the reasons
    scripts/decline_census.py --only 14_stdlib_vector,16_consumer_shapes
    scripts/decline_census.py --reasons 8  # more reasons per rule (default 4)
    scripts/decline_census.py --programs   # also the per-program split
    scripts/decline_census.py --record     # ...and keep the run in bench/portal/decline_census.tsv
    LOFT_BIN=<loft> scripts/decline_census.py

Why
---
A routine over its bar is a shape the pattern matching did not reach: a loop or an access
that fell back to the full store protocol because a rewrite declined it.  The rewrite census
(`scripts/rewrite_census.py`) counts the ADMISSIONS and fails when one is lost; it cannot say
how much of what a rule is written for the rule covers, nor which condition holds the rest
back.  This report is the complement: for each rule, the sites it was shaped for, split into
the ones it rewrote and the ones it refused, and the refusals ranked by reason.  A reason near
the top of a rule that matters is the next arc, chosen by yield rather than by which routine
happens to be over the bar — and since a declined pattern usually sits in many routines, it
also says which neighbours an arc will lift.

How
---
The same body of programs as the rewrite census (every `bench/NN_*/bench.loft` lane and the
portal libraries' benches wherever a checkout exists), each emitted once to native Rust with
`LOFT_REWRITE_CENSUS` and `LOFT_DECLINE_CENSUS` set (`src/rewrite_census.rs`).  The compiler
counts a decline ONCE per `(rule, site)`, under the first reason it gave, so a site asked
about by several probes is one refusal and the count is comparable with the admissions.  A
reason is read with its names masked — `‘x’` and digits become `_` — so that one
condition reads as one row across every site that met it.

Only the rules that report their declines appear with a share; a rule with admissions and no
decline instrumentation is listed as `not reported` rather than as fully covered, and one that
reports declines at some of its refusal sites reads `(at most)`.  A site one probe refuses and
another admits is in both counts, so every share is a floor.  A REPORT, never a gate: it has no
baseline and nothing fails on it.
"""

import os
import re
import subprocess
import sys
import tempfile
from collections import defaultdict
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RECORD = ROOT / "bench" / "portal" / "decline_census.tsv"
sys.path.insert(0, str(ROOT / "scripts"))
from rewrite_census import programs, warm_dependencies  # noqa: E402

# The rules whose declines the compiler reports (`rewrite_census::declined` call sites).
# A rule absent here with admissions present is shown as `not reported`.
REPORTED = {
    "R-PushFill": ("the push window of a counted push loop", "complete"),
    "R-PushFill/loop": ("the counted push loop itself (reserve and fill)", "declines only"),
    "R-Header": ("a loop's vector headers held across the body", "partial"),
    "R-Header/scalars": ("the scalar hoists of a loop", "declines only"),
    "R-RecPtr": ("a held record address for a view", "complete"),
    "R-ValueRecord": ("a record type kept as a value", "partial"),
}


def mask(why):
    """One condition as one row: names in backticks and numbers become `_`."""
    why = re.sub(r"`[^`]*`", "`_`", why)
    return re.sub(r"\d+", "_", why)


def census(loft, prog):
    name, commit, cwd, argv = prog
    with tempfile.TemporaryDirectory() as tmp:
        adm, dec, out = (Path(tmp) / f for f in ("adm.tsv", "dec.tsv", "out.rs"))
        proc = subprocess.run(
            [loft, "--native-emit", str(out), "--lean", *argv],
            cwd=cwd, capture_output=True, text=True, timeout=300,
            env={**os.environ, "LOFT_NO_NATIVE_LIBS": "1",
                 "LOFT_REWRITE_CENSUS": str(adm), "LOFT_DECLINE_CENSUS": str(dec)},
        )
        if proc.returncode != 0 or not adm.exists():
            return name, None, None, proc.stderr.strip().splitlines()[-1:] or ["no output"]
        admitted = {}
        for line in adm.read_text().splitlines():
            rule, _, n = line.partition("\t")
            admitted[rule] = int(n)
        declined = defaultdict(int)
        if dec.exists():
            for line in dec.read_text().splitlines():
                rule, why, n = line.split("\t")
                if why == "admitted":
                    # The rule names its sites: one per loop or view, comparable with the
                    # declines, where the rewrite census counts every emitted copy.
                    admitted[rule] = int(n)
                else:
                    declined[(rule, mask(why))] += int(n)
        return name, admitted, dict(declined), None


def family(rule):
    return rule.split("/")[0]


def report(results, reasons, per_program):
    admitted, declined = defaultdict(int), defaultdict(int)
    by_rule_reason = defaultdict(int)
    for _, adm, dec, _ in results:
        for rule, n in adm.items():
            admitted[rule] += n
        for (rule, why), n in dec.items():
            declined[rule] += n
            by_rule_reason[(rule, why)] += n
    rules = sorted(set(admitted) | set(declined), key=lambda r: (family(r), r))
    lines = [f"decline census: {len(results)} programs, "
             f"{sum(admitted.values())} admissions (sites, for a rule that names them; else "
             f"the rewrite census's count), {sum(declined.values())} declines "
             f"over {len(REPORTED)} reporting rules"]
    lines.append(f"{'rule':<28} {'admitted':>8} {'declined':>8} {'reach':>6}  what the rule is shaped for")
    for rule in rules:
        a, d = admitted.get(rule, 0), declined.get(rule, 0)
        if rule in REPORTED:
            what, how = REPORTED[rule]
            # `partial`: declines reported at some of the rule's refusal sites, so the reach is
            # at most this; `declines only`: no admission is counted under the name.
            reach = "    -" if how == "declines only" or not a + d else f"{100 * a / (a + d):5.0f}%"
            mark = {"partial": " (at most)", "declines only": ""}.get(how, "")
            lines.append(f"{rule:<28} {a if how != 'declines only' else '-':>8} {d:>8} {reach:>6}  {what}{mark}")
            top = sorted(((n, w) for (r, w), n in by_rule_reason.items() if r == rule), reverse=True)
            for n, why in top[:reasons]:
                lines.append(f"{'':<28} {'':>8} {n:>8}         {why}")
            if len(top) > reasons:
                lines.append(f"{'':<28} {'':>8} {sum(n for n, _ in top[reasons:]):>8}         "
                             f"({len(top) - reasons} more reasons)")
        else:
            lines.append(f"{rule:<28} {a:>8} {'':>8} {'':>6}  not reported")
    if per_program:
        lines.append("")
        lines.append(f"{'program':<28} {'rule':<28} {'admitted':>8} {'declined':>8}")
        for name, adm, dec, _ in sorted(results, key=lambda r: r[0]):
            per = defaultdict(int)
            for (rule, _), n in dec.items():
                per[rule] += n
            for rule in sorted(set(adm) | set(per)):
                if rule in REPORTED and (adm.get(rule, 0) or per.get(rule, 0)):
                    lines.append(f"{name:<28} {rule:<28} {adm.get(rule, 0):>8} {per.get(rule, 0):>8}")
    return "\n".join(lines), admitted, declined, by_rule_reason


def record(results, admitted, declined, by_rule_reason):
    head = subprocess.run(["git", "-C", str(ROOT), "rev-parse", "--short=9", "HEAD"],
                          capture_output=True, text=True).stdout.strip() or "?"
    lines = [
        "# The decline census: per rewrite rule, the sites admitted and the sites refused by reason,",
        "# over the bench programs.  Written by `scripts/decline_census.py --record`; a report, not a baseline.",
        f"# commit={head} programs={len(results)}",
        "# rule <TAB> reason (`admitted` for the admissions; names masked as `_`) <TAB> count",
    ]
    for rule in sorted(set(admitted) | set(declined)):
        if admitted.get(rule):
            lines.append(f"{rule}\tadmitted\t{admitted[rule]}")
        for (r, why), n in sorted(by_rule_reason.items()):
            if r == rule:
                lines.append(f"{rule}\t{why}\t{n}")
    RECORD.write_text("\n".join(lines) + "\n")


def main() -> int:
    args = sys.argv[1:]
    if "--help" in args or "-h" in args:
        print(__doc__.strip())
        return 0
    only = None
    if "--only" in args:
        only = set(args[args.index("--only") + 1].split(","))
    reasons = int(args[args.index("--reasons") + 1]) if "--reasons" in args else 4
    loft = os.environ.get("LOFT_BIN") or str(ROOT / "target" / "release" / "loft")
    progs = [p for p in programs() if only is None or p[0] in only]
    if not progs:
        print("decline_census: no program selected", file=sys.stderr)
        return 2
    warm_dependencies(loft, progs)
    with ThreadPoolExecutor(max_workers=4) as pool:
        done = list(pool.map(lambda p: census(loft, p), progs))
    results = [r for r in done if r[1] is not None]
    text, admitted, declined, by_rule_reason = report(results, reasons, "--programs" in args)
    print(text)
    for name, adm, _, err in done:
        if adm is None:
            print(f"  could not census {name}: {err[0]}")
    if "--record" in args:
        record(results, admitted, declined, by_rule_reason)
        print(f"wrote {RECORD.relative_to(ROOT)}")
    return 0 if len(results) == len(done) else 2


if __name__ == "__main__":
    sys.exit(main())
