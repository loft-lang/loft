#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Which phase applies each rewrite rule — measured, not written down.

    scripts/rule_phases.py               # the table, with where each rule is decided
    scripts/rule_phases.py --rule R-Header
    scripts/rule_phases.py --render      # write formal/rewrites.md's `## Applied by` from the table
    scripts/rule_phases.py --check-doc   # fail when that section is not what --render writes

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

`formal/rewrites.md` `## Applied by` is rendered from the table (`--render`), one row per
register entry, so the register states each rule's phase from the measurement and nowhere else;
`scripts/check_doc_drift.sh` fails when the section and the table disagree (`--check-doc`).
Decided-in is left out of the doc on purpose: it moves with any citation anywhere.
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


DOC = ROOT / "doc" / "claude" / "formal" / "rewrites.md"
BEGIN = "<!-- BEGIN applied-by: generated by scripts/rule_phases.py --render from bench/portal/rule_phases.tsv; do not edit -->"
END = "<!-- END applied-by -->"
PHASE_WORDS = {
    "parse": "the parser",
    "ir": "the scope pass, which both backends run",
    "interp": "the interpreter's bytecode generator",
    "c": "the C writer",
    "native": "the rustc writer",
}


def render(table):
    """The `## Applied by` section's body: every entry of the register, with the phases that
    apply each of its census names, or why the census does not count it."""
    sys.path.insert(0, str(ROOT / "scripts"))
    import rule_tags  # noqa: E402
    register = sorted(t for t, f in rule_tags.defined_rules().items() if "rewrites.md" in f)
    counted = {n.split("/")[0] for n in rule_tags.census_calls()}
    names = collections.defaultdict(list)
    for n in sorted(table):
        names[n.split("/")[0]].append(n)
    out = [BEGIN, "",
           "Per entry, each census name with the phases whose admissions over the bench",
           "programs are not zero; `make rewrite-census` fails when a set changes.  The phases:",
           ""]
    out += [f"- `{k}`: {v}" for k, v in PHASE_WORDS.items()]
    out += ["", "| rule | applied by |", "|---|---|"]
    for entry in register:
        if names.get(entry):
            cells = []
            for n in names[entry]:
                label = f"`{n}`" if n == entry else f"`/{n.split('/', 1)[1]}`"
                cells.append(f"{label} {', '.join(table[n]['applied'])}")
            cell = " · ".join(cells)
        elif entry in rule_tags.CENSUS_EXEMPT:
            kind, why = rule_tags.CENSUS_EXEMPT[entry]
            cell = f"not counted — {kind}: {why}"
        elif entry in counted:
            cell = "counted; admitted on no bench program"
        else:
            cell = "not counted (`rule_tags.py phases --check` names it)"
        out.append(f"| `({entry})` | {cell} |")
    out += ["", END]
    return "\n".join(out)


def doc_with(text, body):
    """`text` with its generated section replaced by `body`, or None when it has no markers."""
    a, b = text.find(BEGIN), text.find(END)
    if a < 0 or b < 0:
        return None
    return text[:a] + body + text[b + len(END):]


def main():
    args = sys.argv[1:]
    table = read_table()
    if "--render" in args or "--check-doc" in args:
        text = DOC.read_text()
        want = doc_with(text, render(table))
        if want is None:
            print(f"{DOC.relative_to(ROOT)} has no applied-by markers")
            return 1
        if "--render" in args:
            if want != text:
                DOC.write_text(want)
                print(f"wrote {DOC.relative_to(ROOT)} § Applied by")
            return 0
        if want != text:
            print(f"{DOC.relative_to(ROOT)} § Applied by is not what bench/portal/rule_phases.tsv "
                  "says: `scripts/rule_phases.py --render`")
            return 1
        return 0
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
