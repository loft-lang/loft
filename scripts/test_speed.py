#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Report how the slow tests' speed has DRIFTED, and never fail because of it.

    scripts/test_speed.py run            # run the suite and report
    scripts/test_speed.py report         # report from the last run
    scripts/test_speed.py run --bless    # …and write the new numbers into the tests
    scripts/test_speed.py calibrate      # just print this machine's calibration

Why this is a report and not a gate
-----------------------------------
A time assertion fails for reasons the test is not about: a busy machine, a
different CPU, a change somewhere else in the suite.  Making a build red on
those teaches everyone to widen the band until it means nothing, and the one
real regression arrives inside a band nobody trusts.  So this prints, and exits
0 whatever it finds.  Correctness is what fails a build; speed is what you read.
(A HARD drop — 3x, confirmed by rerunning the test alone — is the one speed question that
does fail a build: `test_speed_gate.py`, which answers noise with a second measurement
instead of a wider band.)

Timeouts still have a job — they bound things we do not control (a socket, a
process spawn, `rustc`).  They are a liveness bound, not a speed measurement,
and a test that takes its whole timeout tells you nothing about how fast it is.

What the number means
---------------------
Seconds **on a reference machine**, not on yours.  Each run measures this
machine with a fixed integer loop that has nothing to do with loft, and scales
every duration by it:

    units = seconds * (CAL_REFERENCE_MS / measured_calibration_ms)

That choice is the point of the design, and the two obvious alternatives are
both wrong here:

* **Raw seconds** move with the machine and with the load, so the same test
  reads differently on a laptop, a CI runner, and a box that happens to be
  compiling something.
* **A share of the suite's total** moves when ANY OTHER test changes.  Make the
  hash faster and every unrelated test's share rises — the report would then
  accuse a dozen innocent tests of getting slower every time something got
  faster, which is precisely the failure mode this exists to avoid.

A calibration deliberately outside loft has neither property: it tracks the
machine and nothing else, so a loft change shows up in full and a neighbouring
test's change does not show up at all.

Why only the slow tests, and why serially
-----------------------------------------
nextest runs 24 tests at once, and under that a test's wall clock is mostly a
statement about what shared the box with it.  Measured: with everything warm and
the numbers freshly blessed, a re-run still moved 48 of 134 annotated tests past
±25%, in both directions, while nothing had changed.  No normalisation fixes
that — a single-threaded calibration cannot model 24-way contention, and one
taken from the suite's own total re-couples every test to every other, which is
the failure this exists to avoid.

So the measuring pass runs `--test-threads=1`, over the ANNOTATED tests only.
That is affordable precisely because the report is about slow tests: a few dozen
of them, not four thousand.  `discover` is the separate, parallel pass that says
which tests deserve an annotation; it is allowed to be noisy, because it only
has to answer "is this over a second", never "did it change".

Why the suite runs more than once
---------------------------------
A single run measures cache warmth, not speed, and by a wide margin.  Measured
here on the first cut of this tool: blessed from one run, then re-run
immediately, **113 of 139 annotated tests moved beyond ±25% and every one of
them was FASTER** — `multiplayer_v2::server_detects_and_retries_a_stolen_port`
by 39x, from 24.9 units to 0.63.  Nothing had changed but the state of the
build cache, the `native-auto` cdylibs and the page cache.

So each test's figure is the **minimum over `--repeat` runs** (default 2).  Cold
caches and a busy box only ever make a run slower, so the smallest observation
is the one least contaminated by anything other than the test — the standard
estimator for exactly this, and the reason a single-run report is not offered as
the default.

Its limits, so nobody over-reads it: it calibrates CPU, so a test dominated by
`rustc`, disk or the network normalises poorly; nextest runs tests in parallel,
so a duration still carries some of whatever shared the box with it; and a
Python upgrade moves every number at once.  All three are why the tolerance is
generous and why one report is a hint, not a verdict — which is also why the
expected number lives in the test file and is committed.  `git log -p` on that
line IS the history: a steady drift is a series of small diffs, and a real
regression is one large one, both reviewable at the moment they happen.

Where the number lives
----------------------
In the test, above its `#[test]`:

    // @speed 12.4
    #[test]
    fn a_slow_test() { … }

Only tests at or above `--floor` (default 1.0 unit ≈ one reference second) carry
one — a 5 ms test's timing is noise, and annotating it would be noise too.
"""

from __future__ import annotations

import argparse
import os
import re
import statistics
import subprocess
import sys
import time
import xml.etree.ElementTree as ET
from dataclasses import dataclass
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

# One reference second.  Measured on the box this landed from (2026-08-10,
# Ubuntu 24.04, x86_64) so a unit reads as "a second on a machine like that
# one".  Re-pinning it re-scales every annotation at once, which is a whole-file
# diff and therefore obvious — unlike a silent drift, which is what it prevents.
CAL_REFERENCE_MS = 107.0

# The nextest profile that writes the JUnit report we read.  Durations come from
# there rather than from the human log because the log's format is a UI, and
# `find_problems.sh` legitimately runs with `--status-level fail`, which prints
# no timing for a passing test at all.
# The two calibrations a `run` takes, kept so the report can say whether the
# machine held still — a single scale cannot describe a run that changed halfway.
CAL_ENDS: tuple[float, float] | None = None

PROFILE = "speed"
JUNIT = REPO / "target" / "nextest" / PROFILE / "junit.xml"

ANNOT = re.compile(r"^(?P<indent>\s*)//\s*@speed\s+(?P<units>[0-9]+(?:\.[0-9]+)?)\s*$")
TEST_ATTR = re.compile(r"^\s*#\[(?:\w+::)*test\]\s*$")
FN_DECL = re.compile(r"^\s*(?:pub\s+)?(?:async\s+)?fn\s+(?P<name>\w+)\s*\(")


def calibrate(rounds: int = 5) -> float:
    """Milliseconds for a fixed integer loop — this machine's speed, in one number.

    The first sample is discarded: it pays for whatever the interpreter has not
    warmed yet, and it is reliably the outlier.  The rest are taken as a median
    rather than a mean so one scheduling hiccup cannot move the result.
    """
    samples = []
    for _ in range(rounds):
        start = time.perf_counter()
        x = 0
        for _ in range(2_000_000):
            x = (x * 1103515245 + 12345) & 0xFFFF_FFFF
        samples.append((time.perf_counter() - start) * 1000.0)
    return statistics.median(samples[1:] or samples)


@dataclass
class Measured:
    binary: str
    name: str
    seconds: float

    @property
    def key(self) -> str:
        return f"{self.binary}::{self.name}"


def read_junit(path: Path) -> list[Measured]:
    """Every test case in the report, with the seconds nextest recorded for it."""
    if not path.exists():
        sys.exit(
            f"no timing report at {path} — run `scripts/test_speed.py run` first "
            f"(it is written by the `{PROFILE}` nextest profile)"
        )  # noqa: E501
    out: list[Measured] = []
    for case in ET.parse(path).getroot().iter("testcase"):
        # nextest writes `classname="loft::<binary>"`.
        binary = (case.get("classname") or "").split("::")[-1]
        name = case.get("name") or ""
        try:
            seconds = float(case.get("time") or 0.0)
        except ValueError:
            seconds = 0.0
        if name:
            out.append(Measured(binary, name, seconds))
    return out


@dataclass
class Annotation:
    file: Path
    fn_line: int  # 0-based index of the `#[test]` attribute
    annot_line: int | None  # 0-based index of an existing `// @speed` line
    units: float | None


def scan_annotations() -> dict[str, list[Annotation]]:
    """Every `#[test] fn` under `tests/`, with the `@speed` it carries, by fn name.

    Keyed by function name alone, and deliberately a LIST: two binaries may name
    a test the same thing, and silently picking one would attach a number to the
    wrong test.  The caller resolves with the binary it came from.
    """
    found: dict[str, list[Annotation]] = {}
    for path in sorted((REPO / "tests").rglob("*.rs")):
        lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
        for i, line in enumerate(lines):
            if not TEST_ATTR.match(line):
                continue
            # The function this attribute belongs to: the next `fn` declaration,
            # past any further attributes.
            name = None
            for j in range(i + 1, min(i + 8, len(lines))):
                m = FN_DECL.match(lines[j])
                if m:
                    name = m.group("name")
                    break
            if not name:
                continue
            annot_line, units = None, None
            if i > 0:
                m = ANNOT.match(lines[i - 1])
                if m:
                    annot_line, units = i - 1, float(m.group("units"))
            found.setdefault(name, []).append(Annotation(path, i, annot_line, units))
    return found


def resolve(annots: dict[str, list[Annotation]], m: Measured) -> Annotation | None:
    """The annotation site for `m`, preferring the file named after its binary."""
    candidates = annots.get(m.name)
    if not candidates:
        return None
    for c in candidates:
        if c.file.stem == m.binary:
            return c
    return candidates[0] if len(candidates) == 1 else None


def report(
    cal_ms: float,
    floor: float,
    tolerance: float,
    bless: bool,
    measured: list[Measured],
    runs: int,
    drift_unreliable: tuple[float, float] | None = None,
) -> None:
    scale = CAL_REFERENCE_MS / cal_ms
    annots = scan_annotations()

    drift: list[tuple[float, str, float, float]] = []
    new: list[tuple[float, str]] = []
    gone: list[tuple[str, float]] = []
    edits: dict[Path, list[tuple[int, int | None, float]]] = {}

    seen: set[str] = set()
    for m in measured:
        units = m.seconds * scale
        site = resolve(annots, m)
        seen.add(m.name)
        if site is None:
            # No annotatable site (a doc test, a name in two binaries).  Still
            # worth naming when it is slow, so it can be annotated by hand.
            if units >= floor:
                new.append((units, m.key))
            continue
        if site.units is None:
            if units >= floor:
                new.append((units, m.key))
                edits.setdefault(site.file, []).append(
                    (site.fn_line, site.annot_line, units)
                )
            continue
        # Hysteresis on the way out: a test has to fall well below the floor
        # before its annotation is dropped, or one lucky run deletes it and the
        # next run adds it back, and the diff churns forever.
        if units < floor / 2:
            gone.append((m.key, site.units))
            if bless and site.annot_line is not None:
                edits.setdefault(site.file, []).append(
                    (site.fn_line, site.annot_line, -1.0)
                )
            continue
        ratio = units / site.units if site.units else 0.0
        if abs(ratio - 1.0) > tolerance:
            drift.append((ratio, m.key, site.units, units))
        if bless:
            edits.setdefault(site.file, []).append(
                (site.fn_line, site.annot_line, units)
            )

    # Annotated tests that did not run at all: renamed, removed, or filtered out.
    for name, sites in annots.items():
        if name in seen:
            continue
        for s in sites:
            if s.units is not None:
                gone.append((f"{s.file.stem}::{name} (did not run)", s.units))

    print("=== Test speed — REPORT ONLY, nothing here fails a build ===")
    print(
        f"calibration {cal_ms:.1f} ms (reference {CAL_REFERENCE_MS:.1f}); "
        f"1 unit = 1 second on the reference machine; "
        f"best of {runs} run(s); floor {floor:g}, tolerance ±{tolerance * 100:.0f}%"
    )
    if runs < 2:
        print(
            "  NOTE: one run measures cache warmth as much as speed — see the "
            "module docstring; use `run` (best of 2) before believing a drift."
        )
    # Distance from the reference is NOT a warning: it only sets the unit scale,
    # and it cancels in a comparison — `(s_now/cal_now) / (s_then/cal_then)` does
    # not contain it.  Worth stating so nobody reads a slow box as a problem.
    if abs(cal_ms / CAL_REFERENCE_MS - 1.0) > 0.15:
        print(
            f"  (this machine measured {cal_ms / CAL_REFERENCE_MS:.2f}x the reference — "
            f"absolute units are not comparable across machines, drift still is)"
        )
    # What DOES invalidate a reading: the machine changing mid-run.  The scaling
    # applies one number to the whole run, so if the box was quiet at the start
    # and busy at the end, half the tests are scaled by the wrong figure and the
    # drift below is a statement about the load rather than about the code.
    if drift_unreliable is not None:
        lo, hi = drift_unreliable
        print(
            f"  ⚠ the machine changed during the run — calibrated {lo:.1f} ms before "
            f"and {hi:.1f} ms after ({max(lo, hi) / max(min(lo, hi), 1e-9):.2f}x). One "
            f"scale cannot describe both halves; treat the drift below as UNRELIABLE."
        )
    print(f"{len(measured)} tests timed, {sum(1 for a in annots.values() for x in a if x.units is not None)} annotated")
    print()

    if drift:
        print(f"--- drifted beyond ±{tolerance * 100:.0f}% ({len(drift)}) ---")
        for ratio, key, was, now in sorted(drift, key=lambda d: -d[0]):
            arrow = "SLOWER" if ratio > 1 else "faster"
            print(f"  {arrow:>6}  {ratio:5.2f}x  {was:8.2f} → {now:8.2f}  {key}")
        print()
    else:
        print("--- no test drifted beyond the tolerance ---\n")

    if new:
        print(f"--- slow and unannotated ({len(new)}) ---")
        for units, key in sorted(new, reverse=True)[:20]:
            print(f"          {units:8.2f}  {key}")
        if len(new) > 20:
            print(f"          … and {len(new) - 20} more")
        print()

    if gone:
        print(f"--- annotated but no longer slow ({len(gone)}) ---")
        for key, was in sorted(gone)[:20]:
            print(f"          was {was:6.2f}  {key}")
        print()

    if bless:
        n = apply_edits(edits)
        print(f"blessed {n} annotation(s) — review the diff before committing")
    elif drift or new or gone:
        print("re-run with --bless to write these numbers into the tests")


def apply_edits(edits: dict[Path, list[tuple[int, int | None, float]]]) -> int:
    """Write the annotations, one file at a time, bottom-up.

    Bottom-up because inserting a line shifts every index below it; going
    backwards means the indices computed against the original file stay correct.
    A `units` of -1 removes the annotation.
    """
    total = 0
    for path, items in edits.items():
        lines = path.read_text(encoding="utf-8").splitlines()
        for fn_line, annot_line, units in sorted(items, key=lambda t: -t[0]):
            indent = re.match(r"^\s*", lines[fn_line]).group(0)
            if units < 0:
                if annot_line is not None:
                    del lines[annot_line]
                    total += 1
                continue
            text = f"{indent}// @speed {units:.1f}"
            if annot_line is not None:
                if lines[annot_line] != text:
                    lines[annot_line] = text
                    total += 1
            else:
                lines.insert(fn_line, text)
                total += 1
        path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return total


def curated_filter() -> str:
    """The nextest filterset `find_problems.sh` runs by default.

    Read from `scripts/test_subjects.sh` rather than restated here: the list of
    heavy binaries has ONE home, and a speed report measuring a different set
    than the gate would invite exactly the comparison it cannot support.

    It also has to be this set for the measurement to hold up at all. The heavy
    eight are the browser and wasm binaries; run alongside everything else they
    contend for browsers, ports and cores, and several of them simply fail —
    which is why the gate curates them out, and why timing them in that company
    would measure the contention rather than the test.
    """
    out = subprocess.run(
        ["bash", "-c", "source scripts/test_subjects.sh && curated_filter"],
        cwd=REPO,
        capture_output=True,
        text=True,
        check=False,
    )
    return out.stdout.strip()


def annotated_filter(annots: dict[str, list[Annotation]]) -> str:
    """A nextest filterset naming exactly the annotated tests.

    Exact names (`test(=…)`), never substrings: `test(foo)` also matches
    `foo_and_more`, which would silently drag in neighbours and make the serial
    pass measure a different set than the one being reported on.
    """
    names = sorted(
        {name for name, sites in annots.items() if any(s.units is not None for s in sites)}
    )
    if not names:
        return ""
    return " + ".join(f"test(={n})" for n in names)


def run_suite(extra: list[str], full: bool, repeat: int) -> tuple[float, list[Measured]]:
    """Run the suite `repeat` times and keep each test's FASTEST observation.

    Calibrated at both ends because a run takes minutes and a box's load can
    change inside one; measuring only before would attribute someone else's
    compile to whichever tests happened to run while it was going on.

    The minimum, not the mean: see the module docstring. The first run of a cold
    tree pays for builds, cdylibs and page cache, and that cost dwarfs the thing
    being measured — one run reported 113 of 139 tests as having changed when
    nothing had.
    """
    before = calibrate()
    print(f"calibration before: {before:.1f} ms", file=sys.stderr)
    select: list[str] = []
    serial: list[str] = []
    if not any(a == "-E" or a.startswith("-E") for a in extra):
        if full:
            f = curated_filter()
        else:
            f = annotated_filter(scan_annotations())
            if not f:
                print(
                    "nothing is annotated yet — run `discover --bless` first",
                    file=sys.stderr,
                )
                return calibrate(), []
            # One at a time: the whole point of measuring only this set.
            serial = ["--test-threads=1"]
        if f:
            select = ["-E", f]
    cmd = [
        "cargo",
        "nextest",
        "run",
        "--release",
        "--no-fail-fast",
        "--profile",
        PROFILE,
        *select,
        *serial,
        *extra,
    ]
    best: dict[str, Measured] = {}
    for run in range(1, repeat + 1):
        print(f"$ [{run}/{repeat}] {' '.join(cmd)}", file=sys.stderr)
        subprocess.run(cmd, cwd=REPO, check=False)
        for m in read_junit(JUNIT):
            prev = best.get(m.key)
            if prev is None or m.seconds < prev.seconds:
                best[m.key] = m
    after = calibrate()
    print(f"calibration after:  {after:.1f} ms", file=sys.stderr)
    global CAL_ENDS
    CAL_ENDS = (before, after)
    return (before + after) / 2, list(best.values())


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("mode", choices=["run", "discover", "report", "calibrate"])
    ap.add_argument("--bless", action="store_true", help="write the numbers into the tests")
    ap.add_argument("--floor", type=float, default=1.0, help="annotate at or above this many units")
    ap.add_argument("--tolerance", type=float, default=0.25, help="report drift beyond this fraction")
    ap.add_argument("--cal", type=float, default=None, help="skip calibration, use this ms")
    ap.add_argument(
        "--full",
        action="store_true",
        help="time every binary, not the curated set (they contend; see curated_filter)",
    )
    ap.add_argument(
        "--repeat",
        type=int,
        default=2,
        help="runs to take the per-test minimum over (default 2; 1 measures cache warmth)",
    )
    ap.add_argument("rest", nargs="*", help="extra args passed to nextest (run mode)")
    args = ap.parse_args()

    if args.mode == "calibrate":
        print(f"{calibrate():.1f} ms  (reference {CAL_REFERENCE_MS:.1f} ms)")
        return 0

    cal = args.cal
    if args.mode in ("run", "discover"):
        # `discover` is the wide parallel pass that finds candidates; `run` is the
        # serial pass over the annotated set that produces numbers worth comparing.
        wide = args.mode == "discover"
        repeat = 1 if wide else max(1, args.repeat)
        cal, measured = run_suite(args.rest, args.full or wide, repeat)
        runs = repeat
    else:
        if cal is None:
            cal = calibrate()
        measured, runs = read_junit(JUNIT), 1

    unreliable = None
    if CAL_ENDS:
        lo, hi = min(CAL_ENDS), max(CAL_ENDS)
        if hi / max(lo, 1e-9) > 1.20:
            unreliable = CAL_ENDS
    report(cal, args.floor, args.tolerance, args.bless, measured, runs, unreliable)
    # Speed never fails a build.  See the module docstring.
    return 0


if __name__ == "__main__":
    sys.exit(main())
