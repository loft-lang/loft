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
"""

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
    total = 0
    over = []
    for case in cases:
        total += 1
        worst = max(attempts(case))
        if worst > budget:
            over.append((worst, f"{case.get('classname', '?')} {case.get('name', '?')}"))
    if total == 0:
        sys.exit(f"test_duration_gate: {junit} lists no test cases — nothing was measured")
    if not over:
        print(f"test duration gate: {total} tests, none over {budget:.0f} s (half the {limit:.0f} s limit)")
        return 0
    print(f"test duration gate: {len(over)} of {total} tests ran longer than {budget:.0f} s, half the "
          f"{limit:.0f} s per-test limit.  Make each one cheaper — split it, parallelise it, cache "
          f"its work — and never raise the limit:")
    for t, name in sorted(over, reverse=True):
        print(f"  {t:8.1f} s  {name}")
    return 1


if __name__ == "__main__":
    sys.exit(main())
