#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Fail when a test got HARD slower than it is on main — and the drop survives a rerun alone.

    scripts/test_speed_gate.py <junit.xml>                  # judge a run against main
    scripts/test_speed_gate.py <junit.xml> --no-confirm     # flag only, rerun nothing
    scripts/test_speed_gate.py <junit.xml> --baseline FILE  # a saved CI log or junit

What it guards
--------------
A regression by a FACTOR, somewhere in the suite: a test that took 5 s on main and takes 100 s
here.  Measured 2026-09-26 over eight main runs: `html_wasm::pln24_a_reachable_c_binding_…`
went from 4.7 s to ~105 s between 09-17 and 09-25 and nothing said so, because the suite's
other duration check (`test_duration_gate.py`) only asks for half the hang limit.

Why a gate here when `test_speed.py` is a report
------------------------------------------------
`test_speed.py` reads DRIFT — 25 % bands — and a band that narrow is noise under 24-way
parallelism, so it prints and never fails.  This asks a coarser question, and answers it with a
second measurement instead of a wider band.  Measured on the same commit twice: single tests
move up to 8x under the suite's contention (`deliver_wasm` 9 s → 73 s), so a threshold alone
fires on every run.  What contention cannot do is repeat when the test runs ALONE, and a real
regression does nothing else — so a flag becomes a failure only when the test, rerun on its own
(the fastest of up to two runs), is still over the bar.

The numbers
-----------
Baseline: the per-test durations of the latest green push-to-main run's `Test (ubuntu-latest)`
job — unsharded, so it names every test — read from its log (the `ci` profile prints each pass
with its time), fetched with `gh` and cached under `target/speed-gate/`.  Only tests present in
BOTH runs are compared: a new test has no history, and a sum over tests would read one as a
slowdown of its whole binary.

Normalised by the run's own speed: the median ratio over the common tests of at least half a
second, so a slower runner or a faster laptop is one factor and not four thousand flags.  A
drop that moved the MEDIAN is therefore invisible here by construction; the median is printed.

Flagged: at least RATIO times the baseline AND at least DELTA seconds more, both after
normalising.  Confirmed: the same two bars, met by the rerun alone.  At most MAX_CONFIRM flags are
rerun, largest first, and none once RERUN_BUDGET seconds have gone to reruns; the rest are listed
as not rerun and do not fail.

Exit 0 when nothing is confirmed (or no baseline could be read — said loudly), 1 otherwise.
"""

import json
import re
import statistics
import subprocess
import sys
import time
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CACHE = ROOT / "target" / "speed-gate"
REPO = "loft-lang/loft"
BASELINE_JOB = "Test (ubuntu-latest)"

RATIO = 3.0  # times the baseline
DELTA = 5.0  # seconds more than the baseline
NORM_FLOOR = 0.5  # tests below this many seconds do not vote on the run's speed
MAX_CONFIRM = 16
RERUN_BUDGET = 300.0  # seconds of reruns at most, inside the PR's 20-minute budget
REPEATS = 2

ANSI = re.compile(r"\x1b\[[0-9;]*m")
PASS = re.compile(
    r"\b(PASS|FLAKY)\s*\[\s*([0-9.]+)s\]\s*(?:\(\s*\d+/\d+\))?\s+(\S+)\s+(\S+)\s*$"
)


def parse_log(text: str) -> dict[str, float]:
    """`binary test` → seconds, from nextest's `PASS [ 1.234s] (n/N) binary test` lines."""
    out = {}
    for line in text.splitlines():
        m = PASS.search(ANSI.sub("", line))
        if m:
            out[f"{m.group(3)} {m.group(4)}"] = float(m.group(2))
    return out


def parse_junit(path: Path) -> dict[str, float]:
    """`binary test` → seconds, for the tests that passed (a failure is another gate's)."""
    out = {}
    for case in ET.parse(path).getroot().iter("testcase"):
        if any(c.tag in ("failure", "error", "skipped") for c in case):
            continue
        out[f"{case.get('classname')} {case.get('name')}"] = float(case.get("time") or 0)
    return out


def read_durations(path: Path) -> dict[str, float]:
    head = path.read_text(errors="replace")[:200]
    return parse_junit(path) if head.lstrip().startswith("<?xml") else parse_log(path.read_text(errors="replace"))


def gh(*args: str) -> str:
    return subprocess.run(
        ["gh", *args], check=True, capture_output=True, text=True, timeout=300
    ).stdout


def main_baseline() -> tuple[dict[str, float], str]:
    """The latest green push-to-main run's unsharded ubuntu job, fetched once and cached."""
    runs = json.loads(
        gh("run", "list", "-R", REPO, "--workflow", "ci.yml", "--branch", "main",
           "--event", "push", "--status", "success", "--limit", "10",
           "--json", "databaseId,headSha")
    )
    for run in runs:
        rid = run["databaseId"]
        cached = CACHE / f"baseline-{rid}.log"
        if not cached.is_file():
            jobs = gh("api", f"repos/{REPO}/actions/runs/{rid}/jobs", "--paginate",
                      "-q", f'.jobs[] | select(.name=="{BASELINE_JOB}" and .conclusion=="success") | .id')
            job = jobs.split()[0] if jobs.split() else None
            if not job:
                continue
            CACHE.mkdir(parents=True, exist_ok=True)
            cached.write_text(gh("api", f"repos/{REPO}/actions/jobs/{job}/logs"))
        durations = parse_log(cached.read_text(errors="replace"))
        if durations:
            return durations, f"main {run['headSha'][:9]} (run {rid})"
    raise RuntimeError("no green push-to-main run with a readable test log")


def normalise(base: dict[str, float], cur: dict[str, float]) -> tuple[list[str], float]:
    common = [k for k in cur if k in base]
    ratios = [cur[k] / base[k] for k in common if base[k] >= NORM_FLOOR and cur[k] > 0]
    return common, statistics.median(ratios) if ratios else 1.0


def over_bar(base: float, seconds: float) -> bool:
    return seconds >= RATIO * base and seconds - base >= DELTA


def rerun_alone(key: str, base: float, factor: float) -> float | None:
    """The fastest of up to REPEATS runs of one test on its own; stops once it is under the bar."""
    binary, test = key.split(" ", 1)
    best = None
    for _ in range(REPEATS):
        proc = subprocess.run(
            ["cargo", "nextest", "run", "--profile", "default", "--retries", "0",
             "--test-threads", "1", "--status-level", "pass", "--no-fail-fast",
             "-E", f"binary_id(={binary}) & test(={test})"],
            cwd=ROOT, capture_output=True, text=True,
        )
        seen = parse_log(proc.stdout + proc.stderr).get(key)
        if seen is None:
            return best
        best = seen if best is None else min(best, seen)
        if not over_bar(base, best / factor):
            return best
    return best


def main() -> int:
    args = sys.argv[1:]
    confirm = "--no-confirm" not in args
    baseline_file = None
    if "--baseline" in args:
        baseline_file = Path(args[args.index("--baseline") + 1])
    paths = [a for a in args if not a.startswith("--") and Path(a) != baseline_file]
    if not paths:
        sys.exit(__doc__)
    cur = read_durations(Path(paths[0]))
    try:
        if baseline_file:
            base, origin = read_durations(baseline_file), str(baseline_file)
        else:
            base, origin = main_baseline()
    except (RuntimeError, subprocess.SubprocessError, OSError) as e:
        print(f"::warning::test speed gate: NO BASELINE, nothing judged — {e}")
        return 0
    common, factor = normalise(base, cur)
    flags = []
    for k in common:
        seen = cur[k] / factor
        if over_bar(base[k], seen):
            flags.append((seen - base[k], k, seen))
    flags.sort(reverse=True)
    print(f"test speed gate: {len(common)} tests against {origin}; this run is {factor:.2f}x its speed "
          f"(median), bar {RATIO:g}x and +{DELTA:g}s after that")
    if not flags:
        return 0
    confirmed = []
    started = time.monotonic()
    for i, (_, k, seen) in enumerate(flags):
        line = f"  {k}: {base[k]:.1f}s on main, {seen:.1f}s here"
        if not confirm or i >= MAX_CONFIRM or time.monotonic() - started > RERUN_BUDGET:
            print(line + ("  (not rerun)" if confirm else ""))
            continue
        alone = rerun_alone(k, base[k], factor)
        if alone is None:
            print(line + ", the rerun did not report it")
            continue
        alone_n = alone / factor
        verdict = "CONFIRMED" if over_bar(base[k], alone_n) else "contention"
        print(line + f", {alone_n:.1f}s alone — {verdict}")
        if verdict == "CONFIRMED":
            confirmed.append((k, base[k], alone_n))
    for k, b, a in confirmed:
        print(f"::error::{k} is {a / b:.1f}x slower than on main ({b:.1f}s → {a:.1f}s, rerun alone)")
    return 1 if confirmed else 0


if __name__ == "__main__":
    sys.exit(main())
