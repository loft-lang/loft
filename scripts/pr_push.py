#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Push the next branch to an open PR — only when the PR's run is over and the push is checked.

    scripts/pr_push.py 2495             # check, then push HEAD to the next branch and the PR
    scripts/pr_push.py 2495 --dry-run   # every check, no push
    scripts/pr_push.py 2495 --watch     # …and then wait on the new run (scripts/pr_watch.py)

Why
---
A push to a PR starts a run that takes an hour and more, so a red a local check could have
shown costs that hour.  The PR cycle (DEVELOPMENT.md § The PR cycle) has two hard rules about
this push — nothing is pushed while the PR's run is in flight, and the fixes go out together
once it has finished — and a handful of checks that were made by hand before it.  This script
makes each one, and refuses with the reason:

1. the PR is open and NONE of its checks is still queued or running;
2. the tree has no uncommitted change, HEAD holds the PR's head (a fast-forward, never a
   force) and is current on `origin/main` (CLAUDE.md § Branch policy rule 6);
3. `scripts/after.py fix --pr` has passed on exactly this HEAD, over a diff that covers
   everything since the PR's head (`.git/loft-after/last-pr`) — when it has not, it is run
   here, and a red step refuses the push.

Then HEAD is pushed to this branch's own name and to the PR's branch, and the command that
watches the new run is printed (`--watch` runs it).
"""
import argparse
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
STATE = Path(subprocess.run(["git", "rev-parse", "--absolute-git-dir"], cwd=ROOT,
                            capture_output=True, text=True, check=True).stdout.strip()) / "loft-after"


def run(*cmd, check=False):
    r = subprocess.run(list(cmd), cwd=ROOT, capture_output=True, text=True)
    if check and r.returncode != 0:
        sys.exit(f"{' '.join(cmd)}: {r.stderr.strip()}")
    return r


def git(*args):
    return run("git", *args).stdout.strip()


def ancestor(a, b):
    return run("git", "merge-base", "--is-ancestor", a, b).returncode == 0


def in_flight(checks):
    """The checks still queued or running: a CheckRun not COMPLETED, a status still PENDING."""
    out = []
    for c in checks:
        if c.get("__typename") == "StatusContext":
            if c.get("state") in ("PENDING", "EXPECTED"):
                out.append(c.get("context", "?"))
        elif c.get("status") != "COMPLETED":
            out.append(c.get("name", "?"))
    return out


def covered(head, pr_head):
    """Did `after.py fix --pr` pass on HEAD over a diff reaching back to the PR's head?"""
    f = STATE / "last-pr"
    if not f.exists():
        return False
    parts = f.read_text(encoding="utf-8").split()
    return len(parts) >= 2 and parts[0] == head and ancestor(parts[1], pr_head)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("pr", type=int)
    ap.add_argument("--dry-run", action="store_true", help="make every check, push nothing")
    ap.add_argument("--watch", action="store_true", help="after the push, wait on the new run")
    args = ap.parse_args()
    sys.stdout.reconfigure(line_buffering=True)

    r = run("gh", "pr", "view", str(args.pr), "--json",
            "state,headRefName,headRefOid,statusCheckRollup", check=True)
    pr = json.loads(r.stdout)
    if pr["state"] != "OPEN":
        sys.exit(f"PR #{args.pr} is {pr['state'].lower()}: nothing to push to")
    busy = in_flight(pr.get("statusCheckRollup") or [])
    if busy:
        shown = ", ".join(busy[:5]) + (f" +{len(busy) - 5}" if len(busy) > 5 else "")
        sys.exit(f"PR #{args.pr}'s run is still in flight ({len(busy)} check(s): {shown}).  Nothing "
                 "is pushed to a running PR; push once it has finished (DEVELOPMENT.md § The PR cycle)")

    run("git", "fetch", "-q", "origin", "main", pr["headRefName"], check=True)
    pr_head, head = pr["headRefOid"], git("rev-parse", "HEAD")
    branch = git("rev-parse", "--abbrev-ref", "HEAD")
    if git("status", "--porcelain", "--untracked-files=no"):
        sys.exit("the tree has uncommitted changes: commit them, or they are not what is pushed")
    if head == pr_head:
        sys.exit(f"HEAD is the PR's head already ({head[:9]}): nothing to push")
    if not ancestor(pr_head, head):
        sys.exit(f"HEAD does not hold the PR's head {pr_head[:9]}: the push would not be a "
                 "fast-forward.  Rebase the next branch on it; a PR branch is never force-pushed")
    if not ancestor("origin/main", head):
        sys.exit("HEAD is not current on origin/main: rebase first (CLAUDE.md § Branch policy rule 6)")
    print(f"PR #{args.pr}: run finished; {head[:9]} fast-forwards {pr_head[:9]} and is current on main")

    if covered(head, pr_head):
        print("after.py fix --pr passed on this HEAD")
    else:
        print(f"running scripts/after.py fix --pr --since {pr_head[:9]} …")
        if subprocess.run(["python3", "scripts/after.py", "fix", "--pr", "--since", pr_head],
                          cwd=ROOT).returncode != 0 or not covered(head, pr_head):
            sys.exit("the pre-push checks are not green: fix them on this branch, then run this again")

    if args.dry_run:
        print(f"dry run: would push {head[:9]} to {branch} and to {pr['headRefName']}")
        return 0
    if branch != "HEAD" and branch != pr["headRefName"]:
        run("git", "push", "-q", "origin", f"HEAD:refs/heads/{branch}", check=True)
    run("git", "push", "-q", "origin", f"HEAD:refs/heads/{pr['headRefName']}", check=True)
    print(f"pushed {pr_head[:9]}..{head[:9]} to {pr['headRefName']}"
          + (f" (and {branch})" if branch not in ("HEAD", pr["headRefName"]) else ""))
    watch = ["python3", "scripts/pr_watch.py", str(args.pr), "--no-nightly"]
    if args.watch:
        return subprocess.run(watch, cwd=ROOT).returncode
    print("watch the new run (in the background): " + " ".join(watch[1:]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
