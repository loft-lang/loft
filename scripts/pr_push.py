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


# ---- the last run's failures, rechecked before the push ------------------------------------
#
# A fix pushed to a PR is meant for a red its last run showed, so that red is rerun here, and
# only that: not the gate.  The failures are grouped by their SIGNATURE (the first line of the
# panic or the refusal), and one test per signature is rerun: when most of a run is red the
# failures share one cause (a wrong directory, a missing tool, a pin), which one representative
# shows as well as all of them would, at the cost of a test instead of a CI.  A job with no test
# failure in its log failed in its setup or a script step (checkout, apt, a toolchain); it and
# the informational mirrors of main's nightly are listed, never rerun.

MAX_RERUNS = 12

TS = r"^\S+Z "  # a GitHub log line's timestamp


def failures_of(log):
    """(kind, name, signature) for each failure a job's log names."""
    import re
    ansi = re.compile(r"\x1b\[[0-9;]*m")
    lines = [ansi.sub("", re.sub(TS, "", l)) for l in log.splitlines()]
    out, msg = [], {}
    for i, l in enumerate(lines):
        m = re.search(r"thread '([^']+)' \(\d+\) panicked at", l)
        if m and i + 1 < len(lines):
            msg[m.group(1)] = lines[i + 1].strip()
    in_block = False
    for l in lines:
        if l.strip() == "failures:":
            in_block = True
            continue
        if in_block:
            m = re.match(r"^\s{4,}(\S+::\S+)\s*$", l)
            if m:
                out.append(("cargo", m.group(1), msg.get(m.group(1), "")))
            elif l.strip().startswith("test result:"):
                in_block = False
    for l in lines:
        m = re.search(r"\bFAIL \[\s*[\d.]+s\] (?:\(\d+/\d+\) )?(\S+) (\S+)\s*$", l)
        if m:
            out.append(("nextest", f"{m.group(1)} {m.group(2)}", msg.get(m.group(2), "")))
        m = re.match(r"^\s*FAIL\s+(interpret|c|native)\s+(\S+\.loft)\s+\(", l)
        if m:
            out.append(("suite", f"{m.group(1)} {m.group(2)}", ""))
        m = re.match(r"^- (\S+\.patch): (.*)$", l)
        if m:
            # Each stale receipt is its own cause (its own patch against its own lines): no
            # signature groups two of them.
            out.append(("receipt", m.group(1), m.group(1)))
    seen, uniq = set(), []
    for f in out:
        if (f[0], f[1]) not in seen:
            seen.add((f[0], f[1]))
            uniq.append(f)
    return uniq


def rerun_one(kind, name, loft):
    """Rerun one failure locally; True when it passes now."""
    if kind == "receipt":
        return run("git", "apply", "--check", f"tests/falsified/{name}").returncode == 0
    if kind == "cargo":
        for target in (["--lib"], ["--tests"]):
            r = run("cargo", "test", "--release", *target, name, "--", "--exact")
            if "0 passed; 0 failed" not in r.stdout:
                return r.returncode == 0
        return False
    if kind == "nextest":
        binary, test = name.split(" ", 1)
        binary = binary.split("::")[-1]
        r = run("cargo", "test", "--release", "--test", binary, test, "--", "--exact")
        return r.returncode == 0
    if kind == "suite":
        import importlib.machinery
        import os
        suite = os.environ.get("LOFT_SUITE_DIR") or str(ROOT.parent / "loft-suite")
        runner = importlib.machinery.SourceFileLoader(
            "loft_suite_runner", str(Path(suite) / "loft-suite")).load_module()
        mode, cell = name.split(" ", 1)
        res = runner.run_cell(loft, str(Path(suite) / cell), mode, 300)
        return bool(res.ok)
    return False


def recheck_last_run(pr):
    """Rerun one failure per signature of the PR's last run; False when one still fails."""
    r = run("gh", "pr", "checks", str(pr), "--json", "name,state,link")
    try:
        checks = json.loads(r.stdout or "[]")
    except json.JSONDecodeError:
        checks = []
    failed = [c for c in checks if c.get("state") == "FAILURE"]
    if not failed:
        print("the last run has no failed check: nothing to recheck")
        return True
    groups, listed = {}, []
    for c in failed:
        if "(informational)" in c["name"]:
            listed.append(f"{c['name']} (mirrors main)")
            continue
        job = c.get("link", "").rstrip("/").split("/")[-1]
        log = run("gh", "api", f"repos/loft-lang/loft/actions/jobs/{job}/logs").stdout
        found = failures_of(log)
        if not found:
            listed.append(f"{c['name']} (no test failure in its log: setup or a script step)")
            continue
        for kind, name, sig in found:
            groups.setdefault(sig or name, []).append((kind, name))
    for line in listed:
        print(f"  listed, not rerun: {line}")
    total = sum(len(v) for v in groups.values())
    print(f"the last run: {total} failure(s) in {len(groups)} signature(s); rerunning one of each")
    loft = str(ROOT / "target" / "release" / "loft")
    still = []
    for n, (sig, members) in enumerate(sorted(groups.items(), key=lambda kv: -len(kv[1]))):
        if n >= MAX_RERUNS:
            print(f"  … {len(groups) - MAX_RERUNS} more signature(s) not rerun: find their common cause")
            break
        k, name = members[0]
        ok = rerun_one(k, name, loft)
        print(f"  {'ok  ' if ok else 'RED '} {k} {name}" + (f"  (+{len(members) - 1} sharing: {sig[:70]})" if len(members) > 1 else ""))
        if not ok:
            still.append(name)
    return not still


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
    ap.add_argument("--no-recheck", action="store_true",
                    help="skip rerunning the last run's failures (one per signature)")
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
    if not args.no_recheck and not recheck_last_run(args.pr):
        sys.exit("a failure of the last run is still red here: fix it on this branch, then run this again")

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
    # Each branch is pushed AS ITSELF.  The PR's head is moved here first with `git fetch .`,
    # which git refuses unless it is a fast-forward (checked above too), so a cross-name push
    # (`HEAD:refs/heads/<head>`) is never needed: a session's permission check reads that
    # spelling as writing over another branch and refuses it.
    if branch != "HEAD" and branch != pr["headRefName"]:
        run("git", "push", "-q", "origin", branch, check=True)
        run("git", "fetch", "-q", ".", f"HEAD:refs/heads/{pr['headRefName']}", check=True)
    elif branch == "HEAD":
        run("git", "fetch", "-q", ".", f"HEAD:refs/heads/{pr['headRefName']}", check=True)
    run("git", "push", "-q", "origin", pr["headRefName"], check=True)
    print(f"pushed {pr_head[:9]}..{head[:9]} to {pr['headRefName']}"
          + (f" (and {branch})" if branch not in ("HEAD", pr["headRefName"]) else ""))
    watch = ["python3", "scripts/pr_watch.py", str(args.pr), "--no-nightly"]
    if args.watch:
        return subprocess.run(watch, cwd=ROOT).returncode
    print("watch the new run (in the background): " + " ".join(watch[1:]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
