#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""@PLN91 — the generated operator table over the corpus: every plain program of tests/scripts
run by `selfhost/bench/fill_main.loft` from its run image, its output compared with
`loft --interpret`'s.

    python3 selfhost/bench/fill_survey.py [--jobs N] [--top N] [--show agree|differ|stopped|fault]

Prints how many programs agree, and what stops the others: the first operator or native not
carried, by the programs it stops (the worklist of 1b-fill-port.md).
"""
import argparse, collections, concurrent.futures as cf, glob, os, re, shutil, subprocess, sys, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def run(cmd, cwd=None, env=None, timeout=120):
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, cwd=cwd, env=env, timeout=timeout)
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired:
        return -9, "", "timeout"


def one(f, loft, runner, tmp):
    img = os.path.join(tmp, re.sub(r"[^A-Za-z0-9]", "_", f) + ".run")
    env = {**os.environ, "LOFT_NO_CACHE": "1"}
    rc, _, _ = run([loft, "--emit-run-image", img, f], env=env)
    if rc != 0:
        return f, "noimage", ""
    rc, want, _ = run([loft, "--interpret", f], cwd=os.path.dirname(f), env=env)
    rc2, got, err = run([runner, img], cwd=os.path.dirname(f), env=env)
    m = re.search(r"stopped: not carried: (\S+)", err)
    if m:
        return f, "stopped", m.group(1)
    if rc2 == -9:
        return f, "fault", "timeout"
    if rc2 not in (0, rc):
        m = re.search(r"stopped: (.*)", err)
        return f, "fault", (m.group(1) if m else f"exit {rc2}")[:80]
    return f, ("agree" if got == want else "differ"), ""


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--jobs", type=int, default=6)
    ap.add_argument("--top", type=int, default=25)
    ap.add_argument("--show", action="append", default=[])
    o = ap.parse_args()
    loft = os.path.join(ROOT, "target", "release", "loft")
    tmp = tempfile.mkdtemp(prefix="loft-fill-survey-")
    try:
        out = os.path.join(tmp, "runner")
        p = subprocess.run([loft, "--native-c", "--ship", "--out", out, "--lib", "src", "--lib", "crt/src",
                            "bench/fill_main.loft"], capture_output=True, text=True,
                           cwd=os.path.join(ROOT, "selfhost"))
        runner = os.path.join(out, "fill_main")
        if not os.path.exists(runner):
            sys.exit(f"the runner did not build:\n{p.stderr[-2000:]}")
        files = sorted(f for f in glob.glob(os.path.join(ROOT, "tests", "scripts", "*.loft"))
                       if re.search(r"^fn main\(\)", open(f, encoding="utf-8").read(), re.M))
        counts, stops, shown = collections.Counter(), collections.Counter(), []
        with cf.ThreadPoolExecutor(o.jobs) as ex:
            for f, verdict, what in ex.map(lambda f: one(f, loft, runner, tmp), files):
                counts[verdict] += 1
                if verdict == "stopped":
                    stops[what] += 1
                if verdict in o.show:
                    shown.append(f"{verdict}\t{os.path.relpath(f, ROOT)}\t{what}")
        print(f"{counts['agree']} of {len(files)} programs agree; " +
              ", ".join(f"{counts[k]} {k}" for k in ("stopped", "differ", "fault", "noimage") if counts[k]))
        for line in sorted(shown):
            print(line)
        if stops:
            print("first operator or native not carried, by programs it stops:")
            for name, n in stops.most_common(o.top):
                print(f"  {n:5d}  {name}")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    main()
