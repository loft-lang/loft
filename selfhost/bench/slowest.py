#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""@PLN91 — which programs the loft interpreter runs slowest, against the Rust interpreter.

    python3 selfhost/bench/slowest.py [--loft target/release/loft] [--jobs N, default: half the cores]
                                      [--chapter operational …] [--scripts-only] [--top 25]

Both interpreters execute the same bytecode step by step, so a program's run should take about
as long on each; a program far apart is a finding — something major wrong in a step, or the
whole loft interpreter slow.  Three things are measured apart, per program:

  compile   the Rust front end's work on the program (`LOFT_TIMING` front_end + codegen); the
            loft side reads the program image the Rust front end emitted and lowers each
            function it calls to its own code (load + setup + lower: `run::Outcome.lowered_us`);
  run       each interpreter's own run of `main` — `loft --interpret`'s `run` phase
            (`LOFT_TIMING_LEDGER`) and the loft runner's (`selfhost/bench/run_timed.loft`,
            compiled once through the C backend as it ships);
  runtime   inside `run`: each side's runtime calls (text, collections, stores) are part of its
            run, so a runtime implementation far slower than its twin shows as a run ratio too.

The corpus is the coverage census's (`selfhost/cover/census.py`): tests/scripts/*.loft and the
language suite's cells under LOFT_SUITE_DIR, plain programs only.  Only a program both runs
answer the same for is ranked: a refusal, a wrong answer or a crash is the census's business.
The survey runs `--jobs` programs at a time; the `--top` largest run ratios and slowest loft runs
are then measured again one at a time on one core, best of `--repeat`, and those are the numbers
printed.  The rows go to target/selfhost-slowest.tsv.
"""
import argparse, concurrent.futures as cf, glob, os, re, shutil, statistics, subprocess, sys, tempfile

sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "cover"))
import census  # noqa: E402  the corpus and the plain-program filter are the census's own

def proc(cmd, env, timeout):
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, env=env, timeout=timeout)
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired:
        return -9, "", "timeout"

def rust_side(path, loft, env, libs, ledger, timeout, pin):
    """`loft --interpret`: (exit, stdout, compile ms, run ms)."""
    shutil.rmtree(ledger, ignore_errors=True)
    e = {**env, "LOFT_TIMING": "1", "LOFT_TIMING_LEDGER": ledger}
    code, out, err = proc(pin + [loft, "--interpret"] + libs + [path], e, timeout)
    fe = re.search(r"front_end=([\d.]+)ms", err)
    cg = re.search(r"codegen=([\d.]+)ms", err)
    run = None
    for f in glob.glob(os.path.join(ledger, "timing-*.tsv")):
        for row in open(f, encoding="utf-8"):
            cols = row.rstrip("\n").split("\t")
            if len(cols) >= 3 and cols[0] == "phase" and cols[1] == "run":
                run = float(cols[2]) * 1000.0
    compile_ms = (float(fe.group(1)) if fe else 0.0) + (float(cg.group(1)) if cg else 0.0)
    return code, out, compile_ms, run

def loft_side(path, runner, img, env, timeout, pin):
    """The loft runner on the image: (exit, stdout, load + setup + lowering ms, run ms)."""
    e = {**env, "LOFT_SOURCE_DIR": os.path.dirname(os.path.abspath(path))}
    code, out, err = proc(pin + [runner, img], e, timeout)
    m = re.search(r"timed load=(\d+) setup=(\d+) lower=(\d+) run=(-?\d+)", err)
    if not m:
        return code, out, None, None
    prep = int(m.group(1)) + int(m.group(2)) + int(m.group(3))
    return code, out, prep / 1000.0, int(m.group(4)) / 1000.0

def one(path, loft, runner, tmp, env, root, timeout, pin=()):
    img = census.path_image(path, tmp, root)
    libs = ["--lib", os.path.join(root, "tests", "lib")]
    ledger = img + ".ledger"
    try:
        code, _, _ = proc([loft, "--emit-image", img] + libs + [path], env, 300)
        if code != 0:
            return None
        rcode, rout, rcomp, rrun = rust_side(path, loft, env, libs, ledger, timeout, list(pin))
        lcode, lout, lload, lrun = loft_side(path, runner, img, env, timeout, list(pin))
        if rcode != lcode or rout != lout or rrun is None or lrun is None:
            return None
        return os.path.relpath(path, root), rcomp, rrun, lload, lrun
    finally:
        shutil.rmtree(ledger, ignore_errors=True)
        if os.path.exists(img):
            os.remove(img)

def ratio(row, floor):
    return row[4] / row[2] if row[2] > 0 and row[2] >= floor else 0.0

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--loft", default="target/release/loft")
    ap.add_argument("--jobs", type=int, default=max(1, (os.cpu_count() or 2) // 2))
    ap.add_argument("--chapter", action="append", default=[])
    ap.add_argument("--scripts-only", action="store_true")
    ap.add_argument("--top", type=int, default=25)
    ap.add_argument("--repeat", type=int, default=3)
    ap.add_argument("--floor", type=float, default=2.0,
                    help="a run ratio counts where the Rust run took at least this many ms")
    ap.add_argument("--timeout", type=int, default=300)
    ap.add_argument("--core", default="0", help="the core the re-measurement runs on")
    o = ap.parse_args()
    root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    loft = o.loft if os.path.isabs(o.loft) else os.path.join(root, o.loft)
    suite = os.environ.get("LOFT_SUITE_DIR") or os.path.join(os.path.dirname(root), "loft-suite")
    env = {**os.environ, "LOFT_NO_WARNINGS": "1"}
    tmp = tempfile.mkdtemp(prefix="loft-slowest-", dir=os.environ.get("TMPDIR"))
    try:
        out = os.path.join(tmp, "runner")
        os.makedirs(out, exist_ok=True)
        p = subprocess.run([loft, "--native-c", "--ship", "--out", out,
                            os.path.join(root, "selfhost", "bench", "run_timed.loft")],
                           capture_output=True, text=True, env=env, timeout=900)
        runner = os.path.join(out, "run_timed")
        if not os.path.exists(runner):
            sys.exit(f"the runner did not build:\n{p.stderr[-2000:]}")
        files = [f for f in census.corpus(root, suite if os.path.isdir(suite) else None,
                                           o.chapter, o.scripts_only) if census.plain(f)]
        rows = []
        with cf.ThreadPoolExecutor(o.jobs) as ex:
            for r in ex.map(lambda f: one(f, loft, runner, tmp, env, root, o.timeout), files):
                if r:
                    rows.append(r)
        # The outliers again, one at a time on one core, best of `--repeat`.
        pin = ["taskset", "-c", o.core] if shutil.which("taskset") else []
        suspects = {r[0] for r in sorted(rows, key=lambda r: -ratio(r, o.floor))[:o.top]}
        suspects |= {r[0] for r in sorted(rows, key=lambda r: -r[4])[:o.top]}
        again = {}
        for prog in sorted(suspects):
            best = None
            for _ in range(o.repeat):
                r = one(os.path.join(root, prog), loft, runner, tmp, env, root, o.timeout, pin)
                if r and (best is None or r[2] + r[4] < best[2] + best[4]):
                    best = r
            if best:
                again[prog] = best
        rows = [again.get(r[0], r) for r in rows]
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    os.makedirs(os.path.join(root, "target"), exist_ok=True)
    tsv = os.path.join(root, "target", "selfhost-slowest.tsv")
    with open(tsv, "w", encoding="utf-8") as f:
        f.write("program\trust_compile_ms\trust_run_ms\tloft_compile_ms\tloft_run_ms\trun_ratio\n")
        for prog, rc, rr, ll, lr in sorted(rows, key=lambda x: -x[4]):
            f.write(f"{prog}\t{rc:.2f}\t{rr:.3f}\t{ll:.2f}\t{lr:.3f}\t"
                    f"{lr / rr if rr else 0:.2f}\n")
    counted = [r for r in rows if r[2] > 0 and r[2] >= o.floor]
    med = statistics.median([r[4] / r[2] for r in counted]) if counted else 0.0
    print(f"{len(rows)} of {len(files)} programs run alike on both interpreters; rows in {tsv}")
    print(f"run, loft against Rust: median {med:.2f}x over the {len(counted)} runs of "
          f"{o.floor:g} ms or more on the Rust side")
    def show(title, sel):
        print(f"\n{title}")
        print(f"  {'ratio':>7} {'loft run':>10} {'rust run':>10} {'rust cmp':>9} {'loft cmp':>9}  program")
        for prog, rc, rr, ll, lr in sel:
            print(f"  {lr / rr if rr else 0:6.1f}x {lr:9.2f}ms {rr:9.2f}ms {rc:8.1f}ms "
                  f"{ll:8.1f}ms  {prog}")
    show(f"The {o.top} largest run ratios (the loft run against the Rust run):",
         sorted(counted, key=lambda r: -(r[4] / r[2]))[:o.top])
    show(f"The {o.top} slowest runs on the loft interpreter:",
         sorted(rows, key=lambda r: -r[4])[:o.top])
    show(f"The {o.top} slowest to compile (the Rust front end):",
         sorted(rows, key=lambda r: -r[1])[:o.top])

if __name__ == "__main__":
    main()
