#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Watch a PR's CI run and hand back each red job as it ends (DEVELOPMENT.md § The PR cycle).

The PR's first run is the discovery run: its reds are worked while it is still going, so the
fixes are ready when it finishes.  This is the wait that makes that the default.  Run it as a
background command: it polls every five minutes and EXITS as soon as a job turns red that it
has not reported before, with the job's log saved, so the agent is woken, starts the fix and
runs it again.  It also exits when the run is over and at its deadline: it always ends.

    scripts/pr_watch.py 2418                 # wait for the next new red, or the end of the run
    scripts/pr_watch.py 2418 --once          # one look: what is red, pending and green now
    scripts/pr_watch.py 2418 --interval 120 --timeout 3600
    scripts/pr_watch.py 0 --sha <commit>     # any commit's checks: a probe on the next branch

On its first call for a head sha it also prints main's red scheduled runs (`--no-nightly`
skips that): the PR cycle fixes those in the same pass.  A red REQUIRED check is marked, since
only that one holds auto-merge.  The reported jobs and their logs live under
`.git/loft-pr-watch/<PR>/`; a new push starts a fresh list.

Exit codes: 10 a new red job (its log path printed) · 0 the run ended green · 2 the run ended
with only reds already reported · 124 the deadline passed · 1 a usage or `gh` failure.
"""

import argparse
import json
import os
import re
import subprocess
import sys
import time

FAILED = {"failure", "timed_out", "startup_failure", "action_required"}
# The lines of a job log that name what failed — a test, an error annotation, a clippy error.
FAIL_LINE = re.compile(
    r"FAIL \[ *[0-9.]+s\] \( *[0-9]+/[0-9]+\) \S+ \S+|test \S+ \.\.\. FAILED"
    r"|^##\[error\].*|^error(\[E\d+\])?: .*|^DEFECT .*|panicked at .*")


def gh(*args, check=True):
    """`gh` with its stdout; a failure is reported with gh's own words."""
    r = subprocess.run(["gh", *args], capture_output=True, text=True, encoding="utf-8")
    if check and r.returncode != 0:
        sys.exit(f"pr_watch: gh {' '.join(args[:3])} failed: {r.stderr.strip()[:300]}")
    return r.stdout


def gh_json(*args):
    out = gh("api", *args)
    return json.loads(out) if out.strip() else None


def repo():
    return gh("repo", "view", "--json", "nameWithOwner", "--jq", ".nameWithOwner").strip()


def head_sha(rp, pr):
    pull = gh_json(f"repos/{rp}/pulls/{pr}")
    return pull["head"]["sha"], pull["state"], pull.get("merged_at")


def check_runs(rp, sha):
    runs, page = [], 1
    while True:
        data = gh_json(f"repos/{rp}/commits/{sha}/check-runs?per_page=100&page={page}")
        batch = data.get("check_runs", []) if data else []
        runs += batch
        if len(batch) < 100:
            return runs
        page += 1


def required_checks(rp):
    """The check names a merge into main waits for: a ruleset's and the classic branch
    protection's, which a repository can use either or both of (empty when neither reads)."""
    names = set()
    r = subprocess.run(["gh", "api", f"repos/{rp}/rules/branches/main"],
                       capture_output=True, text=True, encoding="utf-8")
    if r.returncode == 0:
        for rule in json.loads(r.stdout or "[]"):
            if rule.get("type") == "required_status_checks":
                for c in rule.get("parameters", {}).get("required_status_checks", []):
                    names.add(c.get("context", ""))
    r = subprocess.run(["gh", "api", f"repos/{rp}/branches/main/protection/required_status_checks"],
                       capture_output=True, text=True, encoding="utf-8")
    if r.returncode == 0:
        names.update(json.loads(r.stdout or "{}").get("contexts", []))
    return names


def fetch_log(rp, job_id, logs_dir):
    """Save a job's log; answer its path and the lines that name what failed."""
    path = os.path.join(logs_dir, f"{job_id}.log")
    r = subprocess.run(["gh", "api", f"repos/{rp}/actions/jobs/{job_id}/logs"],
                       capture_output=True, text=True, encoding="utf-8", errors="replace")
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(r.stdout)
    seen, lines = set(), []
    for raw in r.stdout.splitlines():
        line = re.sub(r"^\S+Z ", "", raw)               # the runner's timestamp
        line = re.sub(r"\x1b\[[0-9;]*m", "", line).strip()
        m = FAIL_LINE.search(line)
        if m and m.group(0) not in seen:
            seen.add(m.group(0))
            lines.append(m.group(0)[:200])
    return path, lines[:12]


def nightly_reds(rp):
    """main's latest run of each scheduled workflow that ended red."""
    runs = gh_json(f"repos/{rp}/actions/runs?branch=main&event=schedule&per_page=100")
    latest = {}
    for run in (runs or {}).get("workflow_runs", []):
        name = run["name"]
        if name not in latest or run["created_at"] > latest[name]["created_at"]:
            latest[name] = run
    return [r for r in latest.values() if r.get("conclusion") in FAILED]


def report_nightly(rp):
    reds = nightly_reds(rp)
    if not reds:
        print("main's scheduled runs: none red")
        return
    print(f"main's scheduled runs: {len(reds)} red — fix them in this PR's red-step pass")
    for run in sorted(reds, key=lambda r: r["name"]):
        jobs = gh_json(f"repos/{rp}/actions/runs/{run['id']}/jobs?per_page=100") or {}
        bad = [j["name"] for j in jobs.get("jobs", []) if j.get("conclusion") in FAILED]
        print(f"  {run['name']} ({run['created_at'][:16]}Z, run {run['id']}): {', '.join(bad) or '?'}")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("pr", type=int)
    ap.add_argument("--interval", type=int, default=300, help="seconds between looks (300)")
    ap.add_argument("--timeout", type=int, default=7200, help="give up after this many seconds")
    ap.add_argument("--once", action="store_true", help="one look, then exit")
    ap.add_argument("--no-nightly", action="store_true", help="skip main's scheduled runs")
    ap.add_argument("--sha", help="watch this commit's checks instead of the PR's head "
                    "(a probe on the next branch; a merged PR's last run)")
    a = ap.parse_args()

    rp = repo()
    required = required_checks(rp)
    git_dir = subprocess.run(["git", "rev-parse", "--git-dir"], capture_output=True,
                             text=True, encoding="utf-8").stdout.strip() or ".git"
    base = os.path.join(git_dir, "loft-pr-watch", str(a.pr))
    deadline = time.time() + a.timeout
    while True:
        if a.sha:
            sha, state, merged = a.sha, "open", None
        else:
            sha, state, merged = head_sha(rp, a.pr)
        if merged or state != "open":
            print(f"PR #{a.pr} is {'merged' if merged else state} — nothing to watch")
            return 0
        logs_dir = os.path.join(base, sha[:12])
        os.makedirs(logs_dir, exist_ok=True)
        seen_file = os.path.join(logs_dir, "reported")
        first = not os.path.exists(seen_file)
        reported = set()
        if not first:
            with open(seen_file, encoding="utf-8") as fh:
                reported = set(fh.read().split())
        if first and not a.no_nightly:
            report_nightly(rp)
            open(seen_file, "a", encoding="utf-8").close()

        runs = check_runs(rp, sha)
        red = [r for r in runs if r.get("conclusion") in FAILED]
        pending = [r for r in runs if r.get("status") != "completed"]
        new = [r for r in red if str(r["id"]) not in reported]
        mark = lambda r: " [required]" if r["name"] in required else ""

        if a.once:
            print(f"PR #{a.pr} @ {sha[:9]}: {len(red)} red, {len(pending)} pending, "
                  f"{len(runs) - len(red) - len(pending)} done otherwise")
            for r in red:
                print(f"  RED  {r['name']}{mark(r)}  {r.get('html_url', '')}")
            return 0
        if new:
            with open(seen_file, "a", encoding="utf-8") as fh:
                for r in new:
                    fh.write(f"{r['id']}\n")
            print(f"PR #{a.pr} @ {sha[:9]}: {len(new)} new red job(s); "
                  f"{len(pending)} still running")
            for r in new:
                path, lines = fetch_log(rp, r["id"], logs_dir)
                print(f"RED  {r['name']}{mark(r)}\n     log: {path}")
                for line in lines:
                    print(f"       {line}")
            return 10
        if runs and not pending:
            if red:
                req = [r["name"] for r in red if r["name"] in required]
                print(f"PR #{a.pr} @ {sha[:9]}: run ended, {len(red)} red (all reported)"
                      + (f"; required red: {', '.join(req)} — holds auto-merge" if req
                         else "; no required check red — auto-merge can land it"))
                return 2
            print(f"PR #{a.pr} @ {sha[:9]}: run ended green")
            return 0
        if time.time() + a.interval > deadline:
            print(f"PR #{a.pr} @ {sha[:9]}: deadline — {len(pending)} job(s) still running")
            return 124
        time.sleep(a.interval)


if __name__ == "__main__":
    sys.exit(main())
