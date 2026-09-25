#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Fail when a rewrite stopped firing where it used to — a tightened condition, caught exactly.

    scripts/rewrite_census.py            # census the benches, fail on any drop vs the baseline
    scripts/rewrite_census.py --bless    # …and write what it counted as the new baseline
    scripts/rewrite_census.py --only 12_drawing,drawing
    scripts/rewrite_census.py --verbose  # also list every rise and new row

Why
---
Every rewrite in doc/claude/formal/rewrites.md is admitted under conditions, and a bug fix that
adds a decline can switch it off on real code.  The speed that costs is often inside the noise
of any timing (the test speed gate only sees 3x), yet the COUNT of admissions over a fixed body
of programs is a pure function of the compiler: a drop names the rule and the program that lost
it, and nothing but a compiler change can move it.

How
---
Each program is compiled to native Rust only (`--native-emit --lean`, no rustc, ~0.2 s) with
`LOFT_REWRITE_CENSUS` set; the compiler counts each rewrite where it is ADMITTED
(`src/rewrite_census.rs`).  The body is the benches, the code whose speed is the goal: every
`bench/NN_*/bench.loft` lane in this repo, and the portal libraries' own benches wherever a
checkout exists (`bench/portal/libs.tsv`, `checkout_libs.sh`).  A library row carries the
checkout's commit; when the checkout has moved since the baseline, its rows are not judged —
the library changed, not the compiler.

The baseline is `bench/portal/rewrite_census.tsv`.  A DROP fails; a rise, a new program or a
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
        out.append((name, commit, Path(pkg), ["bench/bench.loft"]))
    return out


def census(loft, prog):
    name, commit, cwd, argv = prog
    with tempfile.TemporaryDirectory() as tmp:
        tsv = Path(tmp) / "census.tsv"
        proc = subprocess.run(
            [loft, "--native-emit", str(Path(tmp) / "out.rs"), "--lean", *argv],
            cwd=cwd, capture_output=True, text=True, timeout=300,
            env={**os.environ, "LOFT_REWRITE_CENSUS": str(tsv)},
        )
        if proc.returncode != 0 or not tsv.exists():
            return name, commit, None, proc.stderr.strip().splitlines()[-1:] or ["no output"]
        counts = {}
        for line in tsv.read_text().splitlines():
            rule, _, n = line.partition("\t")
            counts[rule] = int(n)
        return name, commit, counts, None


def read_baseline():
    rows = {}
    if BASELINE.is_file():
        for line in BASELINE.read_text().splitlines():
            if line.startswith("#") or not line.strip():
                continue
            name, commit, rule, n = line.split("\t")
            rows.setdefault(name, (commit, {}))[1][rule] = int(n)
    return rows


def write_baseline(results):
    lines = [
        "# The rewrite census: how often each rewrite is admitted per bench program.",
        "# Written by `scripts/rewrite_census.py --bless`; a drop against it fails `make rewrite-census`.",
        "# program <TAB> commit (a library checkout's; `-` for an in-repo lane) <TAB> rule <TAB> count",
    ]
    for name, commit, counts in sorted(results, key=lambda r: r[0]):
        for rule, n in sorted(counts.items()):
            lines.append(f"{name}\t{commit}\t{rule}\t{n}")
    BASELINE.write_text("\n".join(lines) + "\n")


def main() -> int:
    args = sys.argv[1:]
    bless = "--bless" in args
    only = None
    if "--only" in args:
        only = set(args[args.index("--only") + 1].split(","))
    loft = str(ROOT / "target" / "release" / "loft")
    progs = [p for p in programs() if only is None or p[0] in only]
    with ThreadPoolExecutor(max_workers=4) as pool:
        done = list(pool.map(lambda p: census(loft, p), progs))
    results, failed = [], []
    for name, commit, counts, err in done:
        if counts is None:
            failed.append(f"{name}: {err[0]}")
        else:
            results.append((name, commit, counts))
    base = read_baseline()
    drops, notes = [], []
    for name, commit, counts in results:
        if name not in base:
            notes.append(f"{name}: no baseline row yet")
            continue
        bcommit, bcounts = base[name]
        if bcommit != commit:
            notes.append(f"{name}: the checkout moved ({bcommit} → {commit}), not judged")
            continue
        for rule in sorted(set(bcounts) | set(counts)):
            was, now = bcounts.get(rule, 0), counts.get(rule, 0)
            if now < was:
                drops.append(f"{rule} in {name}: {was} → {now}")
            elif now > was:
                notes.append(f"{rule} in {name}: {was} → {now}")
    print(f"rewrite census: {len(results)} programs, "
          f"{sum(sum(c.values()) for _, _, c in results)} admissions")
    if notes and ("--verbose" in args or bless is False and len(notes) <= 10):
        for n in notes:
            print(f"  note: {n}")
    elif notes:
        print(f"  {len(notes)} rises / new rows (--verbose lists them)")
    for f in failed:
        print(f"  could not census {f}")
    if bless:
        kept = [(n, *base[n]) for n in base if n not in {r[0] for r in results}]
        write_baseline(results + kept)
        print(f"wrote {BASELINE.relative_to(ROOT)}")
        return 0
    for d in drops:
        print(f"::error::rewrite admitted less often — {d}")
    if drops:
        print("A rewrite fires at fewer sites than the baseline: a condition tightened.  If that was "
              "deliberate, `scripts/rewrite_census.py --bless` records it in this commit.")
    return 1 if drops or failed else 0


if __name__ == "__main__":
    sys.exit(main())
