#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Which programs' C units the loft-written generator writes byte for byte as the Rust one.

@PLN91 strand 12, doc/claude/plans/91-self-host/12-c-generator.md § Steps, G-0.  For every
plain program of the coverage census's corpus (`selfhost/cover/census.py`: tests/scripts and
the language suite), the unit `loft --emit-c` writes (the ORACLE: a program's unit, or its
tests' unit where it has no `main`) is compared with a second unit:

  default            the Rust generator's unit written from the program IMAGE
                     (`LOFT_EMIT_FROM_IMAGE=1`): how many programs the image carries whole —
                     the corpus a generator that reads the image can reach
  --cgen             the loft-written generator's unit (selfhost/front/cgen.loft, built here
                     through the C backend): `cgen <image> <out.c> <src/c>` on the image
                     `loft --emit-image` writes
  --generator EXE    the same with a generator built elsewhere: `EXE <image> <out.c> <src/c>`

`--form debug` compares the DEBUG unit instead (`--native-c --native-debug --native-emit`: the
debug tier's own image, a `#line` per statement, no whole-program rewrite), which only a
generator can be asked for.

`--layer shapes` compares a LAYER instead of the unit: what an analysis derived, one line per
function (12-c-generator.md § The layers beside the unit).  The Rust side prints it while it
writes the unit (`LOFT_TRACE_SHAPES=1`), the generator with `<image> <out> --layer shapes`;
every function the Rust side printed must have the same line on the other side.

A program is counted `agree`; `differ` (the first line that differs, both sides); `refused`
(the second side wrote no unit: its last line of standard error, ranked); `nounit` (the oracle
writes none: the C backend refuses the program, which is no program of this corpus).

    python3 selfhost/bench/cgen_survey.py [--generator EXE] [--scripts-only] [--jobs N] [--top 30]
"""
import argparse, collections, concurrent.futures as cf, os, re, shutil, subprocess, sys, tempfile

sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "cover"))
import census  # noqa: E402  the corpus and the plain-program filter are the census's own


# Where the C runtime's headers are: the generator reads them, as the Rust build embeds them.
RUNTIME_DIR = os.path.join(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))), "src", "c")


def proc(cmd, env, timeout, cwd=None):
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, env=env, timeout=timeout, cwd=cwd)
        return p.returncode, p.stderr
    except subprocess.TimeoutExpired:
        return -9, "timeout"


def last(err):
    lines = [l for l in err.strip().splitlines() if l.strip()]
    return (lines[-1] if lines else "no message")[:110]


def first_difference(a, b):
    """The first line two files differ at: `(line number, a's line, b's line)`."""
    with open(a, encoding="utf-8", errors="replace") as fa, open(b, encoding="utf-8", errors="replace") as fb:
        n = 0
        while True:
            n += 1
            la, lb = fa.readline(), fb.readline()
            if la != lb:
                return n, la.rstrip("\n")[:100], lb.rstrip("\n")[:100]
            if not la:
                return n, "", ""


def layer_one(f, loft, generator, tmp, env, timeout, layer):
    """One program's `layer`: each line the Rust generator traced against the generator's."""
    stem = os.path.join(tmp, re.sub(r"[^A-Za-z0-9]", "_", f))
    want, got, img = stem + ".want.c", stem + ".layer", stem + ".store"
    switch = {"shapes": "LOFT_TRACE_SHAPES", "reach": "LOFT_TRACE_REACH", "functions": "LOFT_TRACE_REACH",
              "counters": "LOFT_TRACE_COUNTERS", "iterations": "LOFT_TRACE_COUNTERS", "nn": "LOFT_TRACE_FACTS", "ranges": "LOFT_TRACE_FACTS"}[layer]
    try:
        rc, err = proc([loft, "--emit-c", want, f], {**env, switch: "1"}, timeout, cwd=os.path.dirname(f))
        if rc != 0:
            return f, "nounit", last(err)
        rust = {}
        for line in err.splitlines():
            if line.startswith(f"[{layer}]\t"):
                rust[line.split("\t")[1]] = line
        rc, err = proc([loft, "--emit-image", img, f], env, timeout, cwd=os.path.dirname(f))
        if rc == 0:
            rc, err = proc([generator, img, got, "--layer", layer], env, timeout, cwd=os.path.dirname(f))
        if rc != 0 or not os.path.exists(got):
            return f, "refused", last(err)
        mine = {}
        with open(got, encoding="utf-8", errors="replace") as fh:
            for line in fh.read().splitlines():
                if line.startswith(f"[{layer}]\t"):
                    mine[line.split("\t")[1]] = line
        for name, line in sorted(rust.items()):
            if mine.get(name) != line:
                return f, "differ", f"{name}\n    oracle: {line[:160]}\n    other:  {(mine.get(name) or '(no line)')[:160]}"
        return f, "agree", str(len(rust))
    finally:
        for p in (want, got, img):
            if os.path.exists(p):
                os.unlink(p)


def one(f, loft, generator, tmp, env, timeout, debug=False):
    stem = os.path.join(tmp, re.sub(r"[^A-Za-z0-9]", "_", f))
    want, got, img = stem + ".want.c", stem + ".got.c", stem + ".store"
    try:
        tier = ["--native-debug"] if debug else []
        oracle = [loft, "--native-c", "--native-debug", "--native-emit", want, f] if debug else [loft, "--emit-c", want, f]
        rc, err = proc(oracle, env, timeout, cwd=os.path.dirname(f))
        if rc != 0 or not os.path.exists(want):
            return f, "nounit", last(err)
        if generator:
            rc, err = proc([loft] + tier + ["--emit-image", img, f], env, timeout, cwd=os.path.dirname(f))
            if rc == 0:
                rc, err = proc([generator, img, got, RUNTIME_DIR] + (["--debug"] if debug else []), env, timeout,
                               cwd=os.path.dirname(f))
        else:
            rc, err = proc([loft, "--emit-c", got, f], {**env, "LOFT_EMIT_FROM_IMAGE": "1"}, timeout,
                           cwd=os.path.dirname(f))
        if rc != 0 or not os.path.exists(got):
            return f, "refused", re.sub(r"^cgen: not carried: (\w+: )?", "", last(err))
        with open(want, "rb") as a, open(got, "rb") as b:
            if a.read() == b.read():
                return f, "agree", ""
        n, la, lb = first_difference(want, got)
        return f, "differ", f"line {n}\n    oracle: {la}\n    other:  {lb}"
    finally:
        for p in (want, got, img):
            if os.path.exists(p):
                os.unlink(p)


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--loft", default=None, help="the loft binary (default: target/release/loft)")
    ap.add_argument("--generator", default=None, help="a built generator: EXE <image> <out.c> <src/c>")
    ap.add_argument("--cgen", action="store_true", help="build selfhost/front/cgen.loft and use it")
    ap.add_argument("--form", choices=["program", "debug"], default="program",
                    help="which unit: the program's, or the debug tier's with its #line marks")
    ap.add_argument("--layer", choices=["shapes", "reach", "functions", "counters", "iterations", "nn", "ranges"], default=None,
                    help="compare what an analysis derived instead of the unit")
    ap.add_argument("--jobs", type=int, default=6)
    ap.add_argument("--scripts-only", action="store_true", help="tests/scripts only, not the language suite")
    ap.add_argument("--chapter", action="append", default=[], help="a suite chapter (repeatable)")
    ap.add_argument("--top", type=int, default=30)
    ap.add_argument("--timeout", type=int, default=120)
    ap.add_argument("--show", action="append", default=[],
                    help="list the programs of a verdict (agree, differ, refused, nounit; repeatable)")
    o = ap.parse_args()
    root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    loft = o.loft or os.path.join(root, "target", "release", "loft")
    suite = os.environ.get("LOFT_SUITE_DIR", os.path.join(os.path.dirname(root), "loft-suite"))
    env = {**os.environ, "LOFT_NO_WARNINGS": "1"}
    tmp = tempfile.mkdtemp(prefix="loft-cgen-", dir=os.environ.get("TMPDIR"))
    if (o.form == "debug" or o.layer) and not (o.cgen or o.generator):
        sys.exit("--form debug and --layer compare a generator's output: give --cgen or --generator")
    try:
        if o.cgen:
            out = os.path.join(tmp, "cgen")
            os.makedirs(out, exist_ok=True)
            p = subprocess.run([loft, "--native-c", "--ship", "--out", out,
                                "--lib", os.path.join(root, "selfhost", "src"),
                                "--lib", os.path.join(root, "selfhost", "crt", "src"),
                                os.path.join(root, "selfhost", "front", "cgen.loft")],
                               capture_output=True, text=True, env=env, timeout=900)
            o.generator = os.path.join(out, "cgen")
            if not os.path.exists(o.generator):
                sys.exit(f"the generator did not build:\n{p.stderr[-2000:]}")
        files = [os.path.abspath(f) for f in census.corpus(root, suite if os.path.isdir(suite) else None,
                                                           o.chapter, o.scripts_only) if census.plain(f)]
        counts = collections.Counter()
        why = collections.Counter()
        shown = []
        run = ((lambda f: layer_one(f, loft, o.generator, tmp, env, o.timeout, o.layer)) if o.layer else
               (lambda f: one(f, loft, o.generator, tmp, env, o.timeout, o.form == "debug")))
        rows = 0
        with cf.ThreadPoolExecutor(o.jobs) as ex:
            for f, verdict, what in ex.map(run, files) if o.layer else ex.map(lambda f: one(f, loft, o.generator, tmp, env, o.timeout, o.form == "debug"), files):
                counts[verdict] += 1
                if o.layer and verdict == "agree":
                    rows += int(what)
                if verdict in o.show:
                    shown.append(f"{verdict}\t{os.path.relpath(f, root)}\t{what}")
                if verdict == "refused":
                    why[what] += 1
        units = len(files) - counts["nounit"]
        if o.layer:
            print(f"layer {o.layer}: {rows} function rows compared")
        print(f"{counts['agree']} of {units} units agree; " +
              ", ".join(f"{counts[k]} {k}" for k in ("differ", "refused", "nounit") if counts[k]))
        for line in sorted(shown):
            print(line)
        if why:
            print("why the second unit was not written, by programs:")
            for name, n in why.most_common(o.top):
                print(f"  {n:5d}  {name}")
        return 0 if counts["agree"] == units else 1
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
