#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""One program against the elaboration gate's kept tools (@PLN91 stage 1, gate E).

The gate (`elab_gate.py`) answers for the whole corpus in minutes; this answers for one
program in a second, with tools built before:

  elab_probe.py one FILE              the two reports diffed (`AGREE`, or the diff)
  elab_probe.py where FILE            each refusal with the source line it was first met on
  elab_probe.py fam TEXT VERDICTS     the source lines behind refusal TEXT over the programs
                                      a `LOFT_ELAB_VERDICTS` file lists with it

The tools: `--tools DIR` is a gate run's kept directory (`LOFT_ELAB_KEEP=1`: `defs/defs`,
`std.store`); `--elab EXE` is the elaborator to ask — a development build
(`loft --native-c --out DIR selfhost/front/elab.loft`) by default `DIR/elab/elab`.  Both
default to `$LOFT_ELAB_TOOLS` and `$LOFT_ELAB_BIN`.
"""
import argparse, collections, difflib, os, subprocess, sys, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def reports(o, path):
    """(Rust's report, loft's report, the refusals' `where` lines) for one program."""
    path = os.path.realpath(path)
    env = dict(os.environ, LOFT_NO_CACHE="1")
    with tempfile.TemporaryDirectory(prefix="loft-probe-") as tmp:
        img = os.path.join(tmp, "p.store")
        subprocess.run([o.loft, path], env=dict(env, LOFT_EMIT_PARSED=img), capture_output=True,
                       timeout=180)
        if not os.path.exists(img):
            return None, None, []
        std = os.path.join(o.tools, "std.store")
        want = subprocess.run([os.path.join(o.tools, "defs", "defs"), img, "program", std],
                              env=env, capture_output=True, text=True, errors="replace",
                              timeout=300).stdout.splitlines()
        m = subprocess.run([o.elab, img, path, std], env=dict(env, LOFT_ELAB_WHERE="1"),
                           capture_output=True, text=True, errors="replace", timeout=300)
    return want, m.stdout.splitlines(), [l for l in m.stderr.splitlines() if l.startswith("where: ")]


def where(o, path):
    """[(line number, refusal, source line)] of a program's refusals."""
    _, _, lines = reports(o, path)
    with open(path, encoding="utf-8", errors="replace") as fh:
        src = fh.read().splitlines()
    out = []
    for l in lines:
        head, _, what = l[len("where: line "):].partition(": ")
        n = int(head)
        out.append((n, what, src[n - 1].strip() if 0 < n <= len(src) else ""))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("what", choices=["one", "where", "fam"])
    ap.add_argument("arg", help="a program, or the refusal text for `fam`")
    ap.add_argument("verdicts", nargs="?", help="`fam`: a LOFT_ELAB_VERDICTS file")
    ap.add_argument("--tools", default=os.environ.get("LOFT_ELAB_TOOLS", ""))
    ap.add_argument("--elab", default=os.environ.get("LOFT_ELAB_BIN", ""))
    ap.add_argument("--loft", default=os.path.join(ROOT, "target", "release", "loft"))
    ap.add_argument("--max", type=int, default=40, help="`fam`: programs read; `one`: diff lines")
    o = ap.parse_args()
    if not o.tools:
        sys.exit("name the kept tools: --tools DIR or LOFT_ELAB_TOOLS (a LOFT_ELAB_KEEP=1 gate run)")
    o.elab = o.elab or os.path.join(o.tools, "elab", "elab")
    if o.what == "one":
        want, mine, _ = reports(o, o.arg)
        if want is None:
            sys.exit("the Rust front end wrote no image")
        if want == mine:
            print("AGREE")
            return
        for l in list(difflib.unified_diff(want, mine, "rust", "loft", lineterm="", n=0))[:o.max]:
            print(l)
        sys.exit(1)
    if o.what == "where":
        for n, what, src in where(o, o.arg):
            print(f"line {n}: {what}\n    | {src}")
        return
    if not o.verdicts:
        sys.exit("fam needs the verdicts file of a gate run (LOFT_ELAB_VERDICTS)")
    seen = collections.Counter()
    base = os.path.dirname(ROOT)
    with open(o.verdicts, encoding="utf-8") as fh:
        rels = [l.split("\t")[1] for l in fh if o.arg in l and l.count("\t") >= 2]
    for rel in rels[:o.max]:
        for _, what, src in where(o, os.path.join(base, rel)):
            if o.arg in what:
                seen[src[:170]] += 1
    for src, k in seen.most_common():
        print(f"{k:4}  {src}")


if __name__ == "__main__":
    main()
