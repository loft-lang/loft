#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Which phase applies each rewrite rule — measured, not written down.

    scripts/rule_phases.py               # the table, with where each rule is decided
    scripts/rule_phases.py --rule R-Header

Why
---
`formal/rewrites.md` `(R-Phase)`: every rule is APPLIED BY one phase, and the phase decides its
oracle.  Prose beside a rule goes stale; the census does not.  `scripts/rewrite_census.py` writes
every bench program with both compiled writers and `LOFT_REWRITE_CENSUS_FN` set, which credits
each admission to the phase that made it — `parse`, `ir` (the scope pass), `interp` (the
interpreter's bytecode generator), `c` and `native` (the two writers) — and this module turns
that into `bench/portal/rule_phases.tsv`, one row per census name (`R-<Entry>` or
`R-<Entry>/<clause>`):

    name  applied-by  programs  parse  ir  interp  c  native

*applied-by* is the set of phases whose admissions are not zero over the bench programs.
`make rewrite-census` fails when a name's applied-by set CHANGES — a rule that moved phase, or
stopped being applied somewhere, is a change of the table that lands with `--bless` in its own
commit (@PLN211 arc C moves rules on purpose; nothing else should move one).  The admission
counts are recorded for reading; only the set is judged.

*decided-in* is not stored: it is read from the code each time, as the families of the source
files that cite the rule (`@FR-<Entry>`, `rule_tags.py sites`) — a shared analysis (`ir`),
the interpreter (`interp`), a writer (`c`, `rust`).  A rule decided in a shared analysis and
applied by a writer is the cheapest to move into the IR: only its emission changes home.
"""

import collections
import os
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TABLE = ROOT / "bench" / "portal" / "rule_phases.tsv"
# The phases, in the order the compiler runs them.  The first three run once per program
# whatever writer follows, so both writers' runs report the same counts for them.
PHASES = ("parse", "ir", "interp", "c", "native")
SHARED = ("parse", "ir", "interp")


def aggregate(results):
    """`results`: `(program, writer, {(phase, rule): admissions})` per census run →
    `{name: (Counter{phase: admissions}, {program, …})}`.  A shared phase is taken from each
    program once (both writers' runs carry it), a writer's phase from that writer's run."""
    shared = {}
    out = collections.defaultdict(lambda: (collections.Counter(), set()))
    for program, writer, split in results:
        for (phase, rule), n in split.items():
            if not n:
                continue
            if phase in SHARED:
                key = (program, phase, rule)
                shared[key] = max(shared.get(key, 0), n)
            else:
                out[rule][0][phase] += n
                out[rule][1].add(program)
    for (program, phase, rule), n in shared.items():
        out[rule][0][phase] += n
        out[rule][1].add(program)
    return dict(out)


def applied_by(counts):
    return [p for p in PHASES if counts.get(p, 0)]


def write_table(agg):
    lines = [
        "# Which phase applies each rewrite: per census name, the phases whose admissions over the",
        "# bench programs are not zero, and the admissions per phase.  Written by",
        "# `scripts/rewrite_census.py --bless`; `make rewrite-census` fails when a name's applied-by",
        "# set changes.  Read it with `scripts/rule_phases.py`.",
        "\t".join(("name", "applied-by", "programs") + PHASES),
    ]
    for name in sorted(agg):
        counts, progs = agg[name]
        lines.append("\t".join([name, ",".join(applied_by(counts)), str(len(progs))]
                               + [str(counts.get(p, 0)) for p in PHASES]))
    TABLE.write_text("\n".join(lines) + "\n")


def read_table(path=TABLE):
    """`{name: {"applied": [phase, …], "programs": n, phase: admissions, …}}`."""
    rows = {}
    if not Path(path).is_file():
        return rows
    header = None
    for line in Path(path).read_text().splitlines():
        if line.startswith("#") or not line.strip():
            continue
        parts = line.split("\t")
        if header is None:
            header = parts
            continue
        r = dict(zip(header, parts))
        rows[r["name"]] = {"applied": [p for p in r["applied-by"].split(",") if p],
                           "programs": int(r["programs"]),
                           **{p: int(r.get(p, 0)) for p in PHASES}}
    return rows


def changes(agg, table):
    """The names whose applied-by set differs from the table's, as messages."""
    out = []
    for name in sorted(set(agg) | set(table)):
        now = applied_by(agg[name][0]) if name in agg else []
        was = table[name]["applied"] if name in table else []
        if now == was:
            continue
        if not was:
            out.append(f"{name}: newly applied by {','.join(now)}")
        elif not now:
            out.append(f"{name}: applied by {','.join(was)} → applied nowhere on the benches")
        else:
            out.append(f"{name}: applied by {','.join(was)} → {','.join(now)}")
    return out


def family(path):
    """The code family a source file belongs to, for the decided-in column."""
    p = path.split("src/", 1)[-1]
    if p.startswith("c/"):
        return "c"
    if p.startswith(("generation/",)) or p in ("codegen_runtime.rs",):
        return "rust"
    if p.startswith(("state/",)) or p in ("fill.rs", "ops.rs", "stack.rs", "keys.rs"):
        return "interp"
    return "ir"


def decided_in(entries):
    """`{entry: [family, …]}` from the source files citing `@FR-<entry>` under `src/`."""
    sys.path.insert(0, str(ROOT / "scripts"))
    import rule_tags  # noqa: E402
    sites = rule_tags.citations_in([os.path.join(str(ROOT), "src")], [".rs", ".h"])
    out = {}
    for entry in entries:
        fams = {family(f) for f, _ in sites.get(entry, [])}
        out[entry] = [f for f in ("ir", "interp", "c", "rust") if f in fams]
    return out


def main():
    args = sys.argv[1:]
    table = read_table()
    if not table:
        print(f"no {TABLE.relative_to(ROOT)} yet: `scripts/rewrite_census.py --bless` writes it")
        return 1
    only = args[args.index("--rule") + 1] if "--rule" in args else None
    names = [n for n in sorted(table) if only is None or n.split("/")[0] == only]
    dec = decided_in({n.split("/")[0] for n in names})
    width = max((len(n) for n in names), default=10)
    print(f"{'name':<{width}}  {'decided in':<18} {'applied by':<18} {'programs':>8}  "
          + "  ".join(f"{p:>7}" for p in PHASES))
    for n in names:
        r = table[n]
        print(f"{n:<{width}}  {','.join(dec[n.split('/')[0]]) or '-':<18} "
              f"{','.join(r['applied']):<18} {r['programs']:>8}  "
              + "  ".join(f"{r[p]:>7}" for p in PHASES))
    return 0


if __name__ == "__main__":
    sys.exit(main())
