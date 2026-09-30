#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Close every issue a merged PR fixes — the PR body AND each of its commits.

GitHub's own closing reads one issue per keyword, and on a squash merge only what the
merge commit's message carries: PR #1744 said `Fixes #1570, #1731, … #1738` in its body and
closed #1570 alone, and the nine commit trailers inside it (`Fixes #1734`, …) closed
nothing, so nine fixed issues stayed open, `fixed-pending-merge` on each, until closed by
hand.  This reads what a fix says it closes the way a person does:

  * a keyword — fix/fixes/fixed, close/closes/closed, resolve/resolves/resolved — then
    `#N`, and every further `#N` joined to it by commas or `and`;
  * in the PR body and in every commit between the PR's base and its head.

`Refs #N` and a bare `#N` close nothing.  An issue already closed, or a number that is a
pull request, is skipped.  Each close carries one line: where the fix landed.

    close_fixed_on_merge.py PR [--dry-run]   # what close-fixed-on-merge.yml runs
    close_fixed_on_merge.py selftest

The PR's commits are read as DATA (`git log` over fetched objects); nothing of the PR runs.
"""

import json
import os
import re
import subprocess
import sys

KEYWORD = re.compile(
    r"\b(?:fix(?:e[sd])?|close[sd]?|resolve[sd]?)\b:?\s+(#\d+(?:\s*(?:,|and|,\s*and)\s*#\d+)*)", re.I)


def closing_refs(text: str) -> list[int]:
    """The issue numbers `text` says it closes, in order, each once."""
    out: list[int] = []
    for m in KEYWORD.finditer(text):
        for n in re.findall(r"#(\d+)", m.group(1)):
            if int(n) not in out:
                out.append(int(n))
    return out


def gh(*args: str, check: bool = True) -> str:
    r = subprocess.run(["gh", *args], capture_output=True, text=True)
    if check and r.returncode != 0:
        raise RuntimeError(f"gh {' '.join(args[:3])}…: {r.stderr.strip()[:300]}")
    return r.stdout


def api(path: str) -> dict:
    return json.loads(gh("api", path) or "null")


def commit_messages(pr: int, base: str, head: str) -> str:
    """Every commit message between base and head: fetched by ref, so a PR of any size is
    read whole (the REST list of a PR's commits stops at 250)."""
    subprocess.run(["git", "fetch", "--quiet", "origin", f"pull/{pr}/head", base], capture_output=True)
    r = subprocess.run(["git", "log", "--format=%B%x00", f"{base}..{head}"], capture_output=True, text=True)
    return r.stdout


def run(pr_number: int, dry: bool) -> int:
    repo = os.environ.get("GH_REPO") or gh("repo", "view", "--json", "nameWithOwner", "--jq", ".nameWithOwner").strip()
    pr = api(f"repos/{repo}/pulls/{pr_number}")
    default = api(f"repos/{repo}")["default_branch"]
    if not pr.get("merged_at") or pr["base"]["ref"] != default:
        print(f"PR #{pr_number} is not merged into `{default}` — nothing to close")
        return 0
    text = (pr.get("body") or "") + "\n" + commit_messages(pr_number, pr["base"]["sha"], pr["head"]["sha"])
    sha = (pr.get("merge_commit_sha") or "")[:9]
    note = f"Fixed on `{default}` by #{pr_number}" + (f" ({sha})." if sha else ".")
    for n in closing_refs(text):
        if n == pr_number:
            continue
        issue = api(f"repos/{repo}/issues/{n}")
        if issue is None or "pull_request" in issue or issue.get("state") != "open":
            continue
        if dry:
            print(f"would close #{n}: {issue['title'][:70]}")
        else:
            gh("issue", "close", str(n), "--reason", "completed", "--comment", note)
            print(f"closed #{n}")
    return 0


def selftest() -> int:
    cases = [
        ("Fixes #12", [12]),
        ("fix it\n\nFixes #1570, #1731, #1732 and #1733", [1570, 1731, 1732, 1733]),
        ("Closes: #4\nResolved #5", [4, 5]),
        ("fixed #6, and #7", [6, 7]),
        ("Refs #8\nsee #9, loft#10 and (#11)", []),
        ("Fixes #3\nFixes #3", [3]),
        ("prefixes #14 and suffixes #15", []),
    ]
    bad = 0
    for text, want in cases:
        got = closing_refs(text)
        if got != want:
            print(f"FAIL {text!r}: {got} != {want}")
            bad += 1
    print("selftest:", "ok" if not bad else f"{bad} failure(s)")
    return 1 if bad else 0


if __name__ == "__main__":
    if sys.argv[1:] == ["selftest"]:
        sys.exit(selftest())
    if len(sys.argv) >= 2 and sys.argv[1].isdigit():
        sys.exit(run(int(sys.argv[1]), dry="--dry-run" in sys.argv[2:]))
    print(__doc__)
    sys.exit(2)
