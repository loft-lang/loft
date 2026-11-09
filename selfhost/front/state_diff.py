#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Diff the scoped parser state of the Rust front end and the elaborator on one program (@PLN91).

Both print `state <pass> <line>:<col> <point> <channel>=<value>…` under `LOFT_STATE_TRACE=1`
(`src/parser/state_trace.rs`, `selfhost/src/el_trace.loft`).  Where gate E says only that the IR
differs, this names the first point — and the channel — where the state the IR was built from
already did.

    state_diff.py --elab <elab binary> --std <empty program's image> PROG [--context N]

`--elab` and `--std` are what `elab_gate.py` builds (its temp dir) or a local build of
`front/elab.loft` and an image of an empty program (`LOFT_EMIT_PARSED`).  A program's
`@ARGS: --lib` directories are passed to both sides, as the gate passes them.
"""
import argparse, difflib, os, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from elab_gate import lib_dirs  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(HERE))


def trace_lines(text):
    return [l for l in text.splitlines() if l.startswith("state ")]


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("prog")
    ap.add_argument("--elab", required=True, help="the elaborator binary")
    ap.add_argument("--std", required=True, help="an empty program's image")
    ap.add_argument("--loft", default=os.path.join(ROOT, "target", "release", "loft"))
    ap.add_argument("--context", type=int, default=2, help="lines around each difference")
    ap.add_argument("--all", action="store_true", help="every difference, not the first")
    o = ap.parse_args()
    prog = os.path.abspath(o.prog)
    with open(prog, encoding="utf-8", errors="replace") as fh:
        libs = lib_dirs(fh.read())
    env = dict(os.environ, LOFT_NO_CACHE="1", LOFT_STATE_TRACE="1", LOFT_ELAB_LIB=":".join(libs))
    with tempfile.TemporaryDirectory() as tmp:
        img = os.path.join(tmp, "p.store")
        args = [a for d in libs for a in ("--lib", d)]
        r = subprocess.run([o.loft] + args + [prog], env=dict(env, LOFT_EMIT_PARSED=img),
                           capture_output=True, text=True, errors="replace")
        if r.returncode != 0 or not os.path.exists(img):
            sys.exit("the Rust front end refuses the program (no image)")
        m = subprocess.run([o.elab, img, prog, o.std], env=env, capture_output=True, text=True,
                           errors="replace")
    rust, port = trace_lines(r.stderr), trace_lines(m.stderr)
    print(f"rust {len(rust)} points, elaborator {len(port)} points")
    sm = difflib.SequenceMatcher(a=rust, b=port, autojunk=False)
    shown = 0
    for tag, i1, i2, j1, j2 in sm.get_opcodes():
        if tag == "equal":
            continue
        lo = max(0, i1 - o.context)
        for l in rust[lo:i1]:
            print(f"  {l}")
        for l in rust[i1:i2]:
            print(f"- {l}")
        for l in port[j1:j2]:
            print(f"+ {l}")
        print("  …")
        shown += 1
        if not o.all and shown >= 1:
            break
    if shown == 0:
        print("the traces agree")


if __name__ == "__main__":
    main()
