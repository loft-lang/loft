#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Which programs the run-image runner runs, and which operator stops the rest.

@PLN91, doc/claude/plans/91-self-host/handoff-2026-11-13-run-image.md step 3.  For every plain
program of the coverage census's corpus (`selfhost/cover/census.py`): its run image is written
(`loft --emit-run-image`), run on the runner built `--native-c --ship` from
selfhost/bench/run_image_run.loft, and its standard output compared with `loft --interpret`'s.
A program is counted as `agree`, `differ` (the runner printed something else), `refused` (an
operator it does not carry, named in the ranking) or `crash`.  The ranking is the order to add
operators in.

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


def one(f, loft, runner, tmp, env, timeout):
    img = os.path.join(tmp, re.sub(r"[^A-Za-z0-9]", "_", f) + ".run")
    rc, _, err = proc([loft, "--emit-run-image", img, f], env, timeout)
    if rc != 0:
        return f, "noimage", err.strip().splitlines()[-1:] or [""]
    rc, want, _ = proc([loft, "--interpret", f], env, timeout, cwd=os.path.dirname(f))
    rc2, got, err2 = proc([runner, img], env, timeout, cwd=os.path.dirname(f))
    m = re.search(r"refused: (.*): (.+?) at \d+", err2)
    if rc2 == 3 and m:
        # An operator or a native by name; a collection the runner does not add to by its kind.
        what = m.group(2) if re.fullmatch(r"[\w#]+", m.group(2)) else m.group(1)
        return f, "refused", [what]
    if rc2 not in (0, 1) and rc2 != rc:
        return f, "crash", [f"exit {rc2}"]
    return f, ("agree" if got == want else "differ"), [""]


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--loft", default=None, help="the loft binary (default: target/release/loft)")
    ap.add_argument("--jobs", type=int, default=6)
    ap.add_argument("--scripts-only", action="store_true", help="tests/scripts only, not the language suite")
    ap.add_argument("--chapter", action="append", default=[], help="a suite chapter (repeatable)")
    ap.add_argument("--top", type=int, default=30)
    ap.add_argument("--timeout", type=int, default=60)
    ap.add_argument("--show", action="append", default=[],
                    help="list the programs of a verdict (agree, differ, refused, crash; repeatable)")
    o = ap.parse_args()
    root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    loft = o.loft or os.path.join(root, "target", "release", "loft")
    suite = os.environ.get("LOFT_SUITE_DIR", os.path.join(os.path.dirname(root), "loft-suite"))
    env = {**os.environ, "LOFT_NO_WARNINGS": "1"}
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
        counts = collections.Counter()
        ops = collections.Counter()
        shown = []
        with cf.ThreadPoolExecutor(o.jobs) as ex:
            for f, verdict, what in ex.map(lambda f: one(f, loft, runner, tmp, env, o.timeout), files):
                counts[verdict] += 1
                if verdict in o.show:
                    shown.append(f"{verdict}\t{os.path.relpath(f, root)}\t{what[0]}")
                if verdict == "refused":
                    ops[what[0]] += 1
        print(f"{counts['agree']} of {len(files)} programs agree; " +
              ", ".join(f"{counts[k]} {k}" for k in ("refused", "differ", "crash", "noimage") if counts[k]))
        for line in sorted(shown):
            print(line)
        if ops:
            print("first operator not carried, by programs it stops:")
            for name, n in ops.most_common(o.top):
                print(f"  {n:5d}  {name}")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    main()
