#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Read `Contract:` trailers off fix commits and apply the `contract:` labels.

The `contract:` axis (.github/LABELS.md) records whether closing a bug had to MOVE the
written standard.  It is set at FIX time, because that is the only moment the answer
exists — it is what the fix turned out to need, not something the filer could know.

Three places read the same trailer, in the order a fix meets them:

* `.githooks/commit-msg` asks for it while you are typing the message;
* the **push workflow** (`.github/workflows/apply-fixed-pending-merge.yml`) applies the
  label the moment the fix is pushed — the same event, and the same commit list, that
  labels the issue `fixed-pending-merge`, so the two verdicts a fix carries land
  together;
* a hand run here is the BACKSTOP for whatever gets through both: `--report` names every
  `Fixes #N` commit on the branch whose trailer is missing, so a miss is recoverable
  later instead of becoming a permanently unjudged issue.

That backstop is the point — `Fixes #N` itself was in CLAUDE.md, ISSUE_TRACKING.md and two
skills before a hook existed, and fixes still shipped without it; prose alone did not make
it stick, and neither will this.

Usage:
    scripts/contract_labels.py                     # report what the branch says (dry run)
    scripts/contract_labels.py --apply             # label the issues on GitHub
    scripts/contract_labels.py --base origin/main  # commits since this ref (default)
    scripts/contract_labels.py --report            # ONLY the unjudged fixes, for a nudge
    scripts/contract_labels.py --event "$GITHUB_EVENT_PATH"   # a push payload, not git
    scripts/contract_labels.py --self-test         # the parse, against a fixed corpus
"""
import argparse
import json
import os
import re
import subprocess
import sys

TRAILER = re.compile(r"^[ \t]*Contract:[ \t]*(settled|strained)\b", re.I | re.M)
CLOSES = re.compile(r"\b(?:fixes|closes|resolves)\s+#(\d+)", re.I)

# GitHub annotations are only meaningful inside a runner; elsewhere they are noise.
ACTIONS = bool(os.environ.get("GITHUB_ACTIONS"))


def git_commits(base):
    """Every commit on HEAD not in `base`, as (sha, subject, body)."""
    sep = "\x1e"
    out = subprocess.run(
        ["git", "log", f"{base}..HEAD", "--no-merges", f"--format=%H%x1f%s%x1f%B{sep}"],
        capture_output=True, text=True, check=False)
    if out.returncode != 0:
        sys.exit(f"git log failed: {out.stderr.strip()}")
    for rec in out.stdout.split(sep):
        rec = rec.strip("\n")
        if not rec:
            continue
        sha, subject, body = rec.split("\x1f", 2)
        yield sha, subject, body


def event_commits(path):
    """The same, read off a GitHub push-event payload instead of a repository.

    The workflow runs beside the `fixed-pending-merge` job and must judge exactly the
    commits that job labels, so it reads the SAME `.commits` array rather than a `git
    log` range — a rebase or a force-push cannot make the two disagree.  GitHub caps
    that array at 20 commits; a longer push is what the hand run above is for.
    """
    with open(path, encoding="utf-8") as fh:
        payload = json.load(fh)
    for commit in payload.get("commits") or []:
        message = commit.get("message") or ""
        yield commit.get("id", ""), message.split("\n", 1)[0], message


def judgements(records):
    """(issue -> verdict, and the unjudged fixes) read off a sequence of commits.

    A later commit wins: a follow-up that revises the call is the newer knowledge, and
    the alternative — first-writer-wins — would pin a verdict the author already
    corrected.
    """
    verdict, unjudged = {}, []
    for sha, subject, body in records:
        issues = sorted({int(n) for n in CLOSES.findall(body)})
        if not issues:
            continue
        m = TRAILER.search(body)
        if m:
            for n in issues:
                verdict[n] = m.group(1).lower()
        else:
            unjudged.append((sha[:8], subject, issues))
    return verdict, unjudged


def apply_labels(verdict):
    """Write the labels via `gh`, and say so when a write does not land.

    A label that silently failed to apply is indistinguishable from a fix nobody judged,
    which is the one reading this axis must never get wrong — so a failure is reported,
    not swallowed.  It stays a warning: the hand run is the recovery path, and a
    transient API error is not a reason to redden a push.
    """
    failed = 0
    for n, v in sorted(verdict.items()):
        # The red-steps dashboard (`ci-advisory`) is no defect a fix settles: never labelled.
        names = subprocess.run(["gh", "issue", "view", str(n), "--json", "labels",
                                "-q", "[.labels[].name] | join(\",\")"],
                               capture_output=True, text=True, check=False).stdout
        if "ci-advisory" in names.strip().split(","):
            print(f"  skipped #{n}: the ci-advisory dashboard")
            continue
        other = "strained" if v == "settled" else "settled"
        out = subprocess.run(["gh", "issue", "edit", str(n),
                              "--add-label", f"contract:{v}",
                              "--remove-label", f"contract:{other}"],
                             capture_output=True, text=True, check=False)
        if out.returncode == 0:
            print(f"  labelled #{n} contract:{v}")
            continue
        failed += 1
        why = (out.stderr or out.stdout).strip().splitlines()
        why = why[-1] if why else f"gh exited {out.returncode}"
        print(f"{'::warning::' if ACTIONS else '  '}#{n} contract:{v} NOT applied — {why}")
    return failed


SELF_TEST = [
    # (name, commits as (sha, message), expected verdict, expected unjudged issue lists)
    ("trailer after the Fixes line",
     [("a1", "fix it\n\nFixes #12\nContract: settled — the rule already said so\n")],
     {12: "settled"}, []),
    ("no trailer at all is UNJUDGED, never settled",
     [("b2", "fix it\n\nCloses #13\n")], {}, [[13]]),
    ("a later commit revises the call",
     [("c3", "first\n\nFixes #14\nContract: settled — looked settled\n"),
      ("c4", "second\n\nFixes #14\nContract: strained — it needed a rule\n")],
     {14: "strained"}, []),
    ("one commit, several issues, one verdict",
     [("d5", "two at once\n\nFixes #20\nResolves #21\nContract: strained — one call\n")],
     {20: "strained", 21: "strained"}, []),
    ("a bare mention is not a fix and owes no verdict",
     [("e6", "mention (#22) in passing\n")], {}, []),
    ("case and leading space in the trailer",
     [("f7", "x\n\nfixes #23\n   contract:   STRAINED — shouty\n")], {23: "strained"}, []),
    ("`Contracts:` is a different word",
     [("g8", "x\n\nFixes #24\nContracts: settled\n")], {}, [[24]]),
    ("a trailer with no fix labels nothing",
     [("h9", "x\n\nContract: settled — nothing to label\n")], {}, []),
]


def self_test():
    """Hold the parse to a fixed corpus.

    Every way this can be wrong is SILENT — a regex that stops matching applies no label,
    which is exactly what an unjudged fix looks like, and the push workflow only runs on a
    push, so nothing else would notice until a month-later count read convergence it never
    measured.  Same reasoning as `tools/label_guard_selftest.mjs`.
    """
    bad = 0
    for name, commits, want_verdict, want_unjudged in SELF_TEST:
        records = [(sha, msg.split("\n", 1)[0], msg) for sha, msg in commits]
        verdict, unjudged = judgements(records)
        got_unjudged = [issues for _, _, issues in unjudged]
        if verdict != want_verdict or got_unjudged != want_unjudged:
            bad += 1
            print(f"  FAIL  {name}")
            print(f"        verdict  want {want_verdict} got {verdict}")
            print(f"        unjudged want {want_unjudged} got {got_unjudged}")
    total = len(SELF_TEST)
    print(f"  contract_labels self-test: {total - bad}/{total} cases pass")
    return 1 if bad else 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", default="origin/main")
    ap.add_argument("--event", help="a GitHub push-event payload to read instead of git")
    ap.add_argument("--apply", action="store_true", help="write the labels via gh")
    ap.add_argument("--report", action="store_true", help="only list unjudged fixes")
    ap.add_argument("--self-test", action="store_true", help="check the parse and exit")
    a = ap.parse_args()

    if a.self_test:
        return self_test()

    source = event_commits(a.event) if a.event else git_commits(a.base)
    verdict, unjudged = judgements(source)
    where = "this push" if a.event else f"{a.base}..HEAD"

    if not verdict and not unjudged:
        # Nothing here fixes an issue, so there is nothing to judge and nothing
        # to act on.  Say that in one line rather than reporting an empty table.
        if not a.report:
            print(f"  no `Fixes #N` in {where} — nothing to judge")
        return 0

    if not a.report:
        print(f"=== `Contract:` trailers on {where} ===")
        if not verdict:
            print("  (none)")
        for n, v in sorted(verdict.items()):
            print(f"  #{n:<6} contract:{v}")

    if unjudged:
        print(f"\n=== UNJUDGED — `Fixes #N` with no `Contract:` trailer ({len(unjudged)}) ===")
        print("  These count as unjudged, never as settled.  Amend the commit, or set the")
        print("  label on the issue by hand.")
        for sha, subject, issues in unjudged:
            refs = " ".join("#" + str(i) for i in issues)
            print(f"  {sha}  {refs:<24} {subject[:56]}")
            if ACTIONS:
                print(f"::warning::commit {sha} fixes {refs} with no `Contract:` trailer — "
                      "the issue stays UNJUDGED, which the monthly ratio counts as neither "
                      "settled nor strained.  Add `Contract: settled|strained — <why>` to "
                      "the commit, or set the label on the issue by hand.")
    elif not a.report:
        print(f"\n  every fix in {where} carries a verdict")

    if a.apply:
        apply_labels(verdict)
    elif verdict and not a.report:
        print("\n  (dry run — re-run with --apply to write these labels)")

    return 1 if unjudged and a.report else 0


if __name__ == "__main__":
    sys.exit(main())
