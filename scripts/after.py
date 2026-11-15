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
    scripts/after.py fix --pr --since <PR head>   # before a push to an open PR (scripts/pr_push.py)

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
`--pr` adds the checks only the PR's run would otherwise make — the front-end allocation pins
and the platform census — because there a miss costs a whole run, and on green it records
`.git/loft-after/last-pr` (HEAD and the base it covered), which `scripts/pr_push.py` reads.
A test this box always fails for its own reasons (a stale fixture) is named one per line in
`.git/loft-after/known-red`: kept out of the run, and listed, so a NEW red stands out.

A rerun repeats only what it must.  A step that passed on a commit is skipped when every path
changed since then is one it never reads (`NEVER_READS`: the cargo and lint steps, which
read no `.md` and no script); every other step always runs.  The tests cannot be skipped that
way — the subject map sends a diff it cannot place to the whole curated set — so the tests a
run left red run FIRST next time, and while one still fails the full set is not run: a fix
that did not work shows in seconds.  `--fresh` runs everything.
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
# The checkout's own git directory: `.git` is a FILE in a worktree, naming the real one.
STATE = Path(subprocess.run(["git", "rev-parse", "--absolute-git-dir"], cwd=ROOT,
                            capture_output=True, text=True, check=True).stdout.strip()) / "loft-after"
# The PROBE profile's binary (`[profile.probe]`, opt-level 0, its own target directory): an
# incremental rebuild after an edit is seconds on an idle box where the dev profile's was
# minutes (@PLN194).  The ratchets, guards and probes run on it; clippy and `cargo check`
# keep their own artefacts.
PROBE = ["--profile", "probe", "--target-dir", "target/probe"]
BIN = ROOT / "target" / "probe" / "probe" / "loft"


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
        self.cached = None
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


def known_red():
    """The tests this box fails for its own reasons, one name per line (machine-local)."""
    f = STATE / "known-red"
    if not f.exists():
        return []
    return [l.strip() for l in f.read_text(encoding="utf-8").splitlines()
            if l.strip() and not l.startswith("#")]


def without_known_red(expr):
    names = known_red()
    if not names:
        return expr
    return f"({expr}) - ({' | '.join(f'test(={n})' for n in names)})"


def suite_note():
    """Say when the language suite the checks read is not the commit the tree pins: a guard
    that lives only in the newer suite is then invisible to the receipts and the hygiene."""
    pin = (ROOT / "tests" / "loft-suite.pin").read_text(encoding="utf-8").strip()
    suite = Path(os.environ.get("LOFT_SUITE_DIR") or ROOT.parent / "loft-suite")
    if not suite.is_absolute():
        suite = ROOT / suite
    head = subprocess.run(["git", "-C", str(suite), "rev-parse", "HEAD"],
                          capture_output=True, text=True).stdout.strip()
    if head != pin:
        print(f"  note  the suite at {suite} is {head[:9] or 'missing'}, the pin is {pin[:9]}: "
              "point LOFT_SUITE_DIR at a checkout of the pin, or suite-backed checks read the wrong cells")


# The paths a step provably never reads.  Read off what each one compiles or runs: `build.rs`
# reads `src`, `default`, `loft-ffi/src` and the environment; the Rust sources include
# `doc/*.js` (so `doc/` is NOT here) and no `.md` or script; `ownership_drop_gate` and every
# python step read scripts, so they are not here at all.  A step missing from this table
# always runs.
_NO_DOCS_OR_SCRIPTS = r"\.md$|^scripts/"
NEVER_READS = {
    "fmt": r"^(?!.*\.rs$)(?!(\.)?rustfmt\.toml$)",
    "build": _NO_DOCS_OR_SCRIPTS,
    "cargo check": _NO_DOCS_OR_SCRIPTS,
    "clippy": _NO_DOCS_OR_SCRIPTS,
    "clippy tests": _NO_DOCS_OR_SCRIPTS,
    "allocation pins": _NO_DOCS_OR_SCRIPTS,
}
# What a step's result also depends on besides the tree.
CACHE_ENV = ("LOFT_SUITE_DIR", "LOFT_NATIVE_BACKEND", "RUSTFLAGS", "CARGO_TARGET_DIR")
CACHE = STATE / "steps.json"
REDS = STATE / "previous-reds.json"


def load_json(f):
    try:
        return json.loads(f.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return {}


def cache_key(step):
    return {"cmd": step.cmd, "env": {k: os.environ.get(k, "") for k in CACHE_ENV}}


def is_ancestor(a, b):
    return subprocess.run(["git", "merge-base", "--is-ancestor", a, b], cwd=ROOT,
                          capture_output=True).returncode == 0


def passed_at(step, cache):
    """The commit this step passed on, when nothing it reads changed since; else None."""
    rx = NEVER_READS.get(step.name)
    rec = cache.get(step.name)
    if not rx or not rec or rec.get("key") != cache_key(step) or not is_ancestor(rec["sha"], "HEAD"):
        return None
    return rec["sha"] if all(re.search(rx, p) for p in changed_paths(rec["sha"])) else None


def record_passes(steps):
    """Remember each passed NEVER_READS step at HEAD — only when the tree it ran on IS HEAD,
    up to paths that step does not read (an untracked source it compiled would not be)."""
    if git("status", "--porcelain", "--untracked-files=no").strip():
        return
    head = git("rev-parse", "HEAD").strip()
    untracked = git("ls-files", "--others", "--exclude-standard").split()
    cache = load_json(CACHE)
    for s in steps:
        rx = NEVER_READS.get(s.name)
        if s.ok and not s.cached and rx and all(re.search(rx, p) for p in untracked):
            cache[s.name] = {"sha": head, "key": cache_key(s)}
    CACHE.write_text(json.dumps(cache, indent=1), encoding="utf-8")


FAIL_LINE = re.compile(r"FAIL \[ *[0-9.]+s\] \( *[0-9]+/[0-9]+\) (\S+) (\S+)")


def record_reds(step):
    """Keep the tests a run left red, so the next run starts with them."""
    reds = sorted({m.groups() for m in FAIL_LINE.finditer(step.out)})
    if step.ok or not reds:
        REDS.unlink(missing_ok=True)
        return
    REDS.write_text(json.dumps({"tests": reds}), encoding="utf-8")


def reds_step():
    reds = load_json(REDS).get("tests") or []
    if not reds:
        return None
    expr = " | ".join(f"(binary_id(={b}) & test(={t}))" for b, t in reds)
    return Step("previous reds",
                f"LOFT_NATIVE_BACKEND=${{LOFT_NATIVE_BACKEND-c}} cargo nextest run --no-fail-fast --status-level fail -E \"{expr}\"",
                ", ".join(t for _b, t in reds[:2]) + (f" +{len(reds) - 2}" if len(reds) > 2 else ""), "B")


def plan(mode, paths, base, want_tests, pr=False, fresh=False):
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
    code = first_match(paths, r"^src/|^default/")
    if code:
        # A receipt whose patch the code moved under is refreshed on the PR's run, and one
        # that no longer merges fails it; that merge is seconds here.
        steps.append(Step("receipts reach", f"python3 scripts/receipts_ci.py --check --base {base}", code, "A"))
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
        steps.append(Step("build", "cargo build -q --bin loft " + " ".join(PROBE), rs or stdlib, "B"))
        steps.append(Step("cargo check", "cargo check -q --all-targets", rs or stdlib, "B"))
    if src:
        steps.append(Step("clippy", "cargo clippy -q --bin loft -- -D warnings", src, "B"))
    if tests_rs:
        steps.append(Step("clippy tests", "cargo clippy -q --tests -- -D warnings", tests_rs, "B"))
    ratchets, runner = artefact_steps(paths)
    steps.extend(ratchets)
    if pr and (src or stdlib):
        # Advisory on the PR (never failing a test leg), so a miss shows only as a red there.
        # The dev profile reads the `linux-debug` row, which the runner pins beside the
        # release one; no release build here.
        steps.append(Step("allocation pins", "cargo test -q --test frontend_counts -- --ignored",
                          src or stdlib, "B"))
    census = first_match(paths, r"\.(py|sh)$|^scripts/windows_smoke\.tsv$|^\.github/")
    # With the tests running, the census rides in their one nextest run (a step of its own
    # paid the test-profile compile again for eight tests that take under a second).
    census_in_tests = pr and bool(census or added) and want_tests
    if pr and (census or added) and not want_tests:
        steps.append(Step("platform census",
                          "cargo nextest run --no-fail-fast --status-level fail -E 'test(/^platform_census::/)'",
                          census or ", ".join(added[:2]), "B"))
    if guards:
        # `--tests` takes ONE target per run, so the guards run one after another.
        files = " ".join(f"'{g}'" for g in guards)
        for backend in ("interpret", "native"):
            steps.append(Step(f"guards {backend}",
                              f"for g in {files}; do LOFT_TIMEOUT=120 LOFT_LEAK_CHECK=1 {BIN} --{backend} --tests \"$g\" || exit 1; done",
                              ", ".join(guards[:2]) + (f" +{len(guards) - 2}" if len(guards) > 2 else ""),
                              "B", needs="bin"))
    if want_tests:
        census_expr = "test(/^platform_census::/)"
        def tests_expr(e):
            return without_known_red(f"({e}) | {census_expr}" if census_in_tests else e)
        if not fresh:
            first = reds_step()
            if first:
                steps.append(first)
        ok, filt, _ = sh(f"bash -c 'source scripts/test_subjects.sh; changed_filter {base}'")
        # The expression is the first line; `changed_filter` reports the subjects it chose
        # on a second line, which nextest would read as unparsed input.
        filt = filt.strip().splitlines()[0] if filt.strip() else ""
        if ok and filt.strip():
            steps.append(Step("tests", f"LOFT_NATIVE_BACKEND=${{LOFT_NATIVE_BACKEND-c}} cargo nextest run --no-fail-fast --status-level fail -E \"{tests_expr(filt.strip())}\"",
                              f"the subjects the diff touches ({filt.strip()[:60]}…)", "B"))
        else:
            expr = tests_expr("$(bash -c 'source scripts/test_subjects.sh; curated_filter')")
            steps.append(Step("tests", "LOFT_NATIVE_BACKEND=${LOFT_NATIVE_BACKEND-c} cargo nextest run --no-fail-fast --status-level fail "
                              f"-E \"{expr}\"",
                              "the diff touches what every binary depends on — the curated set", "B"))
    return steps, runner


# ── running and reporting ───────────────────────────────────────────────────────────────


def run_step(step):
    env = {"LOFT_BIN": str(BIN)} if step.needs == "bin" else None
    step.ok, step.out, step.secs = sh(step.cmd, env)
    return step


def line(step):
    if step.cached:
        return f"  skip  {step.name:18}     0s   passed at {step.cached[:9]}; nothing it reads changed since"
    mark = "ok  " if step.ok else "FAIL"
    return f"  {mark}  {step.name:18} {step.secs:6.0f}s   {step.why}"


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("mode", choices=["join", "fix"])
    ap.add_argument("--since", help="the ref the changed set is measured from")
    ap.add_argument("--dry-run", action="store_true", help="print the plan and stop")
    ap.add_argument("--tests", action="store_true", help="run the test binaries (a join's default is off)")
    ap.add_argument("--no-tests", action="store_true", help="skip the test binaries (a fix's default is on)")
    ap.add_argument("--pr", action="store_true",
                    help="before a push to an open PR: add the PR-only checks, record last-pr on green")
    ap.add_argument("--fresh", action="store_true",
                    help="run every step: no step skipped as passed, no previous reds first")
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
    steps, runner = plan(args.mode, paths, base, want_tests, pr=args.pr, fresh=args.fresh)
    cache = {} if args.fresh else load_json(CACHE)
    for s in steps:
        s.cached = passed_at(s, cache)
        if s.cached:
            s.ok = True
    suite_note()
    if known_red() and want_tests:
        print(f"  note  kept out as this box's known reds ({STATE / 'known-red'}): {', '.join(known_red())}")
    if args.dry_run:
        for s in steps:
            if s.cached:
                print(f"  skip  {s.name:18} {s.phase}   passed at {s.cached[:9]}; nothing it reads changed since")
                continue
            print(f"  plan  {s.name:18} {s.phase}   {s.why}\n        {s.cmd[:110]}")
        for name, why in runner:
            print(f"  runner {name:17}     stale ({why}) — scripts/derive-ci.sh")
        return 0

    others = subprocess.run(["scripts/pgrep_others.sh", "(^|/)cargo (build|test|clippy|check|nextest)", "--count"],
                            cwd=ROOT, capture_output=True, text=True).stdout.strip()
    if others not in ("", "0"):
        print(f"another cargo is running on this box ({others}); one at a time — a build beside "
              "another took minutes where it takes seconds alone (@PLN194).  Wait, or stop it.")
        return 2
    t0 = time.monotonic()
    a_steps = [s for s in steps if s.phase == "A"]
    b_steps = [s for s in steps if s.phase == "B"]
    print(f"phase A — {len(a_steps)} check(s) beside the build; phase B — {len(b_steps)} step(s), one at a time")
    with ThreadPoolExecutor(max_workers=4) as pool:
        futures = [pool.submit(run_step, s) for s in a_steps if not s.cached]
        reds_left = False
        for s in b_steps:
            if s.cached:
                print(line(s))
                continue
            if s.name == "tests" and reds_left:
                # The full set waits until the previous reds are green: a fix that did not
                # work shows in seconds, and nine minutes of other tests add nothing to it.
                s.ok, s.out = False, "not run: the previous reds still fail (`--fresh` runs everything)"
                print(line(s))
                continue
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
            if s.name == "previous reds" and not s.ok:
                reds_left = True
            if s.name == "tests":
                record_reds(s)
        for f in futures:
            print(line(f.result()))
        for s in a_steps:
            if s.cached:
                print(line(s))
    record_passes(steps)
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
    if args.pr:
        dirty = git("status", "--porcelain", "--untracked-files=no").strip()
        if dirty:
            print("last-pr not recorded: the tree has uncommitted changes, so HEAD is not what was checked")
        else:
            record = f"{git('rev-parse', 'HEAD').strip()} {git('rev-parse', base).strip()} {stamp}\n"
            (STATE / "last-pr").write_text(record, encoding="utf-8")
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
