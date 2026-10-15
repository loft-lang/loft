#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# diag_position_audit.py — does a diagnostic's caret point at the code it names?
#
# The position a diagnostic carries is the one channel the corpus captures and
# never compares: `tests/wrap.rs::check_diagnostics` matches an `@EXPECT_ERROR` /
# `@EXPECT_WARNING` by SUBSTRING, so the `file:line:col` on all 272 annotations is
# dropped.  `tests/parse_errors.rs` does pin `line:col` exactly — but almost every
# fixture there is a one-liner or ends in `;`, and the `;` is precisely what hides
# a caret that follows the scan cursor onto the next token's line
# (DIAGNOSTICS.md § Adding a code, item 6).
#
# So this is the corpus-wide re-measure, the position twin of LOFT_TRACE_ASSERTS.
# It is a REPORT, never a gate: both filters below have false positives that a
# line-numbered allow-list could only pin by rotting.
#
#   ident  a message quoting `name` / 'name' must point at a line containing it.
#          False-positives when the quoted name is a TYPE the line never spells
#          (`a nullable integer? is stored into ...`) — read the hits, don't count
#          them.
#   brace  the caret sits on a line that is nothing but `}` / `)` / `];`.  Sharp:
#          this is the shape a cursor-following caret produces.  A whole-CONSTRUCT
#          check (circular init, a generator's discarded tail) legitimately lands
#          on the construct's own closing brace, which is the residue to expect.
#
# Usage:  python3 scripts/diag_position_audit.py [--ident] [--brace] [glob ...]
#         (no filter flag = both; no glob = tests/scripts + tests/docs)

import glob as globmod
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target/release/loft")

DIAG = re.compile(r"^(Error|Warning|Advice|Hint)(?:\[[^\]]*\])?:\s*(.*)\s+at\s+(\S+):(\d+):(\d+)\s*$")
QUOTED = re.compile(r"[`']([A-Za-z_][A-Za-z0-9_]*)[`']")
CLOSING = re.compile(r"^[}\)\];,\s]+$")


def file_args(path):
    """Honour a script's own `// @ARGS:` header, as the test harnesses do."""
    args = []
    for line in list(open(path, encoding="utf-8", errors="replace"))[:20]:
        t = line.strip()
        if t.startswith("// @ARGS:"):
            args += t[len("// @ARGS:") :].split()
    return args


def diagnostics(path):
    env = dict(os.environ, LOFT_ERRORS="compact", LOFT_TIMEOUT="30")
    try:
        p = subprocess.run(
            [BIN, "--interpret", *file_args(path), path],
            capture_output=True, text=True, timeout=90, env=env,
            cwd=os.path.dirname(path),
        )
    except (subprocess.TimeoutExpired, OSError):
        return []
    out = []
    for line in (p.stderr + "\n" + p.stdout).splitlines():
        m = DIAG.match(line.strip())
        if m:
            out.append((m.group(2).strip(), os.path.realpath(m.group(3)), int(m.group(4))))
    return out


def main():
    argv = [a for a in sys.argv[1:]]
    want_ident = "--ident" in argv
    want_brace = "--brace" in argv
    if not (want_ident or want_brace):
        want_ident = want_brace = True
    patterns = [a for a in argv if not a.startswith("--")]
    if patterns:
        files = sorted({f for p in patterns for f in globmod.glob(p)})
    else:
        # tests/scripts, tests/docs and every program of the language suite beside the tree
        # (@PLN193): the audit covers the corpus it covered before the move.
        sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
        from corpus import corpus_files
        files = corpus_files()

    src = {}

    def lines_of(p):
        if p not in src:
            try:
                src[p] = open(p, encoding="utf-8", errors="replace").read().split("\n")
            except OSError:
                src[p] = []
        return src[p]

    total = 0
    ident_hits, brace_hits = [], []
    for path in files:
        for msg, dpath, line in diagnostics(path):
            if "/default/" in dpath:  # a stdlib position is a different question
                continue
            total += 1
            body = lines_of(dpath)
            if not 1 <= line <= len(body):
                continue
            text = body[line - 1]
            if want_brace and text.strip() and CLOSING.match(text):
                brace_hits.append((dpath, line, text.strip(), msg))
            if want_ident:
                names = QUOTED.findall(msg)
                if names and not any(
                    re.search(r"\b" + re.escape(n) + r"\b", text) for n in names
                ):
                    ident_hits.append((dpath, line, text.strip(), msg))

    print(f"{len(files)} files, {total} diagnostics")
    for label, hits in (("brace", brace_hits), ("ident", ident_hits)):
        if not ((label == "brace" and want_brace) or (label == "ident" and want_ident)):
            continue
        print(f"\n=== {label}: {len(hits)} ===")
        for dpath, line, text, msg in sorted(set(hits)):
            print(f"{os.path.relpath(dpath, ROOT)}:{line}\n    line: {text[:90]}\n    msg : {msg[:100]}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
