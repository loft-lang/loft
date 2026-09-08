#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The liveness census: are the gates themselves still live?  A REPORT, never a gate.

@PLN156 phase 5.  2026.8.0's rescue was this census done by hand, once: several "done"
things — the registry entry, the acquisition chain, the anchor — had never been true,
and nothing surfaced that because each gate's silence read as green.  The general rule
the release checklist now enforces (UNKNOWN never aggregates into a pass) covers the
release window; this report covers the DRIFT BETWEEN releases, so it accumulates in
weeks instead of surfacing at the reckoning.

Three censuses, each naming what a reader should act on:

  rationales   every `#[ignore]` rationale and skip-list note that cites an issue —
               is that issue still OPEN?  A suppression justified by a CLOSED issue is
               a gate someone forgot to rearm.
  workflows    for each scheduled/dispatched gate workflow: when did it last actually
               RUN on main, and how did it conclude?  A nightly that has quietly not
               fired in a week is a green badge over nothing.
  checklist    manual items never recorded as done in ANY committed cycle — the steps
               most likely to have never been run at all (the 2026.8.0 class), and the
               automatic items whose latest recorded state is not a pass.

Usage:  scripts/release-liveness.py [--no-network]
Network (gh) is used for issue states and workflow runs; without it those sections say
SKIPPED rather than printing an empty table that reads as clean.
"""

from __future__ import annotations

import argparse
import glob
import json
import os
import re
import datetime
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
REPO = "loft-lang/loft"

# The workflows that ARE gates, with how they are meant to fire.  "scheduled" means
# silence IS drift — a nightly that stopped firing is a green badge over nothing.
# "dispatch" means silence is normal (a deliberate, occasional probe: the release gate,
# the Windows rlib escape-hatch) — its age is reported without the alarm, because an
# alarm that fires on the expected state teaches readers to ignore the census.
# Pure-reaction workflows (labelling, close-on-merge) are not gates and are not here.
GATE_WORKFLOWS = [
    ("ci.yml", "scheduled"),
    ("release-gate.yml", "dispatch"),
    ("registry-validation.yml", "scheduled"),
    ("miri.yml", "scheduled"),
    ("api-compat.yml", "dispatch"),
    ("lib-main-health.yml", "scheduled"),
    ("browser-threads.yml", "scheduled"),
    ("win-cdylib.yml", "dispatch"),
]


def sh(*args: str, timeout: int = 60) -> tuple[int, str]:
    try:
        p = subprocess.run(
            args, cwd=ROOT, capture_output=True, text=True, timeout=timeout
        )
        return p.returncode, (p.stdout + p.stderr).strip()
    except FileNotFoundError:
        return 127, f"{args[0]}: not installed"
    except subprocess.TimeoutExpired:
        return 124, f"{args[0]}: timed out after {timeout}s"


def cited_issues(text: str) -> list[str]:
    """Issue numbers a rationale cites: `#123`, `loft#123`."""
    return sorted({m for m in re.findall(r"(?:loft)?#(\d{2,5})\b", text)})


def issue_states(numbers: set[str], network: bool) -> dict[str, str]:
    """number -> OPEN/CLOSED, in one query per 50."""
    if not network or not numbers:
        return {}
    states: dict[str, str] = {}
    # One `gh issue view` per number: exact, and the census is small.
    for n in sorted(numbers, key=int):
        code, out = sh("gh", "issue", "view", n, "-R", REPO, "--json", "state", timeout=30)
        if code == 0:
            try:
                states[n] = json.loads(out).get("state", "?")
            except json.JSONDecodeError:
                states[n] = "?"
    return states


def census_rationales(network: bool) -> None:
    print("== rationales: suppressions justified by an issue — is it still open? ==")
    rows: list[tuple[str, str, list[str]]] = []  # (where, rationale, issues)
    baseline = os.path.join(ROOT, "tests", "ignored_tests.baseline")
    if os.path.isfile(baseline):
        for line in open(baseline, encoding="utf-8"):
            if not line.strip() or line.startswith("#"):
                continue
            name, _, why = line.rstrip("\n").partition("\t")
            issues = cited_issues(why)
            if issues:
                rows.append((name, why.strip(), issues))
    # Skip-list entries with issue citations, wherever tests spell them.
    code, out = sh(
        "grep", "-rn", "-E", "(SCRIPTS_NATIVE_SKIP|NATIVE_SKIP|ignored_scripts)",
        "tests", "--include=*.rs", "-A", "1",
    )
    if code == 0:
        for line in out.splitlines():
            issues = cited_issues(line)
            if issues and ("#" in line):
                rows.append((line.split(":")[0], line.strip()[:100], issues))
    if not rows:
        print("  no issue-cited suppressions found")
        return
    states = issue_states({n for _, _, ns in rows for n in ns}, network)
    if not states:
        print(f"  {len(rows)} suppression(s) cite issues — states SKIPPED (no network/gh)")
        return
    stale = 0
    for where, why, issues in rows:
        closed = [n for n in issues if states.get(n) == "CLOSED"]
        if closed:
            stale += 1
            print(f"  STALE  {where}")
            print(f"         cites closed #{', #'.join(closed)}: {why[:90]}")
    live = len(rows) - stale
    print(f"  {live} suppression(s) cite open issues (live); {stale} cite CLOSED ones — rearm or re-justify those")


def census_workflows(network: bool) -> None:
    print("\n== workflows: when did each GATE last actually run, and how? ==")
    if not network:
        print("  SKIPPED (no network) — a table not printed is not a table that is clean")
        return
    for wf, kind in GATE_WORKFLOWS:
        code, out = sh(
            "gh", "run", "list", "--workflow", wf, "-R", REPO,
            "--json", "conclusion,createdAt,headBranch", "--limit", "1", timeout=60,
        )
        if code != 0:
            print(f"  {wf:<28} {kind:<10} cannot read runs: {out.splitlines()[-1] if out else code}")
            continue
        try:
            runs = json.loads(out or "[]")
        except json.JSONDecodeError:
            runs = []
        if not runs:
            print(f"  {wf:<28} {kind:<10} NEVER RAN — a gate that has never fired gates nothing")
            continue
        r = runs[0]
        when = r.get("createdAt", "")[:10]
        concl = r.get("conclusion") or "in flight"
        try:
            age = (
                datetime.datetime.now(datetime.timezone.utc)
                - datetime.datetime.fromisoformat(r["createdAt"].replace("Z", "+00:00"))
            ).days
        except (KeyError, ValueError):
            age = -1
        flag = ""
        if age > 7 and kind == "scheduled":
            flag = f"  <- has not fired in {age} days"
        if concl not in ("success", "in flight"):
            flag += f"  <- last verdict: {concl}"
        print(f"  {wf:<28} {kind:<10} {when}  {concl}{flag}")


def census_checklist() -> None:
    print("\n== checklist: items never recorded as run in ANY committed cycle ==")
    # Every manual item the CURRENT checklist defines...
    sys.path.insert(0, os.path.join(ROOT, "scripts"))
    import importlib.util

    spec = importlib.util.spec_from_file_location(
        "release_checklist", os.path.join(ROOT, "scripts", "release-checklist.py")
    )
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    sections = mod.build_items(mod.cargo_version(), network=False)
    manual = [i.id for _, items in sections for i in items if not i.automatic]
    # ...against every recorded tick across the committed cycles.
    ticked: set[str] = set()
    cycles = sorted(glob.glob(os.path.join(ROOT, "doc", "claude", "releases", "*", "checklist.json")))
    for p in cycles:
        try:
            ticked |= set(json.load(open(p, encoding="utf-8")))
        except (OSError, json.JSONDecodeError):
            pass
    never = [i for i in manual if i not in ticked]
    print(f"  {len(cycles)} recorded cycle(s); {len(manual)} manual items in the current list")
    if never:
        for i in never:
            print(f"  NEVER RUN  {i} — no cycle has ever recorded it (the 2026.8.0 class)")
    else:
        print("  every current manual item has been run in at least one recorded cycle")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--no-network", action="store_true")
    args = ap.parse_args()
    network = not args.no_network
    print("release-liveness — the census of whether the gates are live (a REPORT, never a gate)\n")
    census_rationales(network)
    census_workflows(network)
    census_checklist()
    print(
        "\nact on: STALE rationales (rearm or re-justify), gates that stopped firing,"
        "\nand NEVER-RUN items (run them once or retire them) — silence is the defect."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
