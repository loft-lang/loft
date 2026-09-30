#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
# Regenerate tests/ignored_tests.baseline from EVERY test in the tree.
# Usage: python3 tests/dump_ignored_tests.py > tests/ignored_tests.baseline
import glob
import re
import sys

HEADER = """\
# Baseline of every `#[ignore = "..."]` and `#[cfg_attr(<cfg>, ignore = "...")]` in
# tests/*.rs and src/**/*.rs.  One `<file>::<test_name>\\t<reason>` pair per line,
# sorted; the file is part of the key because two `src/` tests share a bare name.
# Checked by tests/doc_hygiene.rs::ignored_tests_baseline_is_current, which fails on
# any drift.  A drift typically means one of:
#   - an ignored test just got its fix landed (un-ignore it + delete its line here)
#   - a new ignored test landed (add its line here, with the reason it carries)
#   - the reason message changed (update the line here)
# `make release-checklist`'s A-ignores reads this file, so a test ignored WITHOUT a
# reason is a release finding, not just a lint.
# Regenerate with: `python3 tests/dump_ignored_tests.py > tests/ignored_tests.baseline`
"""

IGNORE = re.compile(r'\s*#\[(?:cfg_attr\([^,]+,\s*)?ignore\s*=\s*"(.+)"\s*\)?\]')
BARE = re.compile(r'\s*#\[(?:cfg_attr\([^,]+,\s*)?ignore\s*\)?\]')


def scan(path):
    with open(path, encoding="utf-8") as f:
        lines = f.read().splitlines()
    for i, line in enumerate(lines):
        m = IGNORE.match(line)
        reason = None
        if m:
            # Mirror the Rust test's unescape so both sides compare equal.
            reason = m.group(1).replace('\\"', '"').replace("\\\\", "\\")
        elif BARE.match(line):
            reason = ""
        else:
            continue
        for j in range(i + 1, min(i + 10, len(lines))):
            fm = re.match(r"\s*(?:pub(?:\([^)]*\))?\s+)?fn (\w+)\s*[(<]", lines[j])
            if fm:
                yield f"{path}::{fm.group(1)}", reason
                break


def main() -> int:
    files = sorted(glob.glob("tests/*.rs")) + sorted(glob.glob("src/**/*.rs", recursive=True))
    out = sorted(pair for p in files for pair in scan(p))
    sys.stdout.write(HEADER)
    for name, reason in out:
        sys.stdout.write(f"{name}\t{reason}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
