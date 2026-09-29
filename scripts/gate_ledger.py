#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The gate's test ledger: a test that passed on this EXACT tree is not run again.

A gate cut short — cancelled by the budget, killed, or red on a flaky test — used to be
restarted from zero: 5468 of 5486 tests had passed when the budget cancelled one, and the
restart ran all 5486 again.  The ledger records every test that passed, keyed by a
FINGERPRINT of the whole working tree; the next gate on the same fingerprint runs only the
tests with no pass on record (the failed and the not-yet-run ones).

Why only an IDENTICAL tree.  Every test binary links `libloft`, and 156 of the 375 test
binaries read repository files at run time — docs, the `.loft` corpus, other tests' sources —
several through shared helpers in `tests/common` that no path rule can see.  So a pass taken
before ANY change says nothing about the tree after it.  A change of known reach is what
`ci-run.sh recheck` is for; a change of unknown reach owes a full gate (CI_BUDGET.md).

The fingerprint is the git tree of the working tree, untracked files included and ignored
files excluded, written through a throw-away index so the real index is untouched.

    gate_ledger.py plan     before the test run: prints ALL, NONE or FILTER, and on FILTER
                            writes the nextest filterset to target/gate-ledger/filter
    gate_ledger.py record   after it (also after a cancel or a kill): adds this run's passes
    gate_ledger.py fingerprint

`CI_FULL=1` makes `plan` answer ALL whatever the ledger says.
"""
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DIR = os.path.join(ROOT, "target", "gate-ledger")
LEDGER = os.path.join(DIR, "ledger.json")
PENDING = os.path.join(DIR, "pending.json")
FILTER = os.path.join(DIR, "filter")
RESULT = os.path.join(ROOT, "result.txt")
# Above this share of the suite still to run, a filter saves nothing worth its length.
RUN_ALL_ABOVE = 0.9
# nextest's per-test lines: `PASS [   0.3s] (12/34) <binary-id> <test>`, also after `TRY 2`.
LINE = re.compile(r"\b(PASS|FAIL) \[[^\]]*\] +\([^)]*\) +(\S+) +(\S+)\s*$")


def git(*args, env=None):
    return subprocess.run(["git", *args], cwd=ROOT, env=env, check=True,
                          capture_output=True, text=True).stdout.strip()


def fingerprint():
    """The git tree of the working tree as it stands, untracked files included."""
    os.makedirs(DIR, exist_ok=True)
    index = os.path.join(ROOT, git("rev-parse", "--git-path", "index"))
    fd, tmp = tempfile.mkstemp(dir=DIR, prefix="index.")
    os.close(fd)
    try:
        # Starting from the real index keeps git's stat cache, so only changed files are hashed.
        if os.path.exists(index):
            shutil.copyfile(index, tmp)
        env = dict(os.environ, GIT_INDEX_FILE=tmp)
        if not os.path.exists(index):
            git("read-tree", "HEAD", env=env)
        git("add", "-A", env=env)
        return git("write-tree", env=env)
    finally:
        os.unlink(tmp)


def load(path):
    try:
        with open(path) as f:
            return json.load(f)
    except (OSError, ValueError):
        return None


def save(path, data):
    os.makedirs(DIR, exist_ok=True)
    tmp = path + ".tmp"
    with open(tmp, "w") as f:
        json.dump(data, f)
    os.replace(tmp, path)


def selected_tests():
    """Every test the ci profile would run: (binary-id, name) for a matching, non-ignored test."""
    out = subprocess.run(["cargo", "nextest", "list", "--profile", "ci",
                          "--message-format", "json"],
                         cwd=ROOT, check=True, capture_output=True, text=True).stdout
    suites = json.loads(out)["rust-suites"]
    tests = {}
    for bid, suite in suites.items():
        names = [n for n, c in suite.get("testcases", {}).items()
                 if not c.get("ignored") and c.get("filter-match", {}).get("status") == "matches"]
        if names:
            tests[bid] = names
    return tests


def filterset(remaining, tests):
    """A nextest filterset naming `remaining`: a whole binary where all of it remains."""
    terms = []
    for bid in sorted(remaining):
        names = remaining[bid]
        if len(names) == len(tests[bid]):
            terms.append(f"binary_id(={bid})")
        else:
            terms.extend(f"(binary_id(={bid}) & test(={n}))" for n in sorted(names))
    return " | ".join(terms)


def say(msg):
    print(f"make ci: {msg}", file=sys.stderr)


def plan():
    fp = fingerprint()
    ledger = load(LEDGER) or {}
    carried = ledger.get("passed", []) if ledger.get("tree") == fp else []
    if os.environ.get("CI_FULL"):
        carried = []
    save(PENDING, {"tree": fp, "carried": carried, "since": ledger.get("since")
                   if carried else int(time.time())})
    if not carried:
        if ledger.get("tree") and ledger.get("tree") != fp and not os.environ.get("CI_FULL"):
            say("the tree differs from the last gate's, so every test runs "
                "(a change of known reach: ci-run.sh recheck)")
        print("ALL")
        return
    tests = selected_tests()
    done = {tuple(p) for p in carried}
    remaining = {}
    for bid, names in tests.items():
        left = [n for n in names if (bid, n) not in done]
        if left:
            remaining[bid] = left
    total = sum(len(n) for n in tests.values())
    left = sum(len(n) for n in remaining.values())
    stamp = time.strftime("%H:%M", time.localtime(ledger.get("since") or time.time()))
    if left == 0:
        say(f"RESUMED on an identical tree — all {total} tests passed on it since {stamp}; "
            "nothing left to run (CI_FULL=1 runs everything)")
        print("NONE")
        return
    if left > RUN_ALL_ABOVE * total:
        print("ALL")
        return
    with open(FILTER, "w") as f:
        f.write(filterset(remaining, tests))
    say(f"RESUMED on an identical tree — {total - left} of {total} tests passed on it since "
        f"{stamp} and carry over; running the {left} without a pass (CI_FULL=1 runs everything)")
    print("FILTER")


def passes_in_result():
    """This run's passed tests, from result.txt (truncated at the start of every gate)."""
    passed, failed = set(), set()
    try:
        with open(RESULT, errors="replace") as f:
            for line in f:
                m = LINE.search(line)
                if m:
                    (passed if m.group(1) == "PASS" else failed).add((m.group(2), m.group(3)))
    except OSError:
        pass
    return passed, failed


def record():
    pending = load(PENDING)
    if not pending:
        return
    # A tree edited while the tests ran gave passes that belong to neither tree.
    if fingerprint() != pending["tree"]:
        with open(RESULT, "a") as f:
            f.write("make ci: the tree changed while the tests ran — this run's passes are not "
                    "recorded, and the next gate runs every test\n")
        os.unlink(PENDING)
        return
    passed, _ = passes_in_result()
    union = {tuple(p) for p in pending["carried"]} | passed
    save(LEDGER, {"tree": pending["tree"], "since": pending["since"],
                  "passed": sorted(list(p) for p in union)})
    os.unlink(PENDING)


def selftest():
    lines = {
        "        PASS [   0.320s] (5068/5486) loft::threading_chars par_max_threads": ("PASS", "loft::threading_chars", "par_max_threads"),
        "  TRY 2 PASS [   1.001s] (   7/5486) loft::wrap last": ("PASS", "loft::wrap", "last"),
        "        FAIL [   0.760s] (  63/1132) loft::doc_hygiene every_new_guard": ("FAIL", "loft::doc_hygiene", "every_new_guard"),
        "        PASS [   0.014s] (  75/5486) loft cache::tests::prune_dir": ("PASS", "loft", "cache::tests::prune_dir"),
    }
    for line, want in lines.items():
        m = LINE.search(line)
        assert m and m.groups() == want, (line, m and m.groups())
    assert LINE.search("     Summary [ 723.461s] 5468/5486 tests run: 5468 passed") is None
    tests = {"a": ["x", "y"], "b": ["z"]}
    got = filterset({"a": ["y"], "b": ["z"]}, tests)
    assert got == "(binary_id(=a) & test(=y)) | binary_id(=b)", got
    print("gate_ledger selftest: ok")


def main():
    cmd = sys.argv[1] if len(sys.argv) > 1 else ""
    if cmd == "selftest":
        selftest()
        return
    if cmd == "plan":
        plan()
    elif cmd == "record":
        record()
    elif cmd == "fingerprint":
        print(fingerprint())
    else:
        print(__doc__)
        sys.exit(2)


if __name__ == "__main__":
    main()
