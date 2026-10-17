#!/usr/bin/env python3
"""Fail when any test took more than HALF of its per-test limit.

The `ci` profile's `slow-timeout` (`.config/nextest.toml`) is a HANG guard: it kills a test that
is stuck, and it must never be what a healthy test runs into.  A test whose normal run reaches
half of it is one runner-noise spike from a false kill, and it grows with whatever it covers —
so it is a COST defect, fixed by splitting, parallelising or caching, never by raising the limit
(the owner, 2026-09-24).  This is the gate that keeps that true: a report of the same numbers was
proposed and rejected, because a report gets ignored.

Usage:  test_duration_gate.py <junit.xml> [--profile ci]

The limit is read from the profile's `slow-timeout.period`, the one home for it.  A test that
nextest retried is judged on its slowest attempt: a first try that ran long and a retry that
passed quickly is exactly the shape this gate exists to catch.

A test is judged on its OWN work.  The first test to need a native cdylib on a cold runner
builds it while holding the global build lock — the graphics cdylib is ~95 s — and the
timing ledger (`LOFT_TIMING_LEDGER=<dir>`, a `lockheld … miss <secs> <test>` row) says which
test paid and how long; that wait is a shared build every later test gets for free, not the
test's cost, so it is taken off before the budget is applied and named beside the number.
Measured: `crystal_editor_gl_matches_gold` read 306 s and 317 s on two cold runners, 97 s of
each holding the lock for the graphics build, and under 300 s wherever the cdylib was warm.
"""

import os
import re
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:  # python < 3.11
    import tomli as tomllib

ROOT = Path(__file__).resolve().parent.parent


def profile_limit_seconds(profile: str) -> float:
    cfg = tomllib.loads((ROOT / ".config" / "nextest.toml").read_text())
    st = cfg["profile"][profile]["slow-timeout"]
    period = st["period"] if isinstance(st, dict) else st
    m = re.fullmatch(r"\s*([0-9.]+)\s*s\s*", period)
    if not m:
        sys.exit(f"test_duration_gate: cannot read slow-timeout period {period!r}")
    return float(m.group(1))


def attempts(case: ET.Element):
    """Every attempt's duration for one testcase: the final one plus each rerun/flaky one."""
    yield float(case.get("time", "0") or 0)
    for child in case:
        if child.tag in ("flakyFailure", "flakyError", "rerunFailure", "rerunError"):
            t = child.get("time")
            if t:
                yield float(t)


def build_waits(ledger_dir: str | None) -> dict[str, float]:
    """Seconds each test spent holding the build lock for a cache MISS: the ledger's
    `lockheld <name> miss <secs> <test>` rows, summed per test (the last column)."""
    waits: dict[str, float] = {}
    if not ledger_dir or not Path(ledger_dir).is_dir():
        return waits
    for f in Path(ledger_dir).glob("timing-*.tsv"):
        for line in f.read_text(encoding="utf-8", errors="replace").splitlines():
            cols = line.split("\t")
            if len(cols) >= 5 and cols[0] == "lockheld" and cols[2] == "miss" and cols[3]:
                try:
                    waits[cols[4]] = waits.get(cols[4], 0.0) + float(cols[3])
                except ValueError:
                    pass
    return waits


def main() -> int:
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    profile = "ci"
    if "--profile" in sys.argv:
        profile = sys.argv[sys.argv.index("--profile") + 1]
        args = [a for a in args if a != profile]
    if not args:
        sys.exit(__doc__)
    junit = Path(args[0])
    if not junit.is_file():
        sys.exit(f"test_duration_gate: no junit report at {junit} — the test step did not run")
    limit = profile_limit_seconds(profile)
    budget = limit / 2
    cases = ET.parse(junit).getroot().iter("testcase")
    waits = build_waits(os.environ.get("LOFT_TIMING_LEDGER"))
    total = 0
    over = []
    waited = []
    for case in cases:
        total += 1
        worst = max(attempts(case))
        name = case.get("name", "?")
        wait = waits.get(name, 0.0)
        label = f"{case.get('classname', '?')} {name}"
        if worst > budget and worst - wait <= budget:
            waited.append((worst, wait, label))
        elif worst > budget:
            over.append((worst, wait, label))
    if total == 0:
        sys.exit(f"test_duration_gate: {junit} lists no test cases — nothing was measured")
    for t, w, name in waited:
        print(f"test duration gate: {name} ran {t:.1f} s, {w:.1f} s of it holding the build lock for a "
              f"cold cdylib — its own {t - w:.1f} s is under the {budget:.0f} s budget")
    if not over:
        print(f"test duration gate: {total} tests, none over {budget:.0f} s (half the {limit:.0f} s limit)")
        return 0
    print(f"test duration gate: {len(over)} of {total} tests ran longer than {budget:.0f} s, half the "
          f"{limit:.0f} s per-test limit.  Make each one cheaper — split it, parallelise it, cache "
          f"its work — and never raise the limit:")
    for t, w, name in sorted(over, reverse=True):
        wait = f"  ({w:.1f} s of it a shared cdylib build; {t - w:.1f} s its own)" if w else ""
        print(f"  {t:8.1f} s  {name}{wait}")
    return 1


if __name__ == "__main__":
    sys.exit(main())
