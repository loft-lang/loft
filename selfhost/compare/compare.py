#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The strict comparison of the two interpreters (@PLN91, doc/claude/plans/91-self-host/1b-compare.md).

    selfhost/compare/compare.py FILE[::test_name] … [--layers out,alloc,…] [--keep]

For each entry — a program's `main`, or one test function — every output both implementations
produce is compared, one LAYER per line: `EQUAL`, `DIFFERENT` (the first differing row, with
context from both sides), `MISSING <side>` (that side does not produce the layer yet — a
failure, not a skip) or `REFUSED` (the loft interpreter refused the program).  The exit code is
0 only when every selected layer of every entry is `EQUAL`.  The normalisations are the ones
1b-compare.md lists, and nothing else.

The Rust side is the `op-census` build (`LOFT_BIN`, default target/op-census/release/loft), which
writes the allocator trace; the loft side is `selfhost/cover/run_program.loft`, compiled through
the C backend into target/selfhost-compare/ and rebuilt when a selfhost source is newer.  Not a
test: it is run by hand, on any program, test script or suite cell."""
import argparse, os, re, subprocess, sys, tempfile, shutil

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(ROOT, "scripts"))
from c_alloc_diff import program_events  # noqa: E402  — one normalisation for both comparators

LOFT = os.environ.get("LOFT_BIN", os.path.join(ROOT, "target", "op-census", "release", "loft"))
LAYERS = ["out", "alloc", "leak", "ir", "types", "schema", "ctable", "binds", "slots", "lifetimes", "bytecode"]
# The layers the loft side does not produce yet, and the step that builds each (1b-compare.md).
NOT_YET = {
    "ir": "C-1b, the IR display port",
    "types": "C-1b, the variable types display",
    "schema": "C-1b, the schema display",
    "ctable": "C-1b, the C type table layer",
    "binds": "C-2, with 1b-s3.md B-1",
    "slots": "C-3, with 1b-frame.md F-1",
    "lifetimes": "C-3, the compute_intervals port",
    "bytecode": "C-3, with 1b-frame.md F-2",
}
LIBS = ["--lib", os.path.join(ROOT, "tests", "lib")]


def run(cmd, env=None, timeout=300):
    try:
        p = subprocess.run(cmd, capture_output=True, env={**os.environ, **(env or {})}, timeout=timeout)
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired:
        return -9, b"", b"timeout"


def runner():
    """The loft interpreter, compiled through the C backend; rebuilt when a selfhost source is newer."""
    out = os.path.join(ROOT, "target", "selfhost-compare", "runner")
    exe = os.path.join(out, "run_program")
    newest = 0.0
    for d, _, fs in os.walk(os.path.join(ROOT, "selfhost")):
        if "native-auto" in d or os.sep + ".loft" in d:
            continue
        for f in fs:
            if f.endswith((".loft", ".c", ".toml")):
                newest = max(newest, os.path.getmtime(os.path.join(d, f)))
    if not os.path.exists(exe) or os.path.getmtime(exe) < newest:
        os.makedirs(out, exist_ok=True)
        code, _, err = run([LOFT, "--native-c", "--ship", "--out", out,
                            os.path.join(ROOT, "selfhost", "cover", "run_program.loft")], timeout=900)
        if not os.path.exists(exe) or os.path.getmtime(exe) < newest:
            sys.exit("the loft interpreter did not build:\n" + err.decode(errors="replace")[-2000:])
    return exe


def text(b):
    return b.decode("utf-8", errors="replace")


def differ(name, rust, loft):
    """`EQUAL`, or `DIFFERENT` at the first differing row with three rows of context each side."""
    if rust == loft:
        return "EQUAL", []
    i = next((k for k, (a, b) in enumerate(zip(rust, loft)) if a != b), min(len(rust), len(loft)))
    rows = [f"{name}: first difference at row {i + 1} of {len(rust)} (rust) / {len(loft)} (loft)"]
    for k in range(max(0, i - 3), i + 4):
        a = rust[k] if k < len(rust) else "-"
        b = loft[k] if k < len(loft) else "-"
        rows.append(f"  {'>' if k == i else ' '} rust {a!r}")
        rows.append(f"  {'>' if k == i else ' '} loft {b!r}")
    return "DIFFERENT", rows


HARNESS = re.compile(rb"^  (ok|FAIL) ", re.M)


def out_layer(entry_test, diag, rust, loft):
    """stdout, the exit code and stderr past the front end's diagnostics; for a test entry the
    harness block cut from the Rust stdout, its verdict compared with the runner's."""
    rc, rout, rerr = rust
    lc, lout, lerr = loft
    if not rerr.startswith(diag):
        return "DIFFERENT", ["out: the Rust run's stderr does not start with the front end's diagnostics"]
    rerr = rerr[len(diag):]
    if entry_test:
        m = HARNESS.search(rout)
        if not m:
            return "DIFFERENT", ["out: the Rust test run printed no harness line"]
        verdict = m.group(1).decode()
        rout = rout[:m.start()]
        rows = [("stdout", rout.splitlines(), lout.splitlines()),
                ("verdict", [verdict], ["ok" if lc == 0 else "FAIL"])]
        if verdict == "FAIL":
            rows.append(("message", [text(rout[m.start():])], [text(lerr)]))
    else:
        rows = [("stdout", rout.splitlines(), lout.splitlines()),
                ("exit", [str(rc)], [str(lc)]),
                ("stderr", rerr.splitlines(), lerr.splitlines())]
    for name, a, b in rows:
        v, detail = differ(f"out {name}", a, b)
        if v != "EQUAL":
            return v, detail
    return "EQUAL", []


def leaked(events):
    made, freed = [], set()
    for e in events:
        f = e.split()
        if f[0] == "N":
            made.append(f[1])
        elif f[0] == "F":
            freed.add(f[1])
    return [s for s in made if s not in freed]


def compare(spec, exe, layers, keep):
    file, _, test = spec.partition("::")
    report = []
    d = tempfile.mkdtemp(prefix="loft-compare-")
    try:
        img = os.path.join(d, "p.store")
        code, _, diag = run([LOFT, "--emit-image", img] + LIBS + [file])
        if code != 0:
            return [("all", "ERROR", ["the front end refused the program:"] + text(diag).splitlines()[-5:])]
        rtrace, ltrace = os.path.join(d, "rust.trace"), os.path.join(d, "loft.trace")
        if test:
            rcmd = [LOFT] + LIBS + ["--tests", f"{file}::{test}"]
        else:
            rcmd = [LOFT, "--interpret"] + LIBS + [file]
        rust = run(rcmd, {"LOFT_ALLOC_TRACE": rtrace})
        lcmd = [exe, "--compare", img] + (["n_" + test] if test else [])
        loft = run(lcmd, {"LOFT_CRT_ALLOC_TRACE": ltrace})
        if loft[0] == 3 and b"refused: " in loft[2]:
            why = text(loft[2]).split("refused: ", 1)[1].strip().splitlines()[0]
            return [("all", "REFUSED", [why])]
        rev = program_events(open(rtrace, encoding="utf-8")) if os.path.exists(rtrace) else None
        lev = program_events(open(ltrace, encoding="utf-8")) if os.path.exists(ltrace) else None
        for layer in layers:
            if layer == "out":
                v, detail = out_layer(bool(test), diag, rust, loft)
            elif layer in ("alloc", "leak") and rev is None:
                v, detail = "MISSING rust", [f"no allocator trace: {LOFT} is not the op-census build"]
            elif layer in ("alloc", "leak") and lev is None:
                v, detail = "MISSING loft", ["the runner wrote no allocator trace"]
            elif layer == "alloc":
                v, detail = differ("alloc", rev, lev)
            elif layer == "leak":
                v, detail = differ("leak", leaked(rev), leaked(lev))
            else:
                v, detail = "MISSING loft", [NOT_YET[layer]]
            report.append((layer, v, detail))
        return report
    finally:
        if keep:
            print(f"  (kept {d})")
        else:
            shutil.rmtree(d, ignore_errors=True)


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("entries", nargs="+", help="FILE or FILE::test_name")
    ap.add_argument("--layers", default=",".join(LAYERS))
    ap.add_argument("--keep", action="store_true", help="keep each entry's scratch directory")
    a = ap.parse_args()
    layers = [x for x in a.layers.split(",") if x]
    unknown = [x for x in layers if x not in LAYERS]
    if unknown:
        sys.exit(f"unknown layers: {', '.join(unknown)} (known: {', '.join(LAYERS)})")
    if not os.path.exists(LOFT):
        sys.exit(f"no Rust side at {LOFT}: cargo build --release --features op-census "
                 "--target-dir target/op-census --bin loft (or set LOFT_BIN)")
    exe = runner()
    equal = 0
    for spec in a.entries:
        print(f"== {spec}")
        report = compare(spec, exe, layers, a.keep)
        ok = all(v == "EQUAL" for _, v, _ in report)
        equal += ok
        for layer, v, detail in report:
            print(f"   {layer:<10} {v}")
            for row in detail:
                print(f"              {row}")
    print(f"{equal} of {len(a.entries)} entries equal in every layer ({', '.join(layers)})")
    sys.exit(0 if equal == len(a.entries) else 1)


if __name__ == "__main__":
    main()
