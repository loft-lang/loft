#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The quick gate before an elaborator build (@PLN91): the front end only, and only what changed.

A development build of the elaborator (`loft --native-c --out DIR selfhost/front/elab.loft`) takes
a quarter of an hour, and the corpus gate as long again; an edit that does not parse or type
costs that whole cycle.  This runs `loft check` on the elaborator — parse and compile, no C, no
run — and reads its diagnostics against `git diff`:

  - any ERROR fails the gate, wherever it is;
  - a WARNING or ADVICE is shown when it points at a line the working tree changed against
    REF (default HEAD) — the lint of the edit, the existing diagnostics left out.

  elab_pregate.py [--ref REF] [--loft EXE]     exit 0 = clean, 1 = an error, 2 = a new warning

Checked as a whole: a module checked as its own entry types its `use` graph in another order and
reports errors the real build does not.
"""
import argparse, os, re, subprocess, sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
HEAD = re.compile(r"^(error|warning|advice)(\[[^\]]*\])?: (.*)$")
AT = re.compile(r"^\s*--> (.+?):(\d+)(?::\d+)?$")


def changed_lines(ref):
    """{absolute path: set of changed line numbers} of the working tree against `ref`, the
    selfhost `.loft` files; a new untracked file counts whole."""
    out = {}
    diff = subprocess.run(["git", "diff", "-U0", ref, "--", "selfhost"], cwd=ROOT,
                          capture_output=True, text=True).stdout
    path = None
    for l in diff.splitlines():
        if l.startswith("+++ "):
            p = l[4:]
            path = os.path.join(ROOT, p[2:]) if p.startswith("b/") else None
        elif l.startswith("@@") and path and path.endswith(".loft"):
            m = re.match(r"@@ -\S+ \+(\d+)(?:,(\d+))? @@", l)
            if m:
                start, n = int(m.group(1)), int(m.group(2) or 1)
                out.setdefault(path, set()).update(range(start, start + n))
    new = subprocess.run(["git", "ls-files", "--others", "--exclude-standard", "selfhost"],
                         cwd=ROOT, capture_output=True, text=True).stdout
    for p in new.splitlines():
        if p.endswith(".loft"):
            out[os.path.join(ROOT, p)] = None
    return out


def diagnostics(text):
    """[(level, message, path, line, block)] of `loft check`'s output."""
    found, cur = [], None
    for l in text.splitlines():
        m = HEAD.match(l)
        if m:
            cur = [m.group(1), m.group(3), None, 0, [l]]
            found.append(cur)
            continue
        if cur is not None:
            cur[4].append(l)
            a = AT.match(l)
            if a and cur[2] is None:
                cur[2], cur[3] = os.path.realpath(a.group(1)), int(a.group(2))
    return found


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--ref", default="HEAD")
    ap.add_argument("--loft", default=os.path.join(ROOT, "target-probe", "release", "loft"))
    o = ap.parse_args()
    changed = changed_lines(o.ref)
    if not changed:
        print("pre-gate: no selfhost .loft change against", o.ref)
        return 0
    r = subprocess.run([o.loft, "check", os.path.join(ROOT, "selfhost", "front", "elab.loft")],
                       cwd=ROOT, capture_output=True, text=True, errors="replace")
    diags = diagnostics(r.stdout + r.stderr)
    errors = [d for d in diags if d[0] == "error" and not d[1].startswith("aborting")]
    fresh = []
    for d in diags:
        if d[0] == "error" or d[2] is None:
            continue
        lines = changed.get(d[2], set())
        if lines is None or d[3] in lines:
            fresh.append(d)
    for d in errors + fresh:
        print("\n".join(d[4]).rstrip())
        print()
    if errors:
        print(f"pre-gate: {len(errors)} error(s) — the build would fail")
        return 1
    if fresh:
        print(f"pre-gate: no error; {len(fresh)} warning(s) on the changed lines")
        return 2
    print(f"pre-gate: clean ({len(changed)} changed file(s); {len(diags)} diagnostics elsewhere)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
