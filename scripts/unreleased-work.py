#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Does this library repo carry work that never reached anyone?  Three shapes of it, one
# question: a branch, a pull request, or a version bump that has stopped moving and has
# not been published.
#
#   orphan branch     commits ahead of main, no open PR, not merged — nobody can find it
#   stale open PR     open, and nothing has touched it
#   unpublished       loft.toml names a version the registry index does not have
#
# WHY THIS IS A PR GATE AND NOT A REPORT.  `registry_maintain.sh` already lists the first
# two, but only the maintainer sees that, only during a publish run, and by then the branch
# has usually been sitting for months (five in loft-libs-graphics, the oldest from June).
# A red step on every PR puts the fact in front of whoever is already working in the repo,
# which is the only moment somebody knows whether the branch is worth keeping.
#
# ONE THRESHOLD, ONE AXIS: days since the last ACTIVITY.  Work touched yesterday is not
# lying around whatever its age, and a branch nobody has touched in weeks is, whether it is
# one commit or thirty.  Age-since-creation would redden long-running work that is actively
# moving, which teaches people to ignore the step.
#
# A MERGED-BUT-UNDELETED branch is reported and never red: its work landed, so the only
# thing wrong is the leftover ref.
#
# Usage:
#   scripts/unreleased-work.py --repo <owner/name> [--packages '["a","b"]']
#                              [--stale-days N] [--summary <file>]
#
# Needs `gh` authenticated (GH_TOKEN in CI).  Exits 1 when anything is red.

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import urllib.request
from datetime import datetime, timezone

REGISTRY_INDEX = "https://raw.githubusercontent.com/loft-lang/registry/main/index.json"


def gh_json(args: list[str], default):
    """`gh` returning parsed JSON, or `default` when the call fails.

    A failure here is a missing answer, not a finding: this gate must not turn red
    because a token lacked a scope or the API blipped.  Every caller reports what it
    could not read instead of counting it as clean.
    """
    r = subprocess.run(["gh", *args], capture_output=True, text=True)
    if r.returncode != 0:
        return default
    try:
        return json.loads(r.stdout or "null")
    except json.JSONDecodeError:
        return default


def days_since(stamp: str | None) -> float | None:
    """Whole days between an ISO-8601 UTC stamp and now, or None if unreadable."""
    if not stamp:
        return None
    try:
        t = datetime.fromisoformat(stamp.replace("Z", "+00:00"))
    except ValueError:
        return None
    return (datetime.now(timezone.utc) - t).total_seconds() / 86400.0


def registry_versions() -> dict[str, list[str]] | None:
    """Every published version per package, or None when the index cannot be read."""
    try:
        with urllib.request.urlopen(REGISTRY_INDEX, timeout=30) as r:
            index = json.loads(r.read().decode("utf-8"))
    except Exception:  # network, TLS, JSON — all the same answer here
        return None
    return {n: list(p.get("versions") or {}) for n, p in index.get("packages", {}).items()}


def manifest_version(pkg: str) -> str | None:
    """The `version` in a package's loft.toml, read from the checkout."""
    try:
        with open(f"{pkg}/loft.toml", encoding="utf-8") as f:
            for line in f:
                s = line.strip()
                if s.startswith("version"):
                    return s.split("=", 1)[1].strip().strip('"')
    except OSError:
        return None
    return None


def branch_verdict(name, tip, ahead, age, merged, limit, default_branch="main"):
    """One branch's verdict: `merged` (delete the ref), `orphan` (red) or `moving`.

    `merged` is a set of (branch name, head commit) pairs of merged PRs.  A branch is merged
    when nothing is ahead of the default branch, or when its TIP is a head a PR merged — a
    squash-merge leaves the branch "ahead" while its work landed.  A merged PR vouches for
    that commit only: new commits on the same name are unmerged work.
    """
    if ahead == 0 or (name, tip) in merged:
        return "merged", f"branch `{name}` is merged or behind — delete the ref"
    if age is not None and age > limit:
        return "orphan", (
            f"branch `{name}` — {ahead} commit(s) ahead of {default_branch}, no open PR, "
            f"last touched {age:.0f}d ago.  Open a PR for it, or delete it."
        )
    return "moving", f"branch `{name}` — {ahead} ahead, no PR yet, still moving"


def _self_test() -> int:
    merged = {("sq", "aaa"), ("reused", "old")}
    cases = [
        (("sq", "aaa", 1, 40, merged, 14), "merged", "a squash-merged branch at the merged tip"),
        (("reused", "new", 3, 1, merged, 14), "moving", "a merged name with new commits is not merged"),
        (("reused", "new", 3, 30, merged, 14), "orphan", "…and goes red once it stops moving"),
        (("behind", "x", 0, 90, merged, 14), "merged", "nothing ahead of the default branch"),
        (("fresh", "y", 2, 3, merged, 14), "moving", "recent unmerged work is still moving"),
    ]
    bad = 0
    for args, want, what in cases:
        got = branch_verdict(*args)[0]
        ok = got == want
        bad += not ok
        print(f"  {'ok  ' if ok else 'FAIL'}  {what} ({got})")
    print("self-test: every branch rule acts on an input that needs it" if not bad
          else f"self-test FAILED: {bad} rule(s) did not hold")
    return 1 if bad else 0


def main() -> int:
    if "--self-test" in sys.argv[1:]:
        return _self_test()
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", required=True, help="owner/name")
    ap.add_argument("--packages", default="[]", help="JSON array of package dirs")
    ap.add_argument("--stale-days", type=float, default=14.0)
    ap.add_argument("--summary", default=None, help="also write a markdown summary here")
    a = ap.parse_args()
    repo, limit = a.repo, a.stale_days
    try:
        packages = json.loads(a.packages) or []
    except json.JSONDecodeError:
        print(f"::error::--packages is not a JSON array: {a.packages}")
        return 2

    red: list[str] = []
    notes: list[str] = []
    unknown: list[str] = []

    # ---- branches -----------------------------------------------------------------
    # Three list calls up front rather than a query per branch: the answer for one
    # branch needs the whole open-PR and merged-PR picture anyway.
    open_prs = gh_json(
        ["pr", "list", "-R", repo, "--state", "open", "--limit", "200",
         "--json", "number,title,headRefName,updatedAt,isDraft"], None)
    merged_heads = gh_json(
        ["pr", "list", "-R", repo, "--state", "merged", "--limit", "300",
         "--json", "headRefName,headRefOid"], None)
    branches = gh_json(["api", f"repos/{repo}/branches?per_page=100"], None)

    if open_prs is None or merged_heads is None or branches is None:
        unknown.append("branches / pull requests could not be listed (gh call failed)")
        open_prs, merged_heads, branches = open_prs or [], merged_heads or [], branches or []

    open_heads = {p["headRefName"] for p in open_prs}
    # A merged PR vouches for the COMMIT it merged, not for the branch name: a branch that
    # was merged once and then received new commits (`157-native-4x` after #1524,
    # `tuxedo-165-generics` after #1667) carries work no PR has landed.  Keyed on the name
    # alone, both read "merged or behind — delete the ref", and one was deleted with a
    # commit on it that existed nowhere else (2026-09-25; a re-push restored it).
    merged = {(p["headRefName"], p.get("headRefOid")) for p in merged_heads}
    # The whole repo object, not `--jq .default_branch`: gh would print the value
    # unquoted, which is not JSON, so every read would silently take the fallback.
    repo_info = gh_json(["api", f"repos/{repo}"], None) or {}
    default_branch = repo_info.get("default_branch") or "main"

    for b in branches:
        name = b["name"]
        if name == default_branch or name in open_heads:
            continue                      # the default branch, or a branch under review
        cmp_ = gh_json(["api", f"repos/{repo}/compare/{default_branch}...{name}"], None)
        if cmp_ is None:
            unknown.append(f"branch `{name}` could not be compared against {default_branch}")
            continue
        ahead = cmp_.get("ahead_by", 0)
        commits = cmp_.get("commits") or []
        last = commits[-1]["commit"]["committer"]["date"] if commits else None
        age = days_since(last)
        # A squash-merged branch reads as "ahead" — its commits are not on the default
        # branch individually — while its work DID land.  The merged PR is what tells
        # the two apart, so ask that before calling anything orphaned.
        tip = (b.get("commit") or {}).get("sha")
        kind, msg = branch_verdict(name, tip, ahead, age, merged, limit, default_branch)
        (red if kind == "orphan" else notes).append(msg)

    for p in open_prs:
        age = days_since(p.get("updatedAt"))
        draft = " (draft)" if p.get("isDraft") else ""
        if age is not None and age > limit:
            red.append(
                f"PR #{p['number']}{draft} \"{p['title']}\" — nothing has touched it for "
                f"{age:.0f}d.  Merge it, or close it."
            )

    # ---- versions on the default branch the registry has never seen ---------------
    published = registry_versions()
    if published is None:
        unknown.append("the registry index could not be fetched")
    else:
        for pkg in packages:
            ver = manifest_version(pkg)
            if ver is None:
                unknown.append(f"`{pkg}/loft.toml` has no readable version")
                continue
            if ver in (published.get(pkg) or []):
                continue
            # When the manifest last moved, which is when this claim was made.  Read from
            # the API so a shallow CI checkout (no history) still answers.
            commits = gh_json(
                ["api", f"repos/{repo}/commits?path={pkg}/loft.toml&per_page=1"], None)
            age = days_since(
                commits[0]["commit"]["committer"]["date"] if commits else None)
            have = ", ".join(published.get(pkg) or []) or "nothing"
            if age is None or age > limit:
                red.append(
                    f"`{pkg}` {ver} is not in the registry (it has {have})"
                    + (f", and its loft.toml last moved {age:.0f}d ago" if age else "")
                    + ".  Publish it, or put the version back."
                )
            else:
                notes.append(f"`{pkg}` {ver} not published yet — bumped {age:.0f}d ago")

    # ---- report -------------------------------------------------------------------
    out: list[str] = []
    if red:
        out.append(f"### ❌ unreleased work in `{repo}`\n")
        out += [f"- {r}" for r in red]
    else:
        out.append(f"### ✅ no unreleased work in `{repo}`\n")
    if notes:
        out.append("\nIn flight, not counted against this gate:\n")
        out += [f"- {n}" for n in notes]
    if unknown:
        out.append("\n⚠ not checked (reported, never counted as clean):\n")
        out += [f"- {u}" for u in unknown]
    out.append(f"\nStale after {limit:.0f} days without activity.")
    text = "\n".join(out)
    print(text)
    if a.summary:
        with open(a.summary, "a", encoding="utf-8") as f:
            f.write(text + "\n")
    return 1 if red else 0


if __name__ == "__main__":
    sys.exit(main())
