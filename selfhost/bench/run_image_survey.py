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

    python3 selfhost/bench/run_image_survey.py [--scripts-only] [--jobs N] [--top 30]
"""
import argparse, collections, concurrent.futures as cf, os, re, shutil, subprocess, sys, tempfile

sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "cover"))
import census  # noqa: E402  the corpus and the plain-program filter are the census's own


def proc(cmd, env, timeout, cwd=None):
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, env=env, timeout=timeout, cwd=cwd)
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired:
        return -9, "", "timeout"


def one(f, loft, runner, tmp, env, timeout, written=False):
    img = os.path.join(tmp, re.sub(r"[^A-Za-z0-9]", "_", f) + ".run")
    rc, _, err = proc([loft, "--emit-run-image", img, f], env, timeout)
    if rc != 0:
        return f, "noimage", err.strip().splitlines()[-1:] or [""]
    run_env = {**env, "LOFT_NATIVE_LEAK_CHECK": "1"}
    if written:
        # The code is the bytecode writer in loft's, from the program's stored IR.
        store = img[:-4] + ".store"
        rc, _, err = proc([loft, "--emit-image", store, f], env, timeout)
        if rc != 0:
            return f, "noimage", err.strip().splitlines()[-1:] or [""]
        run_env["LOFT_RUN_PROGRAM"] = store
    rc, want, err = proc([loft, "--interpret", f], env, timeout, cwd=os.path.dirname(f))
    # The runner shares one C runtime with the program it runs, so its exit leak check
    # (`LOFT_NATIVE_LEAK_CHECK`) counts the program's stores left at exit, as the interpreter's
    # own report does: the two counts are compared too.
    rc2, got, err2 = proc([runner, img], run_env, timeout, cwd=os.path.dirname(f))
    if "stopped: not carried by the bytecode writer in loft" in err2:
        return f, "unwritten", [""]
    m = re.search(r"stopped: not carried: (\S+)", err2)
    if m:
        return f, "stopped", [m.group(1)]
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
    ap.add_argument("--chapter", action="append", default=[], help="a suite chapter (repeatable)")
    ap.add_argument("--top", type=int, default=30)
    ap.add_argument("--timeout", type=int, default=60)
    ap.add_argument("--written", action="store_true",
                    help="run the code the bytecode writer in loft writes (bgen_image.loft), not the image's")
    ap.add_argument("--show", action="append", default=[],
                    help="list the programs of a verdict (agree, stores, leak, differ, stopped, fault, noimage; repeatable)")
    o = ap.parse_args()
    root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    loft = o.loft or os.path.join(root, "target", "release", "loft")
    suite = os.environ.get("LOFT_SUITE_DIR", os.path.join(os.path.dirname(root), "loft-suite"))
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
        files = [os.path.abspath(f) for f in census.corpus(root, suite if os.path.isdir(suite) else None,
                                                           o.chapter, o.scripts_only) if census.plain(f)]
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
        with cf.ThreadPoolExecutor(o.jobs) as ex:
            for f, verdict, what in ex.map(lambda f: one(f, loft, runner, tmp, env, o.timeout, o.written), files):
                counts[verdict] += 1
                if verdict in o.show:
                    shown.append(f"{verdict}\t{os.path.relpath(f, root)}\t{what[0]}")
                if verdict == "stopped":
                    ops[what[0]] += 1
        print(f"{counts['agree']} of {len(files)} programs agree; " +
              ", ".join(f"{counts[k]} {k}" for k in ("stores", "leak", "stopped", "unwritten", "differ", "fault", "noimage") if counts[k]))
        for line in sorted(shown):
            print(line)
        if ops:
            print("first operator or native not carried, by programs it stops:")
            for name, n in ops.most_common(o.top):
                print(f"  {n:5d}  {name}")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    main()
