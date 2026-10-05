#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Does a lint change what it reports, file by file, over every loft program we can reach?

    scripts/lint_sweep.py --base <loft> --head <loft> --code lost-write ROOT...

Runs `loft --check` on every tracked `.loft` file under each ROOT (a git checkout) with
both binaries, from the file's own directory, and compares how often `warning[CODE]` /
`advice[CODE]` is reported.  A new or widened lint reaches a library's CI as a warning,
so its reach over the libraries and the consumers is measured before it lands, not
discovered by them.

Writes `<out>/sweep.tsv` (file, base count, head count, base exit, head exit) and
`<out>/summary.md`: the files whose count moved, each with the diagnostics the head
binary printed for that code, and how many files did not compile on each side — those
files say nothing about the lint, so the coverage is stated rather than implied.
"""

import argparse
import concurrent.futures
import os
import re
import subprocess
import sys


def tracked_loft(root):
    out = subprocess.run(
        ["git", "-C", root, "ls-files", "*.loft"], capture_output=True, text=True, check=True
    ).stdout.split()
    return [os.path.join(root, f) for f in out]


def check(binary, path, code, timeout, libs):
    env = dict(os.environ, LOFT_NO_CACHE="1")
    cmd = [binary]
    for lib in libs:
        cmd += ["--lib", lib]
    cmd += ["--check", os.path.abspath(path)]
    try:
        r = subprocess.run(
            cmd,
            cwd=os.path.dirname(os.path.abspath(path)),
            capture_output=True,
            text=True,
            timeout=timeout,
            env=env,
            stdin=subprocess.DEVNULL,
        )
        text = r.stdout + r.stderr
        status = r.returncode
    except subprocess.TimeoutExpired:
        text, status = "", "timeout"
    pat = re.compile(r"^(warning|advice)\[" + re.escape(code) + r"\]", re.M)
    return len(pat.findall(text)), status, text


def excerpt(text, code):
    """The head binary's diagnostics for `code`: each from its header to the next blank line."""
    out, keep = [], False
    for line in text.splitlines():
        if re.match(r"^(warning|advice|error)\[", line):
            keep = f"[{code}]" in line
        if keep:
            out.append(line)
        if keep and not line.strip():
            keep = False
    return "\n".join(out).strip()


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--base", required=True, help="the loft binary before the change")
    ap.add_argument("--head", required=True, help="the loft binary after the change")
    ap.add_argument("--code", required=True, help="the diagnostic code to count")
    ap.add_argument("--out", default="lint-sweep", help="output directory")
    ap.add_argument("--jobs", type=int, default=os.cpu_count() or 2)
    ap.add_argument("--timeout", type=int, default=120)
    ap.add_argument(
        "--lib",
        action="append",
        default=[],
        metavar="ROOT=DIR",
        help="pass `--lib DIR` (relative to ROOT) for every file under ROOT; repeatable",
    )
    ap.add_argument("roots", nargs="+")
    a = ap.parse_args()
    # Each check runs from its file's own directory, so the binaries are resolved here.
    a.base, a.head = os.path.abspath(a.base), os.path.abspath(a.head)
    os.makedirs(a.out, exist_ok=True)

    libs = {}
    for spec in a.lib:
        root, _, d = spec.partition("=")
        libs.setdefault(os.path.abspath(root), []).append(os.path.join(os.path.abspath(root), d))
    files = []
    for root in a.roots:
        for f in tracked_loft(root):
            files.append((f, libs.get(os.path.abspath(root), [])))
    print(f"lint-sweep: {len(files)} files under {len(a.roots)} roots, code {a.code}", flush=True)

    def both(item):
        f, lib = item
        b = check(a.base, f, a.code, a.timeout, lib)
        h = check(a.head, f, a.code, a.timeout, lib)
        return f, b, h

    rows = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=a.jobs) as pool:
        for i, row in enumerate(pool.map(both, files), 1):
            rows.append(row)
            if i % 500 == 0:
                print(f"  {i}/{len(files)}", flush=True)

    moved = [(f, b, h) for f, b, h in rows if b[0] != h[0]]
    unbuilt = {
        side: sum(1 for _, b, h in rows if (b if side == "base" else h)[1] != 0)
        for side in ("base", "head")
    }
    with open(os.path.join(a.out, "sweep.tsv"), "w") as t:
        t.write("file\tbase\thead\tbase_exit\thead_exit\n")
        for f, b, h in rows:
            t.write(f"{f}\t{b[0]}\t{h[0]}\t{b[1]}\t{h[1]}\n")
    total_b = sum(b[0] for _, b, _ in rows)
    total_h = sum(h[0] for _, _, h in rows)
    with open(os.path.join(a.out, "summary.md"), "w") as s:
        s.write(f"## `{a.code}` over {len(files)} files\n\n")
        s.write(f"Reports: base {total_b}, head {total_h}.  ")
        s.write(f"Files whose count moved: {len(moved)}.  ")
        s.write(
            f"Files that did not compile (no answer about the lint): "
            f"base {unbuilt['base']}, head {unbuilt['head']}.\n\n"
        )
        if moved:
            s.write("| file | base | head |\n|---|---|---|\n")
            for f, b, h in sorted(moved):
                s.write(f"| `{f}` | {b[0]} | {h[0]} |\n")
            s.write("\n")
            for f, b, h in sorted(moved):
                s.write(f"### `{f}`\n\n```\n{excerpt(h[2], a.code) or '(none on head)'}\n```\n\n")
        changed_status = [(f, b[1], h[1]) for f, b, h in rows if b[1] != h[1]]
        if changed_status:
            s.write("### Files whose exit status changed\n\n| file | base | head |\n|---|---|---|\n")
            for f, x, y in sorted(changed_status):
                s.write(f"| `{f}` | {x} | {y} |\n")
    print(open(os.path.join(a.out, "summary.md")).read())
    return 0


if __name__ == "__main__":
    sys.exit(main())
