#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""How long loft's own front end takes, by phase, on a fixed corpus (@PLN166 A2).

    python3 bench/frontend/frontend.py                    every size, backend and mode
    python3 bench/frontend/frontend.py --sizes tiny,medium --backends interpret --runs 9
    python3 bench/frontend/frontend.py --tsv out.tsv      keep the run, to compare with the next
    python3 bench/frontend/frontend.py --self-test        the harness must see a slowdown it is handed

Three modes, the three a user meets:

    cold   no cache at all (LOFT_NO_CACHE=1) — the first run on a machine
    warm   the whole-program cache answers an unchanged file
    edit   the edit loop: the file changed since the last run, so the program cache misses
           and only the stdlib part can be reused — the cost a developer pays per save

On `--native`, `cold` still reuses the compiled-binary cache (LOFT_NO_CACHE governs the
program cache only), so the native number that includes rustc is `edit`.

`warm` and `edit` set LOFT_PROGRAM_CACHE=1, because a binary under `target/` disables the
cache by design (src/cache.rs `running_a_dev_build`) and would otherwise measure a cold run
under a warm name.  An `edit` run appends a comment unique to that run, and the harness
refuses to report an edit run whose input hash repeats: a repeated input is a cache hit.

The modes are measured INTERLEAVED, run by run, so load drifting on a shared machine hits
all of them alike; `--counts` adds each run's instruction count, which load barely moves.
Times are wall clock, the median of `--runs`; the phase split comes from LOFT_TIMING and is
reported for the interpreter, whose front end is the whole of its compile.  It is a REPORT:
wall time varies with the machine and its load, so nothing here gates.  The count-based
gate that does is @PLN166 C1.

The corpus is generated, not stored: CORPUS_VERSION names it, and a change to the generator
bumps the version so two reports of different corpora are never compared.
"""
from __future__ import annotations

import argparse
import hashlib
import os
import re
import statistics
import subprocess
import sys
import tempfile
import time

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
CORPUS_VERSION = 1
SIZES = {"tiny": 0, "medium": 200, "large": 800}   # units; a unit is ~15 lines
PHASES = ("parse_default", "parse_user", "scopes", "lints", "front_end", "codegen")


# ── the corpus ───────────────────────────────────────────────────────────────

def unit(i: int) -> str:
    return f"""
struct P{i} {{ x: integer, y: integer, name: text }}

fn make{i}(n: integer) -> P{i} {{
  P{i} {{ x: n, y: n * 2, name: "p{i}" }}
}}

fn sum{i}(v: vector<P{i}>) -> integer {{
  t = 0;
  for p in v {{ t += p.x + p.y; }}
  t
}}

fn label{i}(p: P{i}) -> text {{
  if p.x > 10 {{ "{{p.name}}:big" }} else {{ "{{p.name}}:small" }}
}}
"""


def corpus(size: str) -> str:
    n = SIZES[size]
    body = "".join(unit(i) for i in range(n))
    calls = "".join(f"  total += sum{i}([make{i}({i}), make{i}({i + 1})]);\n"
                    for i in range(min(n, 20)))
    return (f"// frontend bench corpus v{CORPUS_VERSION}, size {size}\n{body}\n"
            f"fn main() {{\n  total = 0;\n{calls}  println(\"total {{total}}\");\n}}\n")


# ── one run ──────────────────────────────────────────────────────────────────

def run_once(loft, path, backend, mode, extra_env=None, counts=False):
    env = dict(os.environ)
    env.pop("LOFT_NO_CACHE", None)
    env.pop("LOFT_PROGRAM_CACHE", None)
    env["LOFT_TIMING"] = "1"
    env["LOFT_TIMEOUT"] = "300"
    if mode == "cold":
        env["LOFT_NO_CACHE"] = "1"
    else:
        env["LOFT_PROGRAM_CACHE"] = "1"
    env.update(extra_env or {})
    args = [loft, "--interpret" if backend == "interpret" else "--native", path]
    if counts:
        args = ["perf", "stat", "-x,", "-e", "instructions:u", "--"] + args
    t = time.perf_counter()
    out = subprocess.run(args, env=env, capture_output=True, text=True)
    ms = (time.perf_counter() - t) * 1000.0
    if out.returncode != 0:
        sys.exit(f"frontend: {' '.join(args)} failed:\n{out.stderr[-2000:]}")
    phases = {}
    if counts:
        m = re.search(r"^(\d+),[^,]*,instructions", out.stderr, re.M)
        if m:
            phases["Minstr"] = int(m.group(1)) / 1e6
    for name in PHASES:
        m = re.search(rf"\b{name}=([0-9.]+)ms", out.stderr)
        if m:
            phases[name] = float(m.group(1))
    return ms, phases


def digest(path):
    return hashlib.sha256(open(path, "rb").read()).hexdigest()[:12]


def measure(loft, size, backend, modes, runs, extra_env=None, show=True, counts=False):
    """Every mode of one (size, backend), INTERLEAVED run by run: load on a shared machine
    drifts over minutes, and measuring the modes one after another turned that drift into a
    2x 'difference' between them that the instruction count did not have."""
    src = corpus(size)
    state = {}
    with tempfile.TemporaryDirectory(prefix="loft-frontend-") as d:
        for mode in modes:
            path = os.path.join(d, f"{mode}_{size}.loft")
            with open(path, "w", encoding="utf-8") as f:
                f.write(src)
            if mode in ("warm", "edit"):
                run_once(loft, path, backend, mode, extra_env)      # prime the caches
            state[mode] = {"path": path, "walls": [], "phases": [], "hashes": []}
        nonce = os.urandom(4).hex()
        for r in range(runs):
            for mode in modes:
                st = state[mode]
                if mode == "edit":
                    with open(st["path"], "w", encoding="utf-8") as f:
                        f.write(src + f"// edit {nonce} {r}\n")
                st["hashes"].append(digest(st["path"]))
                w, p = run_once(loft, st["path"], backend, mode, extra_env, counts)
                st["walls"].append(w)
                st["phases"].append(p)
    out = {}
    for mode in modes:
        st = state[mode]
        if mode == "edit" and len(set(st["hashes"])) != len(st["hashes"]):
            sys.exit("frontend: an edit run repeated its input hash — it measured a cache hit")
        if mode == "warm" and len(set(st["hashes"])) != 1:
            sys.exit("frontend: a warm run's input changed between runs")
        if show:
            print(f"   inputs {size}/{backend}/{mode}: {' '.join(st['hashes'])}")
        keys = [k for k in ("Minstr",) + PHASES if any(k in p for p in st["phases"])]
        med = {k: statistics.median(p[k] for p in st["phases"] if k in p) for k in keys}
        out[mode] = (statistics.median(st["walls"]), min(st["walls"]), med)
    return out


# ── the report ───────────────────────────────────────────────────────────────

def report(a):
    lines = {s: corpus(s).count("\n") for s in SIZES}
    print(f"== frontend bench (corpus v{CORPUS_VERSION}; {a.runs} runs, median) — {a.loft}")
    print(f"   sizes: " + ", ".join(f"{s} = {lines[s]} lines" for s in a.sizes))
    load = os.getloadavg()[0]
    if load > (os.cpu_count() or 1) / 2:
        print(f"   ⚠ load average {load:.1f} on {os.cpu_count()} CPUs: wall times are unreliable "
              f"under load — compare --counts instead")
    rows = []
    for size in a.sizes:
        for backend in a.backends:
            if backend == "native" and size == "large":
                continue   # rustc dominates it and the front end is measured on the interpreter
            got = measure(a.loft, size, backend, a.modes, a.runs, counts=a.counts)
            for mode in a.modes:
                med, lo, ph = got[mode]
                rows.append((size, backend, mode, med, lo, ph))
    print(f"\n{'size':<8}{'backend':<11}{'mode':<6}{'median':>9}{'min':>9}   phases (median ms)")
    for size, backend, mode, med, lo, ph in rows:
        split = "  ".join(f"{k}={v:.1f}" for k, v in ph.items()
                          if backend == "interpret" or k == "Minstr")
        print(f"{size:<8}{backend:<11}{mode:<6}{med:>8.1f}ms{lo:>7.1f}ms   {split}")
    if a.tsv:
        with open(a.tsv, "w", encoding="utf-8") as f:
            cols = ("Minstr",) + PHASES
            f.write("corpus\tsize\tbackend\tmode\tmedian_ms\tmin_ms\t" + "\t".join(cols) + "\n")
            for size, backend, mode, med, lo, ph in rows:
                f.write(f"v{CORPUS_VERSION}\t{size}\t{backend}\t{mode}\t{med:.2f}\t{lo:.2f}\t"
                        + "\t".join(f"{ph[k]:.2f}" if k in ph else "" for k in cols) + "\n")
        print(f"\nwrote {a.tsv}")
    return 0


def self_test(a):
    """The harness must report a slowdown it was handed: LOFT_TIMING_INJECT_MS sleeps inside
    the timed scope pass, so both the wall time and the `scopes` phase must rise by it."""
    inject = 60
    base_med, _, base_ph = measure(a.loft, "tiny", "interpret", ["cold"], a.runs, show=False)["cold"]
    slow_med, _, slow_ph = measure(a.loft, "tiny", "interpret", ["cold"], a.runs,
                                   {"LOFT_TIMING_INJECT_MS": str(inject)}, show=False)["cold"]
    d_wall = slow_med - base_med
    d_scopes = slow_ph.get("scopes", 0) - base_ph.get("scopes", 0)
    ok = d_wall >= 0.8 * inject and d_scopes >= 0.8 * inject
    print(f"self-test: injected {inject} ms → wall +{d_wall:.1f} ms, scopes +{d_scopes:.1f} ms: "
          f"{'PASS' if ok else 'FAIL — the harness cannot see a front-end slowdown'}")
    return 0 if ok else 1


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--loft", default=os.path.join(ROOT, "target", "release", "loft"))
    ap.add_argument("--runs", type=int, default=5)
    ap.add_argument("--sizes", default="tiny,medium,large")
    ap.add_argument("--backends", default="interpret,native")
    ap.add_argument("--modes", default="cold,warm,edit")
    ap.add_argument("--tsv")
    ap.add_argument("--counts", action="store_true",
                    help="also count instructions (perf stat): steady under load, where wall time is not")
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--emit", metavar="SIZE",
                    help="print the corpus of one size and exit (the C1 allocation gate reads it)")
    a = ap.parse_args(argv)
    if a.emit:
        if a.emit not in SIZES:
            ap.error(f"unknown size {a.emit}; one of {', '.join(SIZES)}")
        sys.stdout.write(corpus(a.emit))
        return 0
    a.sizes = [s for s in a.sizes.split(",") if s]
    a.backends = [b for b in a.backends.split(",") if b]
    a.modes = [m for m in a.modes.split(",") if m]
    for s in a.sizes:
        if s not in SIZES:
            ap.error(f"unknown size {s}; one of {', '.join(SIZES)}")
    if not os.path.exists(a.loft):
        ap.error(f"no loft binary at {a.loft} (cargo build --release --bin loft)")
    return self_test(a) if a.self_test else report(a)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
