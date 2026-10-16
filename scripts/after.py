#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""What to run after a join or after a fix — only the steps the changed files call for, timed.

    scripts/after.py join                 # after a join: the diff the join added to this tree
    scripts/after.py fix                  # after a fix: the diff since the last green run here
    scripts/after.py fix --since HEAD~2   # …or since a ref of your choice
    scripts/after.py fix --dry-run        # the plan: each step with its reason, nothing run
    scripts/after.py fix --no-tests       # everything but the test binaries
    scripts/after.py join --tests         # a join runs no test binaries unless asked
    scripts/after.py fix --dispatch       # …then push-check and dispatch the runner's workflows

Why
---
Every step a change can need already exists — the ratchets, the lint gates, clippy, the test
subjects, the runner's derivations — and running all of them after every change is what put a
join past an hour: release builds here of what the PR gate builds again on the exact sha.  The
changed files decide which steps are due (`scripts/test_subjects.sh` maps a diff to its test
subjects, `scripts/derived_artefacts.json` names each artefact's inputs), the dev binary serves
every local check, and what needs a release build or a runner is REPORTED as the dispatch it
needs rather than run here.  Each step prints its reason and its seconds, so the next cut is
visible.

How
---
The changed set is `git diff --name-only <base>` plus the untracked files.  `join` takes the
head the join plan recorded (`.git/loft-join/plan.json`), else `origin/main`; `fix` takes the
sha of the last green run (`.git/loft-after/last`), else the branch's upstream, else `HEAD~1`.
The seconds-long checks that need no binary run concurrently while the dev binary builds;
the build-dependent steps run one at a time (one cargo at a time on this box).  A red step's
output is kept under `.git/loft-after/` and its tail printed.  The marker advances on green.
"""
import argparse
import datetime
import json
import os
import re
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
STATE = ROOT / ".git" / "loft-after"
BIN = ROOT / "target" / "debug" / "loft"


def git(*args):
    return subprocess.run(["git", *args], cwd=ROOT, capture_output=True, text=True).stdout


def sh(cmd, env=None):
    """Run a shell command; (ok, output, seconds)."""
    t0 = time.monotonic()
    e = dict(os.environ)
    e.update(env or {})
    r = subprocess.run(cmd, shell=True, cwd=ROOT, capture_output=True, text=True, env=e)
    return r.returncode == 0, r.stdout + r.stderr, time.monotonic() - t0


# ── the changed set ─────────────────────────────────────────────────────────────────────


def base_for(mode, since):
    if since:
        return since, f"--since {since}"
    if mode == "join":
        plan = ROOT / ".git" / "loft-join" / "plan.json"
        if plan.exists():
            head = json.loads(plan.read_text(encoding="utf-8"))["head"]
            if subprocess.run(["git", "merge-base", "--is-ancestor", head, "HEAD"],
                              cwd=ROOT).returncode == 0:
                return head, "the head the join plan recorded"
        return "origin/main", "origin/main (no join plan)"
    last = STATE / "last"
    if last.exists():
        sha = last.read_text(encoding="utf-8").split()[0]
        if subprocess.run(["git", "merge-base", "--is-ancestor", sha, "HEAD"],
                          cwd=ROOT).returncode == 0:
            return sha, "the last green run here"
    up = git("rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}").strip()
    if up and not up.startswith("fatal"):
        return up, "the branch's upstream (no green run recorded)"
    return "HEAD~1", "HEAD~1 (no upstream, no green run recorded)"


def changed_paths(base):
    paths = set(git("diff", "--name-only", base, "--").split())
    paths |= set(git("ls-files", "--others", "--exclude-standard").split())
    return sorted(paths)


def added_paths(base):
    out = git("diff", "--name-status", base, "--")
    added = {l.split("\t")[-1] for l in out.splitlines() if l.startswith("A")}
    added |= set(git("ls-files", "--others", "--exclude-standard").split())
    return sorted(added)


# ── the steps ───────────────────────────────────────────────────────────────────────────


class Step:
    def __init__(self, name, cmd, why, phase, needs=None):
        self.name, self.cmd, self.why, self.phase, self.needs = name, cmd, why, phase, needs
        self.ok = None
        self.secs = 0.0
        self.out = ""


def first_match(paths, pattern, limit=2):
    rx = re.compile(pattern)
    hits = [p for p in paths if rx.search(p)]
    if not hits:
        return None
    more = f" +{len(hits) - limit}" if len(hits) > limit else ""
    return ", ".join(hits[:limit]) + more


def artefact_steps(paths):
    """The ratchets whose inputs moved (their `check`), and the runner's artefacts to report."""
    reg = json.loads((ROOT / "scripts" / "derived_artefacts.json").read_text(encoding="utf-8"))
    ratchets, runner = [], []
    for art in reg["artefacts"]:
        inputs = art.get("inputs") or []
        hit = [p for p in paths if any(p.startswith(i) for i in inputs)]
        if not hit:
            continue
        why = ", ".join(hit[:2]) + (f" +{len(hit) - 2}" if len(hit) > 2 else "")
        if art.get("ci"):
            runner.append((art["name"], why))
        elif art.get("check") and art.get("regen"):
            # A ratchet with no writer is pinned elsewhere (the frontend counts, on CI).
            ratchets.append(Step(art["name"], art["check"], why, "B", needs="bin"))
    return ratchets, runner


def plan(mode, paths, base, want_tests):
    steps = []
    rs = first_match(paths, r"\.rs$|^Cargo\.(toml|lock)$")
    src = first_match(paths, r"^src/")
    tests_rs = first_match(paths, r"^tests/[^/]+\.rs$")
    stdlib = first_match(paths, r"^default/.*\.loft$")
    md = first_match(paths, r"\.md$")
    loft = first_match(paths, r"\.loft$")
    formal = first_match(paths, r"^doc/claude/formal/")
    guards = [p for p in paths if re.match(r"^tests/scripts/.*\.loft$", p) and (ROOT / p).exists()]
    receipts = first_match(paths, r"^tests/falsified/|^tests/scripts/")
    added = added_paths(base)
    # Phase A — no binary needed; these run concurrently while the dev binary builds.
    if rs:
        steps.append(Step("fmt", "cargo fmt -- --check", rs, "A"))
    if rs or loft or formal:
        steps.append(Step("rule tags", "python3 scripts/rule_tags.py check", rs or loft or formal, "A"))
    if src:
        steps.append(Step("o_proxy", "python3 scripts/o_proxy_check.py", src, "A"))
    if receipts:
        steps.append(Step("receipts review", "python3 scripts/falsify-review.py --check", receipts, "A"))
    if added:
        steps.append(Step("feature coverage", "scripts/feature_coverage.sh --check",
                          ", ".join(added[:2]) + (f" +{len(added) - 2}" if len(added) > 2 else ""), "A"))
    if md or rs or loft:
        steps.append(Step("doc gate",
                          f"python3 scripts/doc_lint.py --gate --since \"$(git merge-base origin/main HEAD)\" --changed",
                          md or rs or loft, "A"))
    if md or stdlib:
        steps.append(Step("doc drift", "scripts/check_doc_drift.sh -q", md or stdlib, "A"))
    if stdlib or src:
        steps.append(Step("compiled stdlib", "python3 scripts/compiled_stdlib_fresh.py", stdlib or src, "A"))
    # Phase B — one cargo at a time.
    if rs or stdlib:
        steps.append(Step("build", "cargo build -q --bin loft", rs or stdlib, "B"))
        steps.append(Step("cargo check", "cargo check -q --all-targets", rs or stdlib, "B"))
    if src:
        steps.append(Step("clippy", "cargo clippy -q --bin loft -- -D warnings", src, "B"))
    if tests_rs:
        steps.append(Step("clippy tests", "cargo clippy -q --tests -- -D warnings", tests_rs, "B"))
    ratchets, runner = artefact_steps(paths)
    steps.extend(ratchets)
    if guards:
        files = " ".join(f"'{g}'" for g in guards)
        for backend in ("interpret", "native"):
            steps.append(Step(f"guards {backend}",
                              f"LOFT_TIMEOUT=120 LOFT_LEAK_CHECK=1 {BIN} --{backend} --tests {files}",
                              ", ".join(guards[:2]) + (f" +{len(guards) - 2}" if len(guards) > 2 else ""),
                              "B", needs="bin"))
    if want_tests:
        ok, filt, _ = sh(f"bash -c 'source scripts/test_subjects.sh; changed_filter {base}'")
        if ok and filt.strip():
            steps.append(Step("tests", f"cargo nextest run --no-fail-fast --status-level fail -E \"{filt.strip()}\"",
                              f"the subjects the diff touches ({filt.strip()[:60]}…)", "B"))
        else:
            steps.append(Step("tests", "cargo nextest run --no-fail-fast --status-level fail "
                              "-E \"$(bash -c 'source scripts/test_subjects.sh; curated_filter')\"",
                              "the diff touches what every binary depends on — the curated set", "B"))
    return steps, runner


# ── running and reporting ───────────────────────────────────────────────────────────────


def run_step(step):
    env = {"LOFT_BIN": str(BIN)} if step.needs == "bin" else None
    step.ok, step.out, step.secs = sh(step.cmd, env)
    return step


def line(step):
    mark = "ok  " if step.ok else "FAIL"
    return f"  {mark}  {step.name:18} {step.secs:6.0f}s   {step.why}"


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("mode", choices=["join", "fix"])
    ap.add_argument("--since", help="the ref the changed set is measured from")
    ap.add_argument("--dry-run", action="store_true", help="print the plan and stop")
    ap.add_argument("--tests", action="store_true", help="run the test binaries (a join's default is off)")
    ap.add_argument("--no-tests", action="store_true", help="skip the test binaries (a fix's default is on)")
    ap.add_argument("--dispatch", action="store_true",
                    help="after a green run: check the branch is pushed, dispatch the runner's workflows")
    args = ap.parse_args()
    sys.stdout.reconfigure(line_buffering=True)
    STATE.mkdir(parents=True, exist_ok=True)

    base, base_why = base_for(args.mode, args.since)
    paths = changed_paths(base)
    want_tests = args.tests if args.mode == "join" else not args.no_tests
    shown = base[:12] if re.fullmatch(r"[0-9a-f]{12,40}", base) else base
    print(f"after {args.mode}: {len(paths)} changed path(s) since {shown} — {base_why}")
    if not paths:
        print("nothing changed — nothing to run")
        return 0
    steps, runner = plan(args.mode, paths, base, want_tests)
    if args.dry_run:
        for s in steps:
            print(f"  plan  {s.name:18} {s.phase}   {s.why}\n        {s.cmd[:110]}")
        for name, why in runner:
            print(f"  runner {name:17}     stale ({why}) — scripts/derive-ci.sh")
        return 0

    t0 = time.monotonic()
    a_steps = [s for s in steps if s.phase == "A"]
    b_steps = [s for s in steps if s.phase == "B"]
    print(f"phase A — {len(a_steps)} check(s) beside the build; phase B — {len(b_steps)} step(s), one at a time")
    with ThreadPoolExecutor(max_workers=4) as pool:
        futures = [pool.submit(run_step, s) for s in a_steps]
        for s in b_steps:
            if s.name == "tests":
                # The native harness links the release rlib (`make check-rlib`); a stale one
                # fails those tests with a message shaped like a code defect.
                ok, out, _ = sh("make -s check-rlib")
                if not ok:
                    print("  note  check-rlib: " + out.strip().splitlines()[-1][:100])
            if s.needs == "bin" and not BIN.exists():
                s.ok, s.out = False, f"no binary at {BIN} — nothing changed that builds it; run `cargo build --bin loft`"
                print(line(s))
                continue
            run_step(s)
            print(line(s))
        for f in futures:
            print(line(f.result()))
    total = time.monotonic() - t0
    failed = [s for s in steps if not s.ok]
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    log = STATE / f"{args.mode}-{stamp}.log"
    report = "\n\n".join(f"== {s.name} ({'ok' if s.ok else 'FAIL'}, {s.secs:.0f}s)\n$ {s.cmd}\n{s.out}" for s in steps)
    log.write_text(report, encoding="utf-8")
    for s in failed:
        print(f"\n-- {s.name} failed; its last lines ({log} has all of it):")
        print("\n".join("   " + l for l in s.out.strip().splitlines()[-15:]))
    print(f"\n{len(steps) - len(failed)}/{len(steps)} ok in {total:.0f}s wall")
    for name, why in runner:
        print(f"  runner-owned   {name:18} stale ({why}) — regenerate with `scripts/derive-ci.sh`")
    branch = git("rev-parse", "--abbrev-ref", "HEAD").strip()
    src_moved = any(p.startswith(("src/", "default/")) for p in paths)
    print("next:" if not failed else "next, once green:")
    if runner:
        print("  scripts/derive-ci.sh                      # the runner regenerates the stale artefacts")
    if src_moved:
        print("  scripts/derive-ci.sh --workflow receipts.yml   # the patch receipts the code moved under")
    print(f"  gh workflow run ci.yml --ref {branch} -f os=ubuntu-latest   # the gate, on the exact sha")
    if failed:
        return 1
    (STATE / "last").write_text(git("rev-parse", "HEAD").strip() + f" {stamp}\n", encoding="utf-8")
    if args.dispatch:
        local, remote = git("rev-parse", "HEAD").strip(), git("rev-parse", f"origin/{branch}").strip()
        if local != remote or git("status", "--porcelain", "--untracked-files=no").strip():
            print("dispatch: the branch is not pushed as it stands here — commit and push first")
            return 1
        for wf in (["derive.yml"] if runner else []) + (["receipts.yml"] if src_moved else []) + ["ci.yml"]:
            extra = ["-f", "os=ubuntu-latest"] if wf == "ci.yml" else []
            r = subprocess.run(["gh", "workflow", "run", wf, "--ref", branch, *extra], cwd=ROOT)
            print(f"  dispatched {wf}" if r.returncode == 0 else f"  could not dispatch {wf}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
