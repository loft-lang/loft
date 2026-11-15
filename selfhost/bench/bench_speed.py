#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""@PLN91 — the bytecode runner against `loft --interpret` on the benchmark suite (bench/NN_*).

Each benchmark times its own routines and prints a row per routine (bench/README.md), so a
row's microseconds exclude start-up on both sides.  For every `bench/*/bench.loft`: its run
image is written (`loft --emit-run-image`), run on the runner built `--native-c --ship` from
selfhost/bench/run_image_run.loft and on `loft --interpret`, each `--repeat` times on one core,
the best time per routine kept.  A routine is ranked only where both print the same hash;
`differ`, `stopped` and `fault` are named instead.

    python3 selfhost/bench/bench_speed.py [--runner DIR] [--repeat 3] [--only 03 21 …] [--timeout 300]

`--runner DIR` reuses a runner built there (DIR/run_image_run) and builds it when absent.
The geometric mean of the ratios is the last line; under 1.00 the runner is the faster one.
"""
import argparse, glob, math, os, shutil, subprocess, sys, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def rows(out):
    """routine -> (us, hash) from a benchmark's rows."""
    got = {}
    lines = out.splitlines()
    for i, line in enumerate(lines):
        if line.startswith("routine\t") and i + 1 < len(lines):
            f = lines[i + 1].split("\t")
            if len(f) >= 7 and f[2].isdigit():
                got[f[0]] = (int(f[2]), f[6])
    return got


def run(cmd, env, timeout, cwd):
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, env=env, timeout=timeout, cwd=cwd)
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired:
        return -9, "", "timeout"


def best(cmd, env, timeout, cwd, repeat):
    """The best time per routine over `repeat` runs; the reason where a run did not end."""
    kept = {}
    for _ in range(repeat):
        rc, out, err = run(cmd, env, timeout, cwd)
        if rc != 0:
            why = [l for l in err.splitlines() if l.startswith("stopped:")] or err.strip().splitlines()[-1:] or [f"exit {rc}"]
            return None, why[0][:160]
        for name, (us, h) in rows(out).items():
            if name not in kept or us < kept[name][0]:
                kept[name] = (us, h)
    return kept, ""


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--loft", default=os.path.join(ROOT, "target/release/loft"))
    ap.add_argument("--runner")
    ap.add_argument("--repeat", type=int, default=3)
    ap.add_argument("--only", nargs="*", default=[])
    ap.add_argument("--timeout", type=int, default=300)
    a = ap.parse_args()
    tmp = tempfile.mkdtemp(prefix="loft-bench-speed-")
    out = a.runner or os.path.join(tmp, "runner")
    env = {**os.environ, "LOFT_NO_WARNINGS": "1"}
    if not os.path.exists(os.path.join(out, "run_image_run")):
        rc, o, e = run([a.loft, "--native-c", "--ship", "--out", out, "--lib", "src", "--lib", "crt/src", "bench/run_image_run.loft"],
                       env, 900, os.path.join(ROOT, "selfhost"))
        if not os.path.exists(os.path.join(out, "run_image_run")):
            print((o + e)[-2000:])
            return 2
    pin = ["taskset", "-c", "3"] if shutil.which("taskset") else []
    ratios = []
    for prog in sorted(glob.glob(os.path.join(ROOT, "bench", "[0-9][0-9]_*", "bench.loft"))):
        name = os.path.basename(os.path.dirname(prog))
        if a.only and not any(name.startswith(o) for o in a.only):
            continue
        img = os.path.join(tmp, name + ".run")
        rc, _, err = run([a.loft, "--emit-run-image", img, prog], env, a.timeout, None)
        if rc != 0:
            print(f"{name}: noimage {err.strip().splitlines()[-1:]}")
            continue
        cwd = os.path.dirname(prog)
        rust, why = best(pin + [a.loft, "--interpret", prog], env, a.timeout, cwd, a.repeat)
        if rust is None:
            print(f"{name}: loft --interpret did not end: {why}")
            continue
        mine, why = best(pin + [os.path.join(out, "run_image_run"), img], env, a.timeout, cwd, a.repeat)
        if mine is None:
            print(f"{name}: runner did not end: {why}")
            continue
        for routine, (us, h) in rust.items():
            if routine not in mine:
                print(f"{name}/{routine}: no row from the runner")
            elif mine[routine][1] != h:
                print(f"{name}/{routine}: differ ({h} / {mine[routine][1]})")
            else:
                r = mine[routine][0] / max(us, 1)
                ratios.append(r)
                print(f"{name}/{routine}: {us} / {mine[routine][0]} µs = {r:.2f}")
        sys.stdout.flush()
    if ratios:
        print(f"{len(ratios)} routines, geometric mean {math.exp(sum(math.log(max(r, 1e-9)) for r in ratios) / len(ratios)):.2f}, worst {max(ratios):.2f}")
    if not a.runner:
        shutil.rmtree(tmp, ignore_errors=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
