#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Read what a front end did to one program: its state trace and its IR (@PLN91).

The loft-written elaborator and the Rust front end print the same state lines under
`LOFT_STATE_TRACE=1` (doc/claude/plans/91-self-host/debugging-the-port.md says how to read
them).  This is the reader: one command, no set-up, filtered to what a bug is about.

    elab_trace.py PROG                          the elaborator's whole trace
    elab_trace.py PROG --fn main --pass 1       one function, the first pass
    elab_trace.py PROG --var total              one variable's history, both passes
    elab_trace.py PROG --points refuse,route    where it gave up, and the special routes it took
    elab_trace.py PROG --side both --fn main    the Rust front end's lines beside the elaborator's
    elab_trace.py PROG --side diff              only where the two differ (as state_diff.py)
    elab_trace.py PROG --ir main                the IR the elaborator wrote for one function
    elab_trace.py PROG --ir main --side diff    … against the Rust front end's

The two tools (the elaborator and the report writer) are built once into `target/elab-trace/`
and again only when `selfhost/` or the `loft` binary is newer; `--elab`, `--defs` and `--std`
take built ones (a gate cycle's).  A program's `@ARGS: --lib` directories are passed to both
sides, as the gate passes them.
"""
import argparse, difflib, os, re, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from elab_gate import lib_dirs, sections  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(HERE))
POINTS = ("stmt", "arg", "stmt-end", "var", "fn-end", "route", "refuse")
LINE = re.compile(r"^state (\d) (\d+):(\d+) (\S+)(?: (.*))?$")
FN_END = re.compile(r"^state \d \d+:\d+ fn-end (.+?) (?:ret=|var \d+ )")
VAR = re.compile(r"^state \d \d+:\d+ var (.+) (\d+) (\S+) (?:\+ |.* -> )")


def newest(paths):
    t = 0.0
    for p in paths:
        if os.path.isdir(p):
            for d, _, fs in os.walk(p):
                t = max([t] + [os.path.getmtime(os.path.join(d, f)) for f in fs if f.endswith(".loft")])
        elif os.path.exists(p):
            t = max(t, os.path.getmtime(p))
    return t


def tools(loft, o):
    """(elab, defs, std): the built tools, building what is missing or older than its source."""
    out = os.path.join(ROOT, "target", "elab-trace")
    src = newest([os.path.join(ROOT, "selfhost"), loft])
    built = []
    for name, given, rel in (("elab", o.elab, "selfhost/front/elab.loft"),
                             ("defs", o.defs, "selfhost/compare/defs.loft")):
        if given:
            built.append(given)
            continue
        exe = os.path.join(out, name, name)
        if not os.path.exists(exe) or os.path.getmtime(exe) < src:
            print(f"building {name} (once; again only when selfhost/ changes)…", file=sys.stderr)
            os.makedirs(os.path.dirname(exe), exist_ok=True)
            b = subprocess.run([loft, "--native-c", "--out", os.path.dirname(exe), os.path.join(ROOT, rel)],
                               env=dict(os.environ, LOFT_NO_CACHE="1", LOFT_TIMEOUT="1800"),
                               capture_output=True, text=True)
            if not os.path.exists(exe):
                sys.exit(f"{rel} did not build:\n{b.stderr[-2000:]}")
        built.append(exe)
    std = o.std or os.path.join(out, "std.store")
    if not o.std and (not os.path.exists(std) or os.path.getmtime(std) < src):
        empty = os.path.join(out, "empty.loft")
        with open(empty, "w", encoding="utf-8") as fh:
            fh.write("fn main() {}\n")
        subprocess.run([loft, empty], env=dict(os.environ, LOFT_NO_CACHE="1", LOFT_EMIT_PARSED=std),
                       capture_output=True)
        if not os.path.exists(std):
            sys.exit("the empty program wrote no image")
    return built[0], built[1], std


def named(want, name):
    """Does the definition `name` answer to what the user typed (`main` for `n_main`)?"""
    return name == want or name in (f"n_{want}", f"t_{want}") or name.endswith(f"_{want}")


def by_function(lines):
    """[(function, line)]: a line belongs to the function whose `fn-end` group closes it, as
    the gate cuts a trace; a `var` line names its function itself."""
    out, cur, name = [], [], None
    for l in lines:
        m = FN_END.match(l)
        if name is not None and (m is None or m.group(1) != name):
            out += [(name, x) for x in cur]
            cur, name = [], None
        cur.append(l)
        if m:
            name = m.group(1)
    out += [(name if name is not None else "(top)", x) for x in cur]
    return [((VAR.match(l).group(1) if VAR.match(l) else f), l) for f, l in out]


def select(text, o):
    lines = [l for l in text.splitlines() if LINE.match(l)]
    keep = []
    for fn, l in by_function(lines):
        m = LINE.match(l)
        if o.pass_ and m.group(1) != str(o.pass_):
            continue
        if o.points and m.group(4) not in o.points:
            continue
        if o.fn and not named(o.fn, fn):
            continue
        if o.var:
            v = VAR.match(l)
            e = re.match(r"^state \d \d+:\d+ fn-end .+? var \d+ ([^=]+)=", l)
            if not ((v and v.group(3) == o.var) or (e and e.group(1) == o.var)):
                continue
        keep.append(l)
    return keep


def show(a, b, side, a_name="rust", b_name="port"):
    if side == "rust":
        print("\n".join(a))
    elif side == "port":
        print("\n".join(b))
    elif side == "both":
        print(f"── {a_name} ──\n" + "\n".join(a) + f"\n── {b_name} ──\n" + "\n".join(b))
    else:
        d = list(difflib.unified_diff(a, b, a_name, b_name, lineterm="", n=2))
        print("\n".join(d) if d else "the two sides agree")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("prog")
    ap.add_argument("--fn", help="one function (`main` finds `n_main`)")
    ap.add_argument("--var", help="one variable: its `var` lines and its `fn-end` line")
    ap.add_argument("--pass", dest="pass_", type=int, choices=(1, 2))
    ap.add_argument("--points", help="comma-separated: " + ", ".join(POINTS))
    ap.add_argument("--side", choices=("port", "rust", "both", "diff"), default="port")
    ap.add_argument("--ir", metavar="NAME", help="the definition's IR instead of the trace")
    ap.add_argument("--loft", default=os.path.join(ROOT, "target", "release", "loft"))
    ap.add_argument("--elab", help="a built elaborator")
    ap.add_argument("--defs", help="a built report writer (for --ir on the Rust side)")
    ap.add_argument("--std", help="an empty program's image")
    o = ap.parse_args()
    if o.points:
        o.points = [p.strip() for p in o.points.split(",")]
        bad = [p for p in o.points if p not in POINTS]
        if bad:
            sys.exit(f"no such point: {', '.join(bad)} (the points are {', '.join(POINTS)})")
    prog = os.path.abspath(o.prog)
    with open(prog, encoding="utf-8", errors="replace") as fh:
        libs = lib_dirs(fh.read())
    elab, defs, std = tools(o.loft, o)
    env = dict(os.environ, LOFT_NO_CACHE="1", LOFT_STATE_TRACE="1", LOFT_ELAB_LIB=":".join(libs),
               LOFT_LIB_PATH=os.path.dirname(elab))
    with tempfile.TemporaryDirectory() as tmp:
        img = os.path.join(tmp, "p.store")
        r = subprocess.run([o.loft] + [a for d in libs for a in ("--lib", d)] + [prog],
                           env=dict(env, LOFT_EMIT_PARSED=img), capture_output=True, text=True, errors="replace")
        if not os.path.exists(img):
            sys.exit("the Rust front end refuses the program, so there is no image to elaborate:\n"
                     + "\n".join(l for l in r.stderr.splitlines() if not l.startswith("state "))[-1500:])
        m = subprocess.run([elab, img, prog, std], env=env, capture_output=True, text=True, errors="replace")
        want = ""
        if o.ir and o.side != "port":
            want = subprocess.run([defs, img, "program", std], env=env, capture_output=True, text=True,
                                  errors="replace").stdout
    refused = [l[len("refused: "):] for l in m.stdout.splitlines() if l.startswith("refused: ")]
    if o.ir:
        def ir(report):
            hit = [(k, v) for k, v in sections(report.splitlines()).items() if named(o.ir, k.split("#")[0])]
            return [l for _, v in hit for l in v]
        mine = [l for l in m.stdout.splitlines() if not l.startswith(("refused: ", "refused-in: "))]
        a, b = ir(want), ir("\n".join(mine))
        if not b and o.side != "rust":
            print(f"the elaborator wrote no definition named `{o.ir}`", file=sys.stderr)
        show(a, b, o.side)
    else:
        show(select(r.stderr, o), select(m.stderr, o), o.side)
    if refused:
        print(f"\nthe elaborator refuses this program ({len(refused)}): " + " | ".join(sorted(set(refused)))[:600],
              file=sys.stderr)


if __name__ == "__main__":
    main()
