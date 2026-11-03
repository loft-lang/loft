#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Fail when a rewrite stopped firing where it used to — a tightened condition, caught exactly.

    scripts/rewrite_census.py            # census the benches, fail on any drop vs the baseline
    scripts/rewrite_census.py --bless    # …and write what it counted as the new baseline
    scripts/rewrite_census.py --bless-unless-drop   # one run that checks and records: fails on a
                                         #   drop (writing nothing), else writes the counts
    scripts/rewrite_census.py --only 12_drawing,drawing
    scripts/rewrite_census.py --writer c # one writer only (`rust`, `c`; default both)
    scripts/rewrite_census.py --verbose  # also list every rise and new row
    LOFT_BIN=<loft> scripts/rewrite_census.py   # census another build (default target/release/loft)

Why
---
Every rewrite in doc/claude/formal/rewrites.md is admitted under conditions, and a bug fix that
adds a decline can switch it off on real code.  The speed that costs is often inside the noise
of any timing (the test speed gate only sees 3x), yet the COUNT of admissions over a fixed body
of programs is a pure function of the compiler: a drop names the rule and the program that lost
it, and nothing but a compiler change can move it.

How
---
Each program is written by BOTH compiled writers without compiling the result — as Rust
(`--native-emit --lean`, no rustc) and as C (`--emit-c`, no C compiler), ~0.2 s each — with
`LOFT_REWRITE_CENSUS` set; the compiler counts each rewrite where it is ADMITTED
(`src/rewrite_census.rs`).  A row is per (program, writer, rule): a writer's count holds the
admissions decided before any writer (the parse and IR phases, which both runs repeat) and that
writer's own, so a clause one writer drops fails on that writer's row alone.  The body is the benches, the code whose speed is the goal: every
`bench/NN_*/bench.loft` lane in this repo, and the portal libraries' own benches wherever a
checkout exists (`bench/portal/libs.tsv`, `checkout_libs.sh`).  A library row carries the
checkout's commit and every dependency version its lane's `loft.lock` resolved
(`33aa4cc2c+cbor@0.1.12+crypto@0.3.11`); when either has moved since the baseline, its rows are
not judged — the library or what it compiles with changed, not the compiler.  The lock is no
tracked file of the library's, so two machines can hold two resolutions of one checkout, and a
row compared across them reported a drop no compiler change made.

Every `use`d library is compiled INTO the program (`LOFT_NO_NATIVE_LIBS=1`), so each program counts
the rewrites of every library function it reaches.  A library may otherwise run from a compiled
library of its own, valid only for the loft build that made it: a program emitted while that
library existed called into it and counted none of its functions, one emitted after a rebuild
counted them all, so a rule's count moved between programs from build to build with the total
unchanged (loft#1886).  With the libraries in the program, the count depends on the compiler and
the sources alone.

Each program is emitted TWICE per writer and the two sources must be the same bytes: the census needs a
deterministic emitter, and so does every native cache keyed on the source (a hash-ordered walk
once made a work-buffer function's exit frees change order from run to run).

The same run records WHICH PHASE made each admission (`LOFT_REWRITE_CENSUS_FN`) and checks the
phase table `bench/portal/rule_phases.tsv` (`scripts/rule_phases.py`): a rule whose set of
applying phases changed fails like a drop, and `--bless` rewrites it.  A partial run (`--only`,
`--writer`) cannot judge the table and leaves it alone.

The baseline is `bench/portal/rewrite_census.tsv`; a row of the older four-column form (no writer)
reads as the Rust writer's.  A DROP fails; a rise, a new program or a
new rule is printed and passes.  A deliberate decline lands with `--bless` in the same commit,
so the drop is visible in its review.  A program absent from this run (no checkout) is skipped.
"""

import os
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BASELINE = ROOT / "bench" / "portal" / "rewrite_census.tsv"
sys.path.insert(0, str(ROOT / "bench" / "portal"))
from portal import library_packages  # noqa: E402  (the one home of which libraries)
import rule_phases  # noqa: E402  (the phase table, beside this script)

# Each compiled writer: its name in the baseline, the source it writes, its flags.  `rust` is
# the rustc writer (census phase `native`), `c` the C writer (phase `c`).
WRITERS = {
    "rust": (".rs", lambda out: ["--native-emit", str(out), "--lean"]),
    "c": (".c", lambda out: ["--emit-c", str(out)]),
}


def programs():
    """(name, commit, cwd, argv-after-loft) for every program of the body present here."""
    out = []
    for d in sorted((ROOT / "bench").glob("[0-9][0-9]_*")):
        if (d / "bench.loft").is_file():
            out.append((d.name, "-", ROOT, ["--path", f"{ROOT}/", str(d / "bench.loft")]))
    args = library_packages()
    for spec in args[1::2]:
        pkg, _, name = spec.rpartition("=")
        commit = subprocess.run(["git", "-C", pkg, "rev-parse", "--short=9", "HEAD"],
                                capture_output=True, text=True).stdout.strip() or "?"
        out.append((name, commit + resolved(Path(pkg)), Path(pkg), ["bench/bench.loft"]))
    return out


def resolved(pkg):
    """`+name@version` for every dependency the lane's `loft.lock` resolved, sorted.  A library
    row is a function of its checkout AND of the dependencies its lane compiles with them, and
    the lock is no tracked file of the library's: each machine keeps what its first install
    resolved.  A lane whose dependencies differ from the baseline row's is therefore another
    program, reported as moved and not judged — never a drop (loft#2135: `crypto` 0.3.11
    against 0.3.14 moved four of pluginabi's rows).  Read line by line: this runs on a
    Python without `tomllib`."""
    lock = pkg / "loft.lock"
    if not lock.is_file():
        return ""
    deps, name = [], None
    for line in lock.read_text(encoding="utf-8").splitlines():
        key, _, value = line.partition("=")
        key, value = key.strip(), value.strip().strip('"')
        if line.startswith("[[package]]"):
            name = None
        elif key == "name":
            name = value
        elif key == "version" and name:
            deps.append(f"+{name}@{value}")
            name = None
    return "".join(sorted(deps))


def census(loft, prog, writer):
    """The program's admissions under `writer`, from the first of two emissions; the second
    must be the same bytes — the emitter is deterministic, and a hash-ordered walk that is not
    makes the same program a different source from run to run (a native cache miss for
    nothing)."""
    name, commit, cwd, argv = prog
    ext, flags = WRITERS[writer]
    with tempfile.TemporaryDirectory() as tmp:
        tsv = Path(tmp) / "census.tsv"
        by_fn = Path(tmp) / "census_fn.tsv"
        emitted = []
        for i in range(2):
            out = Path(tmp) / f"out{i}{ext}"
            proc = subprocess.run(
                [loft, *flags(out), *argv],
                cwd=cwd, capture_output=True, text=True, timeout=300,
                env={**rust_writer_env(), "LOFT_NO_NATIVE_LIBS": "1",
                     "LOFT_REWRITE_CENSUS": str(tsv) if i == 0 else "",
                     "LOFT_REWRITE_CENSUS_FN": str(by_fn) if i == 0 else ""},
            )
            if proc.returncode != 0 or not out.exists() or (i == 0 and not tsv.exists()):
                return name, commit, writer, None, proc.stderr.strip().splitlines()[-1:] or ["no output"]
            emitted.append(out.read_bytes().splitlines())
        if emitted[0] != emitted[1]:
            at = next((k for k, (a, b) in enumerate(zip(*emitted)) if a != b), min(map(len, emitted)))
            return name, commit, writer, None, [f"two emissions differ from line {at + 1} — the emitter is not deterministic"]
        counts = {}
        for line in tsv.read_text().splitlines():
            rule, _, n = line.partition("\t")
            counts[rule] = int(n)
        # `phase<TAB>function<TAB>rule<TAB>count` → admissions per (phase, rule).
        split = {}
        if by_fn.exists():
            for line in by_fn.read_text().splitlines():
                phase, _fn, rule, n = line.split("\t")
                split[(phase, rule)] = split.get((phase, rule), 0) + int(n)
        return name, commit, writer, (counts, split), None


def rust_writer_env():
    """The environment for a census run, minus `LOFT_NATIVE_BACKEND`: each writer is chosen by
    its flag (`WRITERS` — `--native-emit` for Rust, `--emit-c` for C), and `make ci` exports
    `LOFT_NATIVE_BACKEND=c` to every step, under which `--native-emit` writes C — the Rust row
    then counted the C writer's rewrites and read every rule the C does not fire as a drop."""
    return {k: v for k, v in os.environ.items() if k != "LOFT_NATIVE_BACKEND"}


def warm_dependencies(loft, progs):
    """Build every library row's dependency cdylibs once, one program at a time, before the
    parallel census.  A program whose dependency's cdylib exists calls into it; one that finds it
    absent, or still being built by another census worker, emits those functions inline — and
    counts their rewrites.  Run four at a time from a cold cache, WHICH program counted them was
    a race: two admissions moved between `hex_recover` and `hex_edge` from run to run, the total
    unchanged, and the gate failed on whichever lost them.  Warmed serially, every program sees
    every dependency present, so the count is the program's own."""
    with tempfile.TemporaryDirectory() as tmp:
        for name, commit, cwd, argv in progs:
            if commit == "-":
                continue
            subprocess.run(
                [loft, "--native-emit", str(Path(tmp) / "warm.rs"), "--lean", *argv],
                cwd=cwd, capture_output=True, timeout=600,
                env={**rust_writer_env(), "LOFT_REWRITE_CENSUS": ""},
            )


def read_baseline():
    """`(program, writer) → (commit, {rule: count})`; a four-column row is the Rust writer's."""
    rows = {}
    if BASELINE.is_file():
        for line in BASELINE.read_text().splitlines():
            if line.startswith("#") or not line.strip():
                continue
            parts = line.split("\t")
            name, commit, rule, n = parts[0], parts[1], parts[-2], parts[-1]
            writer = parts[2] if len(parts) == 5 else "rust"
            rows.setdefault((name, writer), (commit, {}))[1][rule] = int(n)
    return rows


def write_baseline(results):
    lines = [
        "# The rewrite census: how often each rewrite is admitted per bench program.",
        "# Written by `scripts/rewrite_census.py --bless`; a drop against it fails `make rewrite-census`.",
        "# program <TAB> commit (a library checkout's, then `+name@version` per dependency its lane "
        "resolved; `-` for an in-repo lane) <TAB> writer (`rust`, `c`) <TAB> rule <TAB> count",
    ]
    for name, commit, writer, counts in sorted(results, key=lambda r: (r[0], r[2])):
        for rule, n in sorted(counts.items()):
            lines.append(f"{name}\t{commit}\t{writer}\t{rule}\t{n}")
    BASELINE.write_text("\n".join(lines) + "\n")


def main() -> int:
    args = sys.argv[1:]
    # `--bless-unless-drop` is the check and the bless in ONE census: a drop fails exactly as
    # the plain run does and writes nothing; otherwise the counts are recorded, rises included.
    # A join's rederive asks it, so the programs are emitted once rather than three times.
    guarded = "--bless-unless-drop" in args
    bless = "--bless" in args
    only = None
    if "--only" in args:
        only = set(args[args.index("--only") + 1].split(","))
    writers = list(WRITERS)
    if "--writer" in args:
        writers = args[args.index("--writer") + 1].split(",")
        unknown = [w for w in writers if w not in WRITERS]
        if unknown:
            print(f"rewrite census: no writer {', '.join(unknown)} (the writers: {', '.join(WRITERS)})")
            return 2
    # Absolute: each program is compiled from its own directory, where a relative path
    # (`make after-fix` passes `target/probe/probe/loft`) names nothing.
    loft = os.path.abspath(os.environ.get("LOFT_BIN") or str(ROOT / "target" / "release" / "loft"))
    progs = [p for p in programs() if only is None or p[0] in only]
    warm_dependencies(loft, progs)
    with ThreadPoolExecutor(max_workers=4) as pool:
        done = list(pool.map(lambda pw: census(loft, *pw),
                             [(p, w) for p in progs for w in writers]))
    results, failed, splits = [], [], []
    for name, commit, writer, got, err in done:
        if got is None:
            failed.append(f"{name} ({writer}): {err[0]}")
        else:
            results.append((name, commit, writer, got[0]))
            splits.append((name, writer, got[1]))
    # The phase table is judged over the whole body only: a partial run sees part of it.
    whole = only is None and writers == list(WRITERS) and not failed
    phase_changes = []
    if whole:
        phases = rule_phases.aggregate(splits)
        phase_changes = rule_phases.changes(phases, rule_phases.read_table())
    base = read_baseline()
    drops, notes = [], []
    for name, commit, writer, counts in results:
        if (name, writer) not in base:
            notes.append(f"{name} ({writer}): no baseline row yet")
            continue
        bcommit, bcounts = base[(name, writer)]
        if bcommit != commit:
            notes.append(f"{name} ({writer}): the checkout moved ({bcommit} → {commit}), not judged")
            continue
        # The writer goes after the counts: `perf_check.py` reads the program off
        # "<rule> in <program>:".
        for rule in sorted(set(bcounts) | set(counts)):
            was, now = bcounts.get(rule, 0), counts.get(rule, 0)
            if now < was:
                drops.append(f"{rule} in {name}: {was} → {now} ({writer})")
            elif now > was:
                notes.append(f"{rule} in {name}: {was} → {now} ({writer})")
    for writer in writers:
        mine = [c for _, _, w, c in results if w == writer]
        print(f"rewrite census ({writer}): {len(mine)} programs, "
              f"{sum(sum(c.values()) for c in mine)} admissions")
    if notes and ("--verbose" in args or bless is False and len(notes) <= 10):
        for n in notes:
            print(f"  note: {n}")
    elif notes:
        print(f"  {len(notes)} rises / new rows (--verbose lists them)")
    for f in failed:
        print(f"  could not census {f}")
    if guarded and not drops and not failed and not phase_changes:
        bless = True
    if bless:
        ran = {(r[0], r[2]) for r in results}
        kept = [(n, base[(n, w)][0], w, base[(n, w)][1]) for n, w in base if (n, w) not in ran]
        write_baseline(results + kept)
        print(f"wrote {BASELINE.relative_to(ROOT)}")
        for c in phase_changes:
            print(f"  phase: {c}")
        if whole:
            rule_phases.write_table(phases)
            print(f"wrote {rule_phases.TABLE.relative_to(ROOT)}")
            # The register's `## Applied by` is the table rendered: kept in the same step.
            doc = rule_phases.DOC.read_text()
            rendered = rule_phases.doc_with(doc, rule_phases.render(rule_phases.read_table()))
            if rendered is not None and rendered != doc:
                rule_phases.DOC.write_text(rendered)
                print(f"wrote {rule_phases.DOC.relative_to(ROOT)} § Applied by")
        else:
            print(f"  {rule_phases.TABLE.relative_to(ROOT)} left as it is: a partial run")
        return 0
    for d in drops:
        print(f"::error::rewrite admitted less often — {d}")
    if drops:
        print("A rewrite fires at fewer sites than the baseline: a condition tightened.  If that was "
              "deliberate, `scripts/rewrite_census.py --bless` records it in this commit.")
    for c in phase_changes:
        print(f"::error::rewrite applied by other phases — {c}")
    if phase_changes:
        print("A rewrite is applied by a different set of phases than bench/portal/rule_phases.tsv "
              "says (`scripts/rule_phases.py`).  A deliberate move lands with "
              "`scripts/rewrite_census.py --bless` in its own commit.")
    if not whole:
        print("  the phase table is judged on a full run only (no --only, --writer)")
    return 1 if drops or failed or phase_changes else 0


if __name__ == "__main__":
    sys.exit(main())
