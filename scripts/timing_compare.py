#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Compare two timing ledgers phase by phase and test by test — where did the time go?

    scripts/timing_compare.py <before> <after> [--tests N] [--test SUBSTRING]

Each side is a ledger: a directory of `timing-<pid>.tsv` files (`LOFT_TIMING_LEDGER=<dir>`
on any loft run or test run), or `run:<id>` for a CI run, whose `timing-ledger-*` artifacts
are downloaded with `gh` (`run:<id>:<artifact substring>` picks one leg, e.g. `run:123:macos`).

A ledger row is written by every loft process (`src/platform.rs`):

    phase   <name>  <secs>                     <test>   parse_default, parse_user, scopes, lints,
                                                        byte_code, native_emit, run
    binary  <program> hit|miss <secs>          <test>   the program's own native compile
    exec    <tool> <subject> <reason> <secs>   <test>   an external build step (cargo, rustc…)
    cdylib|fixture <name> hit|miss <secs>      <test>   a library's native build

The last column names the test that started that loft (`LOFT_TIMING_TEST`, set by
`platform::process::harness_command`), `-` outside a test.

The report: totals per category (a phase, `binary:miss`, `exec:rustc`, …) on both sides
and their difference, then the tests whose time grew most, each with the categories that
moved.  A slower test reads as WHICH phase grew — the front end, the emitter, rustc, a
cache that stopped hitting, the program's own run — instead of as one duration to bisect.
"""

import glob
import os
import subprocess
import sys
import tempfile
from collections import defaultdict


def rows_of(directory):
    """Every ledger row under `directory` (recursively: artifacts unpack into subfolders)."""
    out = []
    for path in glob.glob(os.path.join(directory, "**", "timing-*.tsv"), recursive=True):
        with open(path, encoding="utf-8", errors="replace") as f:
            for line in f:
                cols = line.rstrip("\n").split("\t")
                if cols and cols[0]:
                    out.append(cols)
    return out


def category(cols):
    """(category, seconds, test) for one row, or None for a row that carries no time."""
    kind = cols[0]
    try:
        if kind == "phase" and len(cols) >= 3:
            return f"phase:{cols[1]}", float(cols[2]), cols[3] if len(cols) > 3 else "-"
        if kind == "exec" and len(cols) >= 5:
            return f"exec:{cols[1]}", float(cols[4]), cols[5] if len(cols) > 5 else "-"
        if len(cols) >= 4 and cols[3]:
            return f"{kind}:{cols[2]}", float(cols[3]), cols[4] if len(cols) > 4 else "-"
    except ValueError:
        return None
    return None


def load(source):
    """{test: {category: [seconds, count]}} for one side."""
    if source.startswith("run:"):
        directory = download(source)
    else:
        directory = source
    if not os.path.isdir(directory):
        sys.exit(f"timing_compare: no ledger directory at {directory}")
    table = defaultdict(lambda: defaultdict(lambda: [0.0, 0]))
    n = 0
    for cols in rows_of(directory):
        c = category(cols)
        if c is None:
            continue
        name, secs, test = c
        cell = table[test][name]
        cell[0] += secs
        cell[1] += 1
        n += 1
    if n == 0:
        sys.exit(f"timing_compare: {source} holds no timed rows")
    return table


def download(source):
    """Download a CI run's timing-ledger artifacts; `run:<id>[:<leg substring>]`."""
    parts = source.split(":")
    run_id = parts[1]
    leg = parts[2] if len(parts) > 2 else ""
    dest = tempfile.mkdtemp(prefix=f"ledger-{run_id}-")
    names = subprocess.run(
        ["gh", "api", f"repos/{{owner}}/{{repo}}/actions/runs/{run_id}/artifacts",
         "--paginate", "--jq", ".artifacts[].name"],
        capture_output=True, text=True, check=False,
    ).stdout.split()
    picked = [n for n in names if n.startswith("timing-ledger") and leg in n]
    if not picked:
        sys.exit(f"timing_compare: run {run_id} has no timing-ledger artifact matching '{leg}'")
    for name in picked:
        subprocess.run(["gh", "run", "download", run_id, "-n", name, "-D",
                        os.path.join(dest, name)], check=True)
    return dest


def totals(table):
    out = defaultdict(lambda: [0.0, 0])
    for cats in table.values():
        for name, (secs, count) in cats.items():
            out[name][0] += secs
            out[name][1] += count
    return out


def test_total(cats):
    return sum(secs for secs, _ in cats.values())


def main(argv):
    if "--help" in argv or "-h" in argv:
        print(__doc__)
        return 0
    args = [a for a in argv if not a.startswith("--")]
    opts = dict(a[2:].split("=", 1) for a in argv if a.startswith("--") and "=" in a)
    if len(args) != 2:
        sys.exit(__doc__)
    show = int(opts.get("tests", "15"))
    only = opts.get("test", "")
    before, after = load(args[0]), load(args[1])
    if only:
        before = {t: c for t, c in before.items() if only in t}
        after = {t: c for t, c in after.items() if only in t}

    tb, ta = totals(before), totals(after)
    print(f"{'category':<24}{'before s':>11}{'after s':>11}{'delta s':>11}{'ratio':>8}"
          f"{'n before':>10}{'n after':>9}")
    for name in sorted(set(tb) | set(ta), key=lambda k: -(ta[k][0] - tb[k][0])):
        b, a = tb[name], ta[name]
        ratio = f"{a[0] / b[0]:.2f}" if b[0] > 0 else "-"
        print(f"{name:<24}{b[0]:>11.2f}{a[0]:>11.2f}{a[0] - b[0]:>+11.2f}{ratio:>8}"
              f"{b[1]:>10}{a[1]:>9}")

    print(f"\ntests whose time grew most (of {len(set(before) | set(after))}):")
    tests = sorted(set(before) | set(after),
                   key=lambda t: -(test_total(after.get(t, {})) - test_total(before.get(t, {}))))
    for t in tests[:show]:
        cb, ca = before.get(t, {}), after.get(t, {})
        db, da = test_total(cb), test_total(ca)
        print(f"  {da - db:+8.2f}s  {db:8.2f} -> {da:8.2f}  {t}")
        moved = sorted(set(cb) | set(ca),
                       key=lambda k: -abs(ca.get(k, [0, 0])[0] - cb.get(k, [0, 0])[0]))
        for k in moved[:4]:
            kb, ka = cb.get(k, [0.0, 0]), ca.get(k, [0.0, 0])
            if abs(ka[0] - kb[0]) < 0.005:
                continue
            print(f"             {k:<22}{kb[0]:8.2f} -> {ka[0]:8.2f}  ({kb[1]} -> {ka[1]} rows)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
