#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Is the bytecode the loft runner executes identical to the Rust interpreter's?

@PLN91, doc/claude/plans/91-self-host/handoff-2026-11-13-run-image.md.  For every plain program
of the coverage census's corpus (`selfhost/cover/census.py`) the run image is written with the
interpreter's own bytecode beside it (`LOFT_TRACE_BYTECODE_RAW`, the stream byte for byte, and
`LOFT_TRACE_BYTECODE`, its operator table and each function's length and FNV-1a hash).  The
loft side — selfhost/bench/run_image_check.loft, built `--native-c --ship` — reads the image
through `load::load_run` and writes back what it holds.  Three comparisons per program, each of
which must hold:

  bytes      the image's code stream equals the interpreter's, byte for byte (`cmp`)
  functions  each function's length and hash, read through the image's start table, equal the
             trace's, in definition order
  operators  the image's operator table (number and name) equals the trace's

    python3 selfhost/bench/run_image_bytecode.py [--scripts-only] [--jobs N] [--show]
"""
import argparse, concurrent.futures as cf, os, re, shutil, subprocess, sys, tempfile

sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "cover"))
import census  # noqa: E402  the corpus and the plain-program filter are the census's own


def proc(cmd, env, timeout):
    try:
        p = subprocess.run(cmd, capture_output=True, env=env, timeout=timeout)
        return p.returncode, p.stdout.decode("utf-8", "replace"), p.stderr.decode("utf-8", "replace")
    except subprocess.TimeoutExpired:
        return -9, "", "timeout"


def one(f, loft, check, tmp, env, timeout):
    base = os.path.join(tmp, re.sub(r"[^A-Za-z0-9]", "_", f))
    img, raw, trace, code = base + ".run", base + ".raw", base + ".trace", base + ".code"
    e = {**env, "LOFT_TRACE_BYTECODE": trace, "LOFT_TRACE_BYTECODE_RAW": raw}
    rc, _, err = proc([loft, "--emit-run-image", img, f], e, timeout)
    if rc != 0 or not os.path.exists(raw):
        return f, "noimage", err.strip().splitlines()[-1:] or [""]
    rc, out, err = proc([check, img, code], env, timeout)
    if rc != 0 or not os.path.exists(code):
        return f, "nocheck", [err.strip()[-200:]]
    got = out.splitlines()
    want = open(trace, encoding="utf-8").read().splitlines()
    bad = []
    if open(raw, "rb").read() != open(code, "rb").read():
        bad.append("bytes")
    if [l for l in got if l.startswith("fn\t")] != ["\t".join((x.split("\t")[0],) + tuple(x.split("\t")[2:4])) for x in want if x.startswith("fn\t")]:
        bad.append("functions")
    if [l for l in got if l.startswith("op\t")] != ["\t".join(x.split("\t")[0:3]) for x in want if x.startswith("op\t")]:
        bad.append("operators")
    for p in (img, img + ".dschema", raw, trace, code):
        try:
            os.remove(p)
        except OSError:
            pass
    return f, ("identical" if not bad else "differs"), bad


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--loft", default=None, help="the loft binary (default: target/release/loft)")
    ap.add_argument("--jobs", type=int, default=6)
    ap.add_argument("--scripts-only", action="store_true", help="tests/scripts only, not the language suite")
    ap.add_argument("--chapter", action="append", default=[], help="a suite chapter (repeatable)")
    ap.add_argument("--timeout", type=int, default=120)
    ap.add_argument("--show", action="store_true", help="list every program that is not identical")
    o = ap.parse_args()
    root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    loft = o.loft or os.path.join(root, "target", "release", "loft")
    suite = os.environ.get("LOFT_SUITE_DIR", os.path.join(os.path.dirname(root), "loft-suite"))
    env = {**os.environ, "LOFT_NO_WARNINGS": "1"}
    tmp = tempfile.mkdtemp(prefix="loft-runbytes-", dir=os.environ.get("TMPDIR"))
    try:
        out = os.path.join(tmp, "check")
        os.makedirs(out, exist_ok=True)
        p = subprocess.run([loft, "--native-c", "--ship", "--out", out,
                            "--lib", os.path.join(root, "selfhost", "src"),
                            os.path.join(root, "selfhost", "bench", "run_image_check.loft")],
                           capture_output=True, text=True, env=env, timeout=900)
        check = os.path.join(out, "run_image_check")
        if not os.path.exists(check):
            sys.exit(f"the check did not build:\n{p.stderr[-2000:]}")
        files = [f for f in census.corpus(root, suite if os.path.isdir(suite) else None,
                                          o.chapter, o.scripts_only) if census.plain(f)]
        counts, rows = {}, []
        with cf.ThreadPoolExecutor(o.jobs) as ex:
            for f, verdict, what in ex.map(lambda f: one(f, loft, check, tmp, env, o.timeout), files):
                counts[verdict] = counts.get(verdict, 0) + 1
                if verdict != "identical":
                    rows.append(f"{verdict}\t{os.path.relpath(f, root)}\t{' '.join(what)}")
        print(f"{counts.get('identical', 0)} of {len(files)} programs: the run image's bytecode is "
              f"identical to the interpreter's" +
              "".join(f"; {counts[k]} {k}" for k in ("differs", "noimage", "nocheck") if counts.get(k)))
        if o.show or counts.get("differs"):
            for r in sorted(rows):
                if o.show or r.startswith("differs"):
                    print(r)
        sys.exit(1 if counts.get("differs") else 0)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    main()
