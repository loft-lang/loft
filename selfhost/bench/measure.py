#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""@PLN91 S1.4 — the loft interpreter against the Rust interpreter, on the S1 kernels.

    python3 selfhost/bench/measure.py [--loft target/release/loft] [--runs 5]

Rust side: `loft --interpret kernels/<k>.loft`, whose `main` times its own call.  Loft side:
the kernel's program image run by `selfhost/bench/run_kernel.loft`, compiled through the C
backend as it ships (`--native-c --ship`), which times the run apart from the image load.
Best of `--runs` per side; the two answers must agree or the row is not a measurement.  A
ratio over 3x is printed as such — the owner's line for telling the performance stream.
"""
import argparse, os, subprocess, sys, tempfile

KERNELS = [("fib", "n_fib", ["30"]), ("mix", "n_mix", ["5000000", "1"]),
           ("collatz", "n_longest", ["1", "100000"])]
LINE = 3.0

def run(cmd, env):
    """Run `cmd`; answer the fields of its last stdout line."""
    out = subprocess.run(cmd, capture_output=True, text=True, env=env, timeout=1200)
    lines = out.stdout.strip().splitlines()
    if out.returncode != 0 or not lines:
        sys.exit(f"failed: {' '.join(cmd)}\n{out.stdout[-1000:]}\n{out.stderr[-2000:]}")
    return lines[-1].split()

def emit(cmd, env):
    """Run a command that prints nothing on success."""
    out = subprocess.run(cmd, capture_output=True, text=True, env=env, timeout=1200)
    if out.returncode != 0:
        sys.exit(f"failed: {' '.join(cmd)}\n{out.stderr[-2000:]}")

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--loft", default="target/release/loft")
    ap.add_argument("--runs", type=int, default=5)
    o = ap.parse_args()
    root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    loft = os.path.join(root, o.loft) if not os.path.isabs(o.loft) else o.loft
    env = dict(os.environ, LOFT_NO_CACHE="1")
    tmp = tempfile.mkdtemp(prefix="loft-s14-", dir=os.environ.get("TMPDIR"))
    kdir = os.path.join(root, "selfhost", "kernels")
    driver = os.path.join(root, "selfhost", "bench", "run_kernel.loft")
    print(f"{'kernel':10} {'answer':>10} {'rust us':>10} {'loft us':>10} {'ratio':>7}  load us")
    worst = 0.0
    for k, entry, args in KERNELS:
        src = os.path.join(kdir, f"{k}.loft")
        image = os.path.join(tmp, f"{k}.store")
        emit([loft, "--emit-image", image, src], env)
        rust = [run([loft, "--interpret", src], env) for _ in range(o.runs)]
        mine = [run([loft, "--native-c", "--ship", driver, image, entry] + args, env)
                for _ in range(o.runs)]
        answers = {r[0] for r in rust} | {m[0] for m in mine}
        if len(answers) != 1:
            sys.exit(f"{k}: the two sides disagree: {sorted(answers)}")
        r_us = min(int(r[1]) for r in rust)
        m_us = min(int(m[1]) for m in mine)
        load_us = min(int(m[2]) for m in mine)
        ratio = m_us / max(r_us, 1)
        worst = max(worst, ratio)
        flag = "  OVER 3x" if ratio > LINE else ""
        print(f"{k:10} {answers.pop():>10} {r_us:>10} {m_us:>10} {ratio:>6.2f}x  {load_us}{flag}")
    print(f"worst ratio {worst:.2f}x — {'over' if worst > LINE else 'within'} the {LINE:.0f}x line")

if __name__ == "__main__":
    main()
