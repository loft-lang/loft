#!/usr/bin/env python3
"""ps.py N BIN... — per-op instructions/cycles from runs at N and 2N (setup cancels), pinned CPU."""
import subprocess, sys, os
CPU = os.environ.get("CPU", "0"); R = os.environ.get("R", "3")
PKG = os.path.dirname(os.path.abspath(__file__)) + "/pkg"
def run(b, n):
    env = dict(os.environ, LC_ALL="C")
    p = subprocess.run(["taskset", "-c", CPU, "perf", "stat", "-x,", "-e", "instructions:u,cycles:u", "-r", R,
                        "../" + b, "--n", str(n)], cwd=PKG, capture_output=True, text=True, env=env)
    v = {}
    for l in p.stderr.splitlines():
        f = l.split(",")
        if len(f) > 2 and f[0].replace(".", "").isdigit():
            v[f[2].split("/")[1] if "/" in f[2] else f[2].split(":")[0]] = float(f[0])
    row = p.stdout.splitlines()[1].split("\t")
    return v["instructions"], v["cycles"], int(row[3]), row[6]
n = int(sys.argv[1])
for b in sys.argv[2:]:
    i1, c1, ns1, h = run(b, n); i2, c2, ns2, h2 = run(b, 2 * n)
    print(f"{b:24s} instr/op {(i2-i1)/n/1e6:9.3f} M  cyc/op {(c2-c1)/n/1e6:9.3f} M  ns/op {ns2:>10}  hash {h}{'' if h==h2 else ' MISMATCH'}")
