#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""How much of the corpus the bytecode writer in loft writes, byte for byte.

@PLN91, doc/claude/plans/91-self-host/rust-dependencies.md step 1.  For every plain program of
the coverage census's corpus (`selfhost/cover/census.py`) the Rust compiler writes its program
image and its run image; `selfhost/bench/bgen_check.loft`, built `--native-c --ship`, writes
every function from the first and compares its bytes with the second.  Printed: the functions
identical, the functions that DIFFER (a defect, each named with its program) and the ones not
written yet, with what stops them by count — the order to port in.  The standard library's
functions are in every image, so a function is counted once, by name.  Then, per program
(step 2), the whole code stream and each definition's start in it written in loft against
the image's.

    python3 selfhost/bench/bgen_survey.py [--scripts-only] [--jobs N] [--top 20]
    python3 selfhost/bench/bgen_survey.py [--functions] --check BASELINE | --write-baseline BASELINE

`--functions` surveys the corpus's test files instead (`fn test_*()` and no `main`): each test
function as a program of its own, as `run_image_survey.py --functions` runs it (the file with a
`main` calling that one test, `tests/lib` on `--lib`, emitted from the file's own directory),
under the layers `functions-tests` and `streams-tests`.

The position is gated by a committed baseline (`bgen_survey.baseline.tsv` beside this file, in
`baseline.py`'s format), two layers: `functions` (each function not identical yet, by name) and
`streams` (each program whose whole code stream and starts are not identical yet).  A function
or a stream that DIFFERS, or a program with another operator, shared-body or entry table, fails
whether or not it is listed: those are wrong bytes.  `unwritten` and `natives` (a natives table
loft's does not hold yet) are listed until they agree.
"""
import argparse, collections, concurrent.futures as cf, hashlib, os, re, shutil, subprocess, sys, tempfile

sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "cover"))
import census  # noqa: E402
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import baseline  # noqa: E402  one baseline format for every survey
import run_image_survey  # noqa: E402  the test functions are the runner survey's


def stem_of(tmp, name):
    """A scratch path for `name`; a long one keeps its end and a hash (255-byte file names)."""
    stem = re.sub(r"[^A-Za-z0-9]", "_", name)
    if len(stem) > 150:
        stem = stem[-120:] + "_" + hashlib.sha1(stem.encode()).hexdigest()[:12]
    return os.path.join(tmp, stem)


def one(f, loft, checker, tmp, env, test=None, libargs=()):
    stem = stem_of(tmp, f + (f"::{test}" if test else ""))
    prog, cwd = f, None
    if test:
        # A short name: the program's path is a text constant of it (an `assert`'s file), and
        # one of 256 bytes or more takes another constant layout than a plain program's.
        stem = os.path.join(tmp, "t" + hashlib.sha1(f"{f}::{test}".encode()).hexdigest()[:16])
        # The test file with a `main` calling the one test (run_image_survey.py's form), emitted
        # from the file's own directory, where its relative paths resolve.
        prog, cwd = stem + ".loft", os.path.dirname(f)
        with open(prog, "w", encoding="utf-8") as fh:
            fh.write(open(f, encoding="utf-8", errors="replace").read() + f"\n\nfn main() {{\n  {test}();\n}}\n")
    for flag, ext in (("--emit-image", ".store"), ("--emit-run-image", ".run")):
        try:
            p = subprocess.run([loft, *libargs, flag, stem + ext, prog], capture_output=True, text=True,
                               env=env, timeout=120, cwd=cwd)
        except subprocess.TimeoutExpired:
            return f, None
        if p.returncode != 0:
            return f, None
    try:
        p = subprocess.run([checker, stem + ".store", stem + ".run", "--names"], capture_output=True,
                           text=True, encoding="utf-8", errors="replace", env=env, timeout=300)
    except subprocess.TimeoutExpired:
        return f, None
    try:
        q = subprocess.run([checker, stem + ".store", stem + ".run", "--stream"], capture_output=True,
                           text=True, encoding="utf-8", errors="replace", env=env, timeout=300)
    except subprocess.TimeoutExpired:
        return f, p.stdout
    return f, p.stdout + q.stdout


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--jobs", type=int, default=6)
    ap.add_argument("--scripts-only", action="store_true")
    ap.add_argument("--functions", action="store_true",
                    help="the test functions of the test files, each as a program, instead of the plain programs")
    ap.add_argument("--top", type=int, default=20)
    ap.add_argument("--examples", type=int, default=0, help="name this many functions, with their program, per reason")
    ap.add_argument("--check", metavar="BASELINE", help="exit 1 on a drop against BASELINE or any wrong bytes")
    ap.add_argument("--write-baseline", metavar="BASELINE", help="write both layers' blocks of BASELINE")
    o = ap.parse_args()
    if (o.check or o.write_baseline) and o.scripts_only:
        sys.exit("--check and --write-baseline read the whole corpus: drop --scripts-only")
    root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    loft = os.path.join(root, "target", "release", "loft")
    suite = os.environ.get("LOFT_SUITE_DIR", os.path.join(os.path.dirname(root), "loft-suite"))
    pin = baseline.require_pin(root, suite, o.check) if (o.check or o.write_baseline) else None
    # The Rust writer's rewrites the loft writer does not make yet are switched off on the
    # Rust side, each by its own switch, so the bytes compared are one named form of the
    # writer: no text local is a view (`LOFT_NO_TEXT_BORROW`).
    env = {**os.environ, "LOFT_NO_WARNINGS": "1", "LOFT_NO_TEXT_BORROW": "1"}
    tmp = tempfile.mkdtemp(prefix="loft-bgen-", dir=os.environ.get("TMPDIR"))
    try:
        out = os.path.join(tmp, "checker")
        os.makedirs(out, exist_ok=True)
        p = subprocess.run([loft, "--native-c", "--ship", "--out", out,
                            "--lib", os.path.join(root, "selfhost", "src"),
                            "--lib", os.path.join(root, "selfhost", "crt", "src"),
                            os.path.join(root, "selfhost", "bench", "bgen_check.loft")],
                           capture_output=True, text=True, env=env, timeout=900)
        checker = os.path.join(out, "bgen_check")
        if not os.path.exists(checker):
            sys.exit(f"the checker did not build:\n{p.stderr[-2000:]}")
        corpus = [os.path.abspath(f) for f in census.corpus(root, suite if os.path.isdir(suite) else None,
                                                            [], o.scripts_only) if os.path.isfile(f)]
        if o.functions:
            jobs = run_image_survey.test_functions([f for f in corpus if not census.plain(f)])
            libargs = ("--lib", os.path.join(root, "tests", "lib"))
        else:
            jobs = [(f, None) for f in corpus if census.plain(f)]
            libargs = ()
        files = [f for f, _ in jobs]
        # The compiler keeps a warm cache that does not know `LOFT_NO_TEXT_BORROW`: the first
        # emission after the switch changed reads what the other setting left.  One emission
        # of each kind first, so every program's two images come from one setting.
        if files:
            for flag in ("--emit-image", "--emit-run-image"):
                subprocess.run([loft, flag, os.path.join(tmp, "warm"), files[0]], capture_output=True,
                               text=True, env=env, timeout=300)
        state = {}            # function name -> verdict, the first program's
        differ = []
        reasons = collections.Counter()
        examples = collections.defaultdict(list)
        programs = 0
        streams = collections.Counter()
        stream_differs = []
        stream_of = {}        # program -> its stream verdict
        with cf.ThreadPoolExecutor(o.jobs) as ex:
            for (src, test), (f, text) in zip(jobs, ex.map(
                    lambda j: one(j[0], loft, checker, tmp, env, j[1], libargs), jobs)):
                if text is None:
                    continue
                programs += 1
                pname = baseline.name_of(src, root, suite) + (f"::{test}" if test else "")
                for line in text.splitlines():
                    if line.startswith("stream\t"):
                        verdict = line.split("\t")[1]
                        streams[verdict] += 1
                        stream_of[pname] = verdict
                        if verdict in ("differs", "natives", "tables"):
                            stream_differs.append(f"{pname}\t{line.split(chr(9), 2)[2]}")
                        continue
                    m = re.match(r"(same|differs|unwritten)\t([^\t]+)(?:\t(.*))?$", line)
                    if not m:
                        continue
                    # A name is counted once, but a difference or a refusal in ANY program is
                    # one: two programs' functions of one name are not the same function, so a
                    # name written in one program and refused in another is not written.
                    if m.group(1) == "differs" and state.get(m.group(2)) != "differs":
                        state[m.group(2)] = "differs"
                        differ.append(f"{m.group(2)}\t{pname}")
                        continue
                    if m.group(1) == "unwritten" and state.get(m.group(2)) == "same":
                        state[m.group(2)] = "unwritten"
                        reasons[m.group(3) or ""] += 1
                        if len(examples[m.group(3) or ""]) < o.examples:
                            examples[m.group(3) or ""].append(f"{m.group(2)}  {pname}")
                        continue
                    if m.group(2) in state:
                        continue
                    state[m.group(2)] = m.group(1)
                    if m.group(1) == "unwritten":
                        reasons[m.group(3) or ""] += 1
                        if len(examples[m.group(3) or ""]) < o.examples:
                            examples[m.group(3) or ""].append(f"{m.group(2)}  {pname}")
        n = collections.Counter(state.values())
        print(f"{programs} programs, {len(state)} functions: {n['same']} identical, "
              f"{n['differs']} differ, {n['unwritten']} not written")
        print(f"whole code stream and starts: {streams['same']} programs identical, "
              f"{streams['differs']} differ, {streams['unwritten']} hold a function not written, "
              f"{streams['natives']} with another natives table than loft's, "
              f"{streams['tables']} with another operator, shared-body or entry table")
        for d in sorted(stream_differs)[:20]:
            print(f"stream differs\t{d}")
        for d in sorted(differ)[:40]:
            print(f"differs\t{d}")
        for why, k in reasons.most_common(o.top):
            print(f"  {k:5d}  {why}")
            for e in examples[why]:
                print(f"           {e}")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    sfx = "-tests" if o.functions else ""
    layers = {"functions" + sfx: (state, ("differs",)), "streams" + sfx: (stream_of, ("differs", "tables"))}
    if o.write_baseline:
        for layer, (results, _) in layers.items():
            good = sum(v == "same" for v in results.values())
            baseline.write(o.write_baseline, pin, layer, good, len(results),
                           {k: v for k, v in results.items() if v != "same"})
    if o.check:
        base = baseline.read(o.check)
        failed = False
        for layer, (results, fatal) in layers.items():
            good = sum(v == "same" for v in results.values())
            fails, repins = baseline.check(layer, base, good, len(results), results, fatal=fatal, good="same")
            try:
                baseline.report(layer, o.check, fails, repins)
            except SystemExit:
                failed = True
        if failed:
            sys.exit(1)


if __name__ == "__main__":
    main()
