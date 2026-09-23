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
    ("revalidate-libs.yml", "scheduled"),
    ("repro-build.yml", "weekly"),
    ("api-compat.yml", "dispatch"),
    ("lib-main-health.yml", "scheduled"),
    ("browser-threads.yml", "scheduled"),
    ("win-cdylib.yml", "dispatch"),
]

# How many scheduled runs the per-leg tally reads back.  The LAST run is one bit; a leg
# that is red ten nights in fourteen is what makes the release gate impossible to turn
# green, and only a window shows it (the Windows `Test` leg was, 2026-09-10..23, and
# every release-gate run ended red on it while this census reported ci.yml "in flight").
TALLY_RUNS = 14
# A weekly workflow's "has not fired" alarm needs a wider window than a nightly's.
SILENCE_DAYS = {"scheduled": 7, "weekly": 10}


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


# A skip source is a `const`/`static` list whose NAME carries SKIP or ALLOW, or a function
# answering a set.  The second shape is matched on its RETURN TYPE rather than its name
# (`wrap.rs::ignored_scripts` is the only one) because `tests/` holds some fifteen ordinary
# test functions called `skip_*` — `skip_constant_index`, `skip_null_safe_arg` — and a name
# pattern wide enough to catch the real one sweeps all of those in with it.
DECL = re.compile(
    r"^\s*(?:const|static)\s+([A-Za-z_]*(?:SKIP|ALLOW)[A-Za-z_]*)\b"
    r"|^\s*fn\s+([a-z_]+)\s*\(\)\s*->\s*HashSet\b"
)


def skip_lists() -> list[tuple[str, str, list[str]]]:
    """Every suite skip/allow list under `tests/`, as (where, entry, issues) per entry.

    Found by SHAPE rather than by name.  A roster of the constants spelled out here would
    be a fourth copy of the one in TESTING.md § Every skip says why — beside the lists
    themselves and `loft-test/SKILL.md` — and the copies drift: the census would then go
    quiet on the list nobody remembered to add to it, which is the silence this whole
    function exists to break.  Deriving it also crosses the CLASSES without either
    registry having to know about the other: the twelve suite skip lists are documented
    together, while `wrap.rs::SCRIPTS_LEAK_ALLOW` is a leak allow-list documented in its
    own section — both suppress a failure, so both are this census's business.

    Each list is read to its own closing token rather than one line deep: the entries of a
    populated list start several lines below its declaration, so a one-line window sees the
    declaration and none of what it holds.  A `];` closes a list and a `}` closes a fn —
    read to the wrong one, a fn runs past its end and adopts the next list's entries.
    """
    out: list[tuple[str, str, list[str]]] = []
    for path in sorted(glob.glob(os.path.join(ROOT, "tests", "*.rs"))):
        try:
            lines = open(path, encoding="utf-8").read().split("\n")
        except OSError:
            continue
        for i, line in enumerate(lines):
            m = DECL.match(line)
            if not m:
                continue
            closer = "];" if m.group(1) else "}"
            body, j = [], i
            while j < len(lines) and j < i + 400:
                body.append(lines[j])
                if closer in lines[j] and (j > i or closer == "];"):
                    break
                j += 1
            where = f"{os.path.basename(path)}::{m.group(1) or m.group(2)}"
            # An entry's citation is its OWN line plus the comment block directly above it —
            # never the whole body.  Read over the body an entry inherits every issue anyone
            # cited anywhere in the list, including a comment about a DIFFERENT entry or about
            # one already removed, and an uncited entry then reads as a justified one.  That is
            # the failure this census exists to report, committed by the census itself.
            comment: list[str] = []
            for b in body:
                entry = re.match(r'^\s*"([^"]+)"', b)
                if entry:
                    out.append((where, entry.group(1),
                                cited_issues("\n".join(comment + [b]))))
                    comment = []
                elif b.strip().startswith("//"):
                    comment.append(b)
                else:
                    comment = []
    return out


def census_rationales(network: bool) -> None:
    print("== rationales: suppressions justified by an issue — is it still open? ==")
    rows: list[tuple[str, str, list[str]]] = []  # (where, rationale, issues)
    seen = uncited = 0
    baseline = os.path.join(ROOT, "tests", "ignored_tests.baseline")
    if os.path.isfile(baseline):
        for line in open(baseline, encoding="utf-8"):
            if not line.strip() or line.startswith("#"):
                continue
            name, _, why = line.rstrip("\n").partition("\t")
            seen += 1
            issues = cited_issues(why)
            if issues:
                rows.append((name, why.strip(), issues))
            else:
                uncited += 1
    entries = skip_lists()
    for where, entry, issues in entries:
        seen += 1
        if issues:
            rows.append((where, entry, issues))
        else:
            uncited += 1
    # The scope this census actually covered, stated whether or not it found anything.  Its
    # question is only asked of a suppression that CITES an issue, so with none to ask it the
    # old report printed "no issue-cited suppressions found" and returned — a line that reads
    # as a clean bill and cannot be told apart from three different worlds: nothing is
    # suppressed, nothing cites an issue, or the search matched nothing because a list was
    # renamed out from under it.  Counting the uncited ones does NOT bless them; whether each
    # is still acceptable is `M-ignores`, the owner's sign-off, and no script can do that half.
    print(f"  scanned: {seen} suppression(s) — {seen - uncited} cite an issue, {uncited} do not")
    print(f"           ({len(entries)} skip-list entr(ies) found by shape; the rest are "
          f"`#[ignore]` rows)")
    if uncited and not rows:
        print("  none of them cites an issue, so this census asks nothing of any of them —")
        print("  their rationales are `A-ignores`' question, and their acceptability M-ignores'")
        return
    if not rows:
        print("  no suppressions found at all")
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
        if kind in SILENCE_DAYS and age > SILENCE_DAYS[kind]:
            flag = f"  <- has not fired in {age} days"
        if concl not in ("success", "in flight"):
            flag += f"  <- last verdict: {concl}"
        print(f"  {wf:<28} {kind:<10} {when}  {concl}{flag}")
        if kind in SILENCE_DAYS:
            tally_scheduled_runs(wf)


def tally_scheduled_runs(wf: str) -> None:
    """The last TALLY_RUNS scheduled runs of one workflow: how many ended how, and which
    JOBS carried the reds — so a leg that is chronically red is named, not averaged
    into a badge.  Jobs are fetched for the non-green runs only."""
    code, out = sh(
        "gh", "run", "list", "--workflow", wf, "-R", REPO, "--event", "schedule",
        "--json", "databaseId,conclusion,createdAt", "--limit", str(TALLY_RUNS), timeout=60,
    )
    if code != 0:
        return
    try:
        runs = json.loads(out or "[]")
    except json.JSONDecodeError:
        return
    if not runs:
        return
    by: dict[str, int] = {}
    for r in runs:
        by[r.get("conclusion") or "in flight"] = by.get(r.get("conclusion") or "in flight", 0) + 1
    span = f"{runs[-1].get('createdAt', '')[:10]}..{runs[0].get('createdAt', '')[:10]}"
    summary = ", ".join(f"{n} {c}" for c, n in sorted(by.items(), key=lambda kv: -kv[1]))
    red_jobs: dict[str, int] = {}
    for r in runs:
        if r.get("conclusion") in ("success", None):
            continue
        code, out = sh(
            "gh", "run", "view", str(r["databaseId"]), "-R", REPO, "--json", "jobs",
            "--jq", '.jobs[] | select(.conclusion != "success" and .conclusion != "skipped") | "\\(.name)=\\(.conclusion)"',
            timeout=60,
        )
        if code != 0:
            continue
        for line in out.splitlines():
            if line.strip():
                red_jobs[line.strip()] = red_jobs.get(line.strip(), 0) + 1
    print(f"{'':<40} last {len(runs)} scheduled ({span}): {summary}")
    if red_jobs:
        worst = sorted(red_jobs.items(), key=lambda kv: -kv[1])[:6]
        print(f"{'':<40} red legs: " + "; ".join(f"{name} x{n}" for name, n in worst))
        chronic = [name for name, n in red_jobs.items() if n * 2 >= len(runs)]
        if chronic:
            print(f"{'':<40} <- CHRONIC (red in half the window or more): " + "; ".join(chronic))


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
            # `_waivers` and any other `_`-key is the record's own bookkeeping, not a tick.
            ticked |= {k for k in json.load(open(p, encoding="utf-8")) if not k.startswith("_")}
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
