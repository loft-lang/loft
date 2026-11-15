#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Which programs the loft bytecode runner runs, and what stops the rest.

@PLN91, doc/claude/plans/91-self-host/1b-fill-port.md.  For every plain program of the coverage
census's corpus (`selfhost/cover/census.py`: tests/scripts and the language suite): its run
image is written (`loft --emit-run-image`), run on the runner built `--native-c --ship` from
selfhost/bench/run_image_run.loft, and its standard output compared with `loft --interpret`'s.
A program is counted as `agree`; `stores` (equal but for a `store_memory()` report: the store
table is the C runtime's); `leak` (the same output, another count of stores left at exit than
the interpreter reports); `differ`; `stopped` (an operator or native not carried, named in the
ranking); `fault` (the run stopped for another reason, or crashed); `noimage`.

`--functions` runs the corpus's test files too (`fn test_*()` and no `main`): each test function
as a program of its own — the file with a `main` calling that one test, written to a scratch
directory and run from the file's own directory, `tests/lib` on `--lib` — so a run image, which
starts at `main`, covers what `loft --tests` runs.

    python3 selfhost/bench/run_image_survey.py [--scripts-only] [--functions] [--jobs N] [--top 30]
    python3 selfhost/bench/run_image_survey.py [--written] [--functions] --check BASELINE
    python3 selfhost/bench/run_image_survey.py [--written] [--functions] --write-baseline BASELINE

The position is gated by a committed baseline (`run_image_survey.baseline.tsv` beside this file,
in `baseline.py`'s format):
per LAYER — `image` (the run image's own code), `written` (the code the loft writer writes),
each with `-functions` when the test functions run too — the agree count and every program
that does not agree yet, with its verdict.  `--write-baseline` replaces its own layer's block
and keeps the others; `--check` exits 1 on a DROP (a program the block does not list stops
agreeing, a program new to the corpus does not agree, or the agree count falls) and on any
`differ`, which is a wrong answer whether or not it is listed.  A listed program that agrees now
is printed as a re-pin and does not fail: the baseline moves only in the commit whose change
moves it, with the reason in the message.  Both need the whole corpus (no `--chapter`,
`--scripts-only`) and the language suite at the commit `tests/loft-suite.pin` names, which the
baseline records: a suite checkout at another commit is another corpus, and is refused by name.
"""
import argparse, collections, concurrent.futures as cf, hashlib, os, re, shutil, subprocess, sys, tempfile

sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "cover"))
import census  # noqa: E402  the corpus and the plain-program filter are the census's own
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import baseline  # noqa: E402  one baseline format for every survey


def proc(cmd, env, timeout, cwd=None):
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, env=env, timeout=timeout, cwd=cwd)
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired:
        return -9, "", "timeout"


TEST_FN = re.compile(r"^fn (test\w*)\s*\(\s*\)", re.M)


def test_functions(files):
    """(file, test) for every test function of the files that have no `main`."""
    out = []
    for f in files:
        src = open(f, encoding="utf-8", errors="replace").read()
        if census.SKIP.search(src) or re.search(r"^\s*fn main\s*\(", src, re.M):
            continue
        out += [(f, t) for t in TEST_FN.findall(src)]
    return out


def one(f, loft, runner, tmp, env, timeout, written=False, test=None, libargs=()):
    src_dir = os.path.dirname(f)
    # A temporary directory of its own: the test functions of one file run at once here, where
    # `loft test` runs them one after another, and a test that keeps a fixed file under
    # `temp_dir()` (a trace, a scratch store) would read another test's.
    job_tmp = tempfile.mkdtemp(prefix="job-", dir=tmp)
    env = {**env, "TMPDIR": job_tmp}
    if test:
        # The test file with a `main` that calls the one test, beside the other programs; it runs
        # from the test file's own directory, where its relative paths resolve.
        # A file name past the file system's 255 bytes is no file: a long one keeps its end and a hash.
        stem = re.sub(r"[^A-Za-z0-9]", "_", f"{f}::{test}")
        if len(stem) > 150:
            stem = stem[-120:] + "_" + hashlib.sha1(stem.encode()).hexdigest()[:12]
        prog = os.path.join(tmp, stem + ".loft")
        with open(prog, "w", encoding="utf-8") as fh:
            fh.write(open(f, encoding="utf-8", errors="replace").read() + f"\n\nfn main() {{\n  {test}();\n}}\n")
        f = prog
    # A file name past the file system's 255 bytes is no file (a long suite cell under a deep
    # checkout): a long stem keeps its end and a hash, as a test function's program does.
    stem = re.sub(r"[^A-Za-z0-9]", "_", f)
    if len(stem) > 150:
        stem = stem[-120:] + "_" + hashlib.sha1(stem.encode()).hexdigest()[:12]
    img = f[:-5] + ".run" if test else os.path.join(tmp, stem + ".run")
    rc, _, err = proc([loft, *libargs, "--emit-run-image", img, f], env, timeout, cwd=src_dir)
    if rc != 0:
        return f, "noimage", err.strip().splitlines()[-1:] or [""]
    run_env = {**env, "LOFT_NATIVE_LEAK_CHECK": "1"}
    if written:
        # The code is the bytecode writer in loft's, from the program's stored IR.
        store = img[:-4] + ".store"
        rc, _, err = proc([loft, *libargs, "--emit-image", store, f], env, timeout, cwd=src_dir)
        if rc != 0:
            return f, "noimage", err.strip().splitlines()[-1:] or [""]
        run_env["LOFT_RUN_PROGRAM"] = store
    rc, want, err = proc([loft, *libargs, "--interpret", f], env, timeout, cwd=src_dir)
    # The runner shares one C runtime with the program it runs, so its exit leak check
    # (`LOFT_NATIVE_LEAK_CHECK`) counts the program's stores left at exit, as the interpreter's
    # own report does: the two counts are compared too.
    rc2, got, err2 = proc([runner, img], run_env, timeout, cwd=src_dir)
    if "stopped: not carried by the bytecode writer in loft" in err2:
        return f, "unwritten", [""]
    m = re.search(r"stopped: not carried: ([^\n]+)", err2)
    if m:
        return f, "stopped", [m.group(1).strip()[:100]]
    if rc2 == -9:
        return f, "fault", ["timeout"]
    if rc2 not in (0, rc):
        m = re.search(r"stopped: (.*)", err2)
        return f, "fault", [(m.group(1) if m else f"exit {rc2}")[:90]]
    if got != want:
        if STORE_REPORT.sub("", got) == STORE_REPORT.sub("", want):
            return f, "stores", [""]
        return f, "differ", [""]
    leaked = [int(m.group(1)) if m else 0
              for m in (re.search(r"(\d+) stores not freed at program exit", e) for e in (err, err2))]
    if leaked[0] != leaked[1]:
        return f, "leak", [f"interpreter {leaked[0]}, runner {leaked[1]}"]
    return f, "agree", [""]


STORE_REPORT = re.compile(r"^(stores: \d+ live|  #\d+ ).*\n", re.M)


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--loft", default=None, help="the loft binary (default: target/release/loft)")
    ap.add_argument("--jobs", type=int, default=6)
    ap.add_argument("--scripts-only", action="store_true", help="tests/scripts only, not the language suite")
    ap.add_argument("--functions", action="store_true",
                    help="also every test function of the test files, each as a program of its own")
    ap.add_argument("--chapter", action="append", default=[], help="a suite chapter (repeatable)")
    ap.add_argument("--top", type=int, default=30)
    ap.add_argument("--timeout", type=int, default=60)
    ap.add_argument("--written", action="store_true",
                    help="run the code the bytecode writer in loft writes (bgen_image.loft), not the image's")
    ap.add_argument("--show", action="append", default=[],
                    help="list the programs of a verdict (agree, stores, leak, differ, stopped, fault, noimage; repeatable)")
    ap.add_argument("--check", metavar="BASELINE", help="exit 1 on a drop against BASELINE or any differ")
    ap.add_argument("--write-baseline", metavar="BASELINE", help="write this layer's block of BASELINE")
    o = ap.parse_args()
    if (o.check or o.write_baseline) and (o.chapter or o.scripts_only):
        sys.exit("--check and --write-baseline read the whole corpus: drop --chapter and --scripts-only")
    layer = ("written" if o.written else "image") + ("-functions" if o.functions else "")
    root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    loft = o.loft or os.path.join(root, "target", "release", "loft")
    suite = os.environ.get("LOFT_SUITE_DIR", os.path.join(os.path.dirname(root), "loft-suite"))
    if o.check or o.write_baseline:
        pin = baseline.require_pin(root, suite, o.check)
    env = {**os.environ, "LOFT_NO_WARNINGS": "1"}
    if o.written:
        # The loft writer holds every text local as its own text; the image's tables are
        # written for that same form.
        env["LOFT_NO_TEXT_BORROW"] = "1"
    tmp = tempfile.mkdtemp(prefix="loft-runimg-", dir=os.environ.get("TMPDIR"))
    try:
        out = os.path.join(tmp, "runner")
        os.makedirs(out, exist_ok=True)
        p = subprocess.run([loft, "--native-c", "--ship", "--out", out,
                            "--lib", os.path.join(root, "selfhost", "src"),
                            "--lib", os.path.join(root, "selfhost", "crt", "src"),
                            os.path.join(root, "selfhost", "bench", "run_image_run.loft")],
                           capture_output=True, text=True, env=env, timeout=900)
        runner = os.path.join(out, "run_image_run")
        if not os.path.exists(runner):
            sys.exit(f"the runner did not build:\n{p.stderr[-2000:]}")
        # Absolute: each program runs from its own directory, where a path relative to the
        # working directory (a relative LOFT_SUITE_DIR) does not resolve.
        corpus = [os.path.abspath(f) for f in census.corpus(root, suite if os.path.isdir(suite) else None,
                                                            o.chapter, o.scripts_only) if os.path.isfile(f)]
        files = [f for f in corpus if census.plain(f)]
        jobs = [(f, None) for f in files]
        if o.functions:
            jobs += test_functions([f for f in corpus if not census.plain(f)])
        libargs = ("--lib", os.path.join(root, "tests", "lib"))
        # The compiler keeps a warm cache that does not know `LOFT_NO_TEXT_BORROW`: the first
        # emission after the switch changed reads what the other setting left.  One emission
        # of each kind first, so every program's two images come from one setting.
        if files:
            for flag in ("--emit-image", "--emit-run-image"):
                subprocess.run([loft, flag, os.path.join(tmp, "warm"), files[0]], capture_output=True,
                               text=True, env=env, timeout=300)
        counts = collections.Counter()
        ops = collections.Counter()
        shown = []
        results = {}
        with cf.ThreadPoolExecutor(o.jobs) as ex:
            for (src, test), (f, verdict, what) in zip(jobs, ex.map(
                    lambda j: one(j[0], loft, runner, tmp, env, o.timeout, o.written, j[1], libargs), jobs)):
                counts[verdict] += 1
                name = baseline.name_of(src, root, suite) + (f"::{test}" if test else "")
                results[name] = verdict
                if verdict in o.show:
                    shown.append(f"{verdict}\t{name}\t{what[0]}")
                if verdict == "stopped":
                    ops[what[0]] += 1
        print(f"{counts['agree']} of {len(jobs)} programs agree; " +
              ", ".join(f"{counts[k]} {k}" for k in ("stores", "leak", "stopped", "unwritten", "differ", "fault", "noimage") if counts[k]))
        for line in sorted(shown):
            print(line)
        if ops:
            print("first operator or native not carried, by programs it stops:")
            for name, n in ops.most_common(o.top):
                print(f"  {n:5d}  {name}")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    if o.write_baseline:
        baseline.write(o.write_baseline, pin, layer, counts["agree"], len(jobs),
                       {p: v for p, v in results.items() if v != "agree"})
    if o.check:
        fails, repins = baseline.check(layer, baseline.read(o.check), counts["agree"], len(jobs), results)
        baseline.report(layer, o.check, fails, repins)

if __name__ == "__main__":
    main()
