#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""stats.py — the benchmark suite as STATISTICS: is each routine within its bar of Rust?

    python3 bench/stats.py [--only 01,08] [--lanes native,rust] [--samples 7]
                           [--target-ms 400] [--bar 2.0] [--tsv out.tsv] [--no-pin]
                           [--package DIR[=NAME]]...

`run_bench.sh` prints one wall time per lane, which for a routine that finishes in a few
milliseconds is mostly the timer's resolution.  This tool makes the same programs answer
with numbers that can be compared from one pass to the next:

  * every program runs its op `--n` times inside its own timed region and prints ns per op
    (bench/README.md § The row protocol), so start-up and set-up are never in the number;
  * `--n` is CALIBRATED per lane until a run lasts `--target-ms`, so a 3 ms op and a 300 ms
    op are both measured over the same stretch of wall time;
  * each lane is sampled `--samples` times, the lanes INTERLEAVED so drift lands on both,
    and the process is pinned to one performance core where the machine has `taskset`
    (a hybrid CPU otherwise moves a run between core kinds, a ±10 % swing by itself);
  * one warm-up round is discarded, and a row reports the MEDIAN, the spread of the
    samples (their interquartile range, as a percentage of the median) and the ratio with
    its RANGE (the native lane's first quartile over the reference's third, and the
    reverse).  The verdict reads the range, not the point: `ok` only when the whole range
    is inside the bar, `OVER` only when the whole range is outside it, and `unclear`
    otherwise — a row this tool cannot decide says so instead of printing a number that
    looks decided;
  * the HASHES must agree across the lanes that ran a routine.  A routine whose lanes
    disagree is not one algorithm and its times do not compare — always fatal.

Lanes: `native` (loft `--native-emit --lean`, rustc opt-level 3, one codegen unit — what
`--native-release` ships), `rust` (the reference, `rustc -O` unless `--ref-flags` says
otherwise), `interp` (the loft interpreter) and `python`.  The ratio column is
native / rust; the other lanes are reported beside it.

`--package DIR` measures a LIBRARY's own bench — `DIR/bench/bench.loft` beside its twin
`DIR/bench/bench.rs`, the layout the drawing library set (@PLN158).  A package has
dependencies, so its native lane is built by loft itself (`--native-release`, what a consumer
gets) and sampled from the binary loft cached.  Point it at a SCRATCH CLONE, never at the
library's working tree: the build writes caches beside the source.

A REPORT, never a gate: timings are machine-bound.  Exit 1 on a hash disagreement or a
lane that fails to build or run; exit 0 otherwise, whatever the ratios say.
"""
import argparse
import os
import shutil
import statistics
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)


def fail(msg):
    sys.stderr.write(f"stats.py: {msg}\n")
    sys.exit(1)


# ── pinning ──────────────────────────────────────────────────────────────────────────────
def fastest_cpus(count):
    """The first hardware thread of the `count` fastest distinct cores, or [] when the
    topology cannot be read (not Linux) — the caller then runs unpinned and says so."""
    base = "/sys/devices/system/cpu"
    cores = {}
    try:
        for name in os.listdir(base):
            if not (name.startswith("cpu") and name[3:].isdigit()):
                continue
            cpu = int(name[3:])
            with open(f"{base}/{name}/cpufreq/cpuinfo_max_freq") as f:
                freq = int(f.read())
            with open(f"{base}/{name}/topology/core_id") as f:
                core = int(f.read())
            best = cores.get(core)
            if best is None or cpu < best[1]:
                cores[core] = (freq, cpu)
    except OSError:
        return []
    ranked = sorted(cores.values(), key=lambda fc: (-fc[0], fc[1]))
    return [cpu for _, cpu in ranked[:count]]


def pin_prefix(threads, enabled):
    if not enabled or not shutil.which("taskset"):
        return []
    cpus = fastest_cpus(threads)
    if len(cpus) < threads:
        return []
    return ["taskset", "-c", ",".join(str(c) for c in cpus)]


# ── building the lanes ───────────────────────────────────────────────────────────────────
def run_checked(cmd, what, **kw):
    p = subprocess.run(cmd, capture_output=True, text=True, **kw)
    if p.returncode != 0:
        fail(f"{what} failed ({p.returncode}):\n{p.stderr[-3000:]}")
    return p


def build(bench, lanes, loft, lib_dir, ref_flags):
    """The command that runs each requested lane of `bench`, built where it needs building."""
    d = os.path.join(HERE, bench)
    out = os.path.join(d, ".loft")
    os.makedirs(out, exist_ok=True)
    cmds = {}
    if "native" in lanes:
        rs = os.path.join(out, "stats_native.rs")
        exe = os.path.join(out, "stats_native")
        run_checked([loft, "--native-emit", rs, "--lean", "--path", ROOT + "/",
                     os.path.join(d, "bench.loft")], f"{bench}: loft --native-emit")
        run_checked(["rustc", "-C", "opt-level=3", "-C", "codegen-units=1", "--edition=2024",
                     "--extern", f"loft={lib_dir}/libloft.rlib", "-L", f"{lib_dir}/deps",
                     "-o", exe, rs], f"{bench}: rustc (native lane)")
        os.remove(rs)
        cmds["native"] = [exe]
    if "rust" in lanes and os.path.exists(os.path.join(d, "bench.rs")):
        exe = os.path.join(out, "stats_rs")
        run_checked(["rustc", *ref_flags, "-o", exe, os.path.join(d, "bench.rs")],
                    f"{bench}: rustc (reference)")
        cmds["rust"] = [exe]
    if "interp" in lanes:
        cmds["interp"] = [loft, "--interpret", "--path", ROOT + "/", os.path.join(d, "bench.loft")]
    if "python" in lanes and os.path.exists(os.path.join(d, "bench.py")):
        cmds["python"] = [sys.executable, os.path.join(d, "bench.py")]
    return cmds


def build_package(pkg_dir, lanes, loft, ref_flags):
    """The lanes of a library's own bench.  The native lane is what `loft --native-release`
    builds and caches for `bench/bench.loft`; the newest file in that cache IS the binary."""
    bench_dir = os.path.join(pkg_dir, "bench")
    src = os.path.join(bench_dir, "bench.loft")
    if not os.path.exists(src):
        fail(f"{pkg_dir}: no bench/bench.loft")
    cmds = {}
    if "native" in lanes:
        run_checked([loft, "--native-release", "bench/bench.loft", "--n", "2"],
                    f"{pkg_dir}: loft --native-release", cwd=pkg_dir)
        cache = os.path.join(bench_dir, ".loft", "cache")
        built = [os.path.join(cache, f) for f in os.listdir(cache) if f.startswith("bench-")] \
            if os.path.isdir(cache) else []
        if not built:
            fail(f"{pkg_dir}: loft left no cached native binary under bench/.loft/cache")
        cmds["native"] = [max(built, key=os.path.getmtime)]
    if "rust" in lanes and os.path.exists(os.path.join(bench_dir, "bench.rs")):
        out = os.path.join(bench_dir, ".build")
        os.makedirs(out, exist_ok=True)
        exe = os.path.join(out, "stats_rs")
        run_checked(["rustc", *ref_flags, "--edition=2021", "-o", exe,
                     os.path.join(bench_dir, "bench.rs")], f"{pkg_dir}: rustc (reference)")
        cmds["rust"] = [exe]
    if "interp" in lanes:
        cmds["interp"] = [loft, "--interpret", "bench/bench.loft"]
    return cmds


# ── running ──────────────────────────────────────────────────────────────────────────────
def rows_of(cmd, n, pin, what, cwd=None):
    p = subprocess.run([*pin, *cmd, "--n", str(n)], capture_output=True, text=True, cwd=cwd)
    if p.returncode != 0:
        fail(f"{what} failed ({p.returncode}):\n{(p.stderr or p.stdout)[-3000:]}")
    rows = {}
    for line in p.stdout.splitlines():
        parts = line.split("\t")
        if len(parts) != 7 or parts[0] == "routine":
            continue
        name, iters, us, ns_op, _items, _per, h = parts
        rows[name] = dict(iters=int(iters), us=int(us), ns=int(ns_op), hash=h)
    if not rows:
        fail(f"{what}: no rows in its output:\n{p.stdout[-1500:]}")
    return rows


def calibrate(cmd, pin, target_us, what, cwd=None):
    """The even `--n` at which the SLOWEST routine of this lane runs for about the target.
    Two probe runs: the first finds the scale, the second corrects a first op that carried
    one-time costs (a buffer's growth, a cold cache)."""
    n = 2
    for _ in range(2):
        rows = rows_of(cmd, n, pin, what, cwd)
        per_op = max(max(r["us"], 1) / r["iters"] for r in rows.values())
        want = int(target_us / per_op)
        n = max(2, min(want - want % 2, 2_000_000))
    return n


def quartiles(xs):
    """(q1, q3) of the samples; the extremes themselves when there are too few to split."""
    if len(xs) < 4:
        return min(xs), max(xs)
    q = statistics.quantiles(xs, n=4, method="inclusive")
    return q[0], q[2]


def spread(xs):
    """The interquartile range as a percentage of the median: how far the MIDDLE half of
    the samples sits apart.  One spike among seven moves max−min and leaves this alone."""
    m = statistics.median(xs)
    q1, q3 = quartiles(xs)
    return (q3 - q1) / m * 100.0 if m else 0.0


def run_metadata(a):
    """What a reader of a saved run needs to know about where it came from: the machine is
    part of every row (a ratio is between two lanes on ONE machine)."""
    import datetime
    import platform

    def git(*args):
        p = subprocess.run(["git", "-C", ROOT, *args], capture_output=True, text=True)
        return p.stdout.strip()

    return {
        "host": platform.node(),
        "arch": f"{platform.machine()}-{platform.system().lower()}",
        "date": datetime.date.today().isoformat(),
        "commit": git("rev-parse", "--short", "HEAD"),
        "dirty": "yes" if git("status", "--porcelain", "--untracked-files=no") else "no",
        "rustc": subprocess.run(["rustc", "--version"], capture_output=True, text=True).stdout.strip(),
        "samples": a.samples,
        "target_ms": a.target_ms,
        "ref_flags": a.ref_flags,
        "pinned": "no" if a.no_pin or not pin_prefix(1, True) else "yes",
    }


def threads_of(bench):
    return 4 if bench.startswith("11_") else 1


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--only", default="", help="comma-separated bench numbers or names")
    ap.add_argument("--lanes", default="native,rust")
    ap.add_argument("--samples", type=int, default=7)
    ap.add_argument("--target-ms", type=float, default=400.0)
    ap.add_argument("--bar", type=float, default=2.0)
    ap.add_argument("--noisy", type=float, default=5.0, help="spread %% above which a row is flagged")
    ap.add_argument("--ref-flags", default="-O", help="rustc flags for the reference lane")
    ap.add_argument("--package-target-ms", type=float, default=4000.0,
                    help="the run length a --package lane is calibrated to: a library's bench "
                         "prints routines a thousand times apart in cost under ONE --n, so its "
                         "slowest routine has to run long for its fastest to be resolved")
    ap.add_argument("--coarse-us", type=float, default=200.0,
                    help="a row timed over fewer microseconds than this is flagged coarse")
    ap.add_argument("--tsv", default="")
    ap.add_argument("--package", action="append", default=[], metavar="DIR[=NAME]",
                    help="a library's own bench (DIR/bench/bench.loft + bench.rs); repeatable")
    ap.add_argument("--no-suite", action="store_true", help="measure only the --package lanes")
    ap.add_argument("--no-pin", action="store_true")
    ap.add_argument("--show-samples", action="store_true", help="print every sample under its row")
    ap.add_argument("--loft", default=os.environ.get("LOFT_BIN", os.path.join(ROOT, "target/release/loft")))
    ap.add_argument("--lib-dir", default=os.environ.get("LOFT_LIB_DIR", os.path.join(ROOT, "target/release")))
    a = ap.parse_args()

    lanes = [x for x in a.lanes.split(",") if x]
    unknown = set(lanes) - {"native", "rust", "interp", "python"}
    if unknown:
        fail(f"unknown lane(s): {', '.join(sorted(unknown))}")
    if not os.access(a.loft, os.X_OK):
        fail(f"no loft binary at {a.loft} (cargo build --release)")
    benches = sorted(b for b in os.listdir(HERE)
                     if b[:2].isdigit() and os.path.exists(os.path.join(HERE, b, "bench.loft")))
    if a.only:
        want = [w.strip() for w in a.only.split(",")]
        benches = [b for b in benches
                   if any(b == w or b.split("_")[0].lstrip("0") == w.lstrip("0") for w in want)]
    if a.no_suite:
        benches = []
    elif not benches:
        fail("no benchmark matches --only")
    packages = []
    for spec in a.package:
        pkg_dir, _, name = spec.partition("=")
        pkg_dir = os.path.abspath(pkg_dir)
        packages.append((name or os.path.basename(pkg_dir.rstrip("/")), pkg_dir))

    target_us = a.target_ms * 1000.0
    pinned = bool(pin_prefix(1, not a.no_pin))
    print(f"# lanes {','.join(lanes)} · {a.samples} samples of ~{a.target_ms:.0f} ms each · "
          f"reference rustc {a.ref_flags} · "
          f"{'pinned to the fastest core(s)' if pinned else 'NOT pinned'} · bar {a.bar}x")
    head = f"{'bench':15} {'routine':13}"
    for lane in lanes:
        head += f" {lane + ' ns/op':>15} {'±%':>5}"
    if "native" in lanes and "rust" in lanes:
        head += f" {'nat/rust':>9} {'range':>13}  verdict"
    print(head)

    stamp = run_metadata(a)
    out_rows = []
    ratios = []
    verdicts = {"ok": 0, "OVER": 0, "unclear": 0}
    units = [(b, None) for b in benches] + packages
    for bench, pkg_dir in units:
        if pkg_dir is None:
            cmds = build(bench, lanes, a.loft, a.lib_dir, a.ref_flags.split())
        else:
            cmds = build_package(pkg_dir, lanes, a.loft, a.ref_flags.split())
        cwd = pkg_dir
        pin = pin_prefix(threads_of(bench), not a.no_pin)
        unit_target = target_us if pkg_dir is None else max(target_us, a.package_target_ms * 1000.0)
        n_of = {lane: calibrate(cmd, pin, unit_target, f"{bench} [{lane}]", cwd) for lane, cmd in cmds.items()}
        samples = {lane: {} for lane in cmds}
        regions = {lane: {} for lane in cmds}
        hashes = {lane: {} for lane in cmds}
        # One discarded round first: the first run of a lane after the OTHER lane's
        # calibration reads 5–8 % slow (caches, frequency ramp), every later one does not.
        for cmd_lane, cmd in cmds.items():
            rows_of(cmd, n_of[cmd_lane], pin, f"{bench} [{cmd_lane}] warm-up", cwd)
        for _ in range(a.samples):
            for lane, cmd in cmds.items():
                for name, r in rows_of(cmd, n_of[lane], pin, f"{bench} [{lane}]", cwd).items():
                    samples[lane].setdefault(name, []).append(r["ns"])
                    regions[lane].setdefault(name, []).append(r["us"])
                    prev = hashes[lane].setdefault(name, r["hash"])
                    if prev != r["hash"]:
                        fail(f"{bench}/{name} [{lane}]: the hash changed between runs ({prev} vs {r['hash']})")
        names = list(next(iter(samples.values())))
        for name in names:
            seen = {lane: hashes[lane].get(name) for lane in cmds if name in hashes[lane]}
            if len(set(seen.values())) != 1:
                fail(f"{bench}/{name}: the lanes compute different results — {seen}")
            line = f"{bench:15} {name:13}"
            rec = dict(bench=bench, routine=name, hash=next(iter(seen.values())),
                       commit=stamp["commit"], date=stamp["date"])
            for lane in lanes:
                xs = samples.get(lane, {}).get(name)
                if not xs:
                    line += f" {'—':>15} {'':>5}"
                    continue
                med, sp = statistics.median(xs), spread(xs)
                line += f" {med:>15,.0f} {sp:>5.1f}"
                rec.update({f"{lane}_ns": med, f"{lane}_min": min(xs), f"{lane}_max": max(xs),
                            f"{lane}_spread": sp, f"{lane}_n": n_of[lane]})
            nat, ref = samples.get("native", {}).get(name), samples.get("rust", {}).get(name)
            if nat and ref:
                ratio = statistics.median(nat) / statistics.median(ref)
                (nq1, nq3), (rq1, rq3) = quartiles(nat), quartiles(ref)
                lo, hi = nq1 / rq3, nq3 / rq1
                if hi <= a.bar:
                    verdict = "ok"
                elif lo > a.bar:
                    verdict = "OVER"
                else:
                    verdict = "unclear"
                verdicts[verdict] += 1
                ratios.append(ratio)
                noisy = max(spread(nat), spread(ref)) > a.noisy
                # A program runs every routine `--n` times, calibrated on its SLOWEST one, so a
                # routine a thousand times faster is timed over a region of a few microseconds
                # of a microsecond clock.  Such a row is COARSE: its figure is real but blunt.
                coarse = min(statistics.median(regions["native"][name]),
                             statistics.median(regions["rust"][name])) < a.coarse_us
                flags = ("  (noisy)" if noisy else "") + ("  (coarse)" if coarse else "")
                line += f" {ratio:>9.2f} {f'{lo:.2f}–{hi:.2f}':>13}  {verdict}{flags}"
                rec.update(ratio=ratio, ratio_lo=lo, ratio_hi=hi, verdict=verdict,
                           flags=(("noisy " if noisy else "") + ("coarse" if coarse else "")).strip())
            print(line, flush=True)
            if a.show_samples:
                for lane in lanes:
                    xs = samples.get(lane, {}).get(name)
                    if xs:
                        print(f"    {lane:7} n={n_of[lane]:<7} " + " ".join(f"{x:,}" for x in xs))
            out_rows.append(rec)

    if ratios:
        print(f"\n{len(ratios)} routine(s): median ratio {statistics.median(ratios):.2f}x, "
              f"worst {max(ratios):.2f}x · within {a.bar}x: {verdicts['ok']} · over: {verdicts['OVER']} · "
              f"unclear: {verdicts['unclear']}")
    if a.tsv:
        keys = sorted({k for r in out_rows for k in r})
        keys = ["bench", "routine"] + [k for k in keys if k not in ("bench", "routine")]
        with open(a.tsv, "w") as f:
            for k, v in run_metadata(a).items():
                f.write(f"# {k}={v}\n")
            f.write("\t".join(keys) + "\n")
            for r in out_rows:
                f.write("\t".join(f"{r.get(k, ''):.2f}" if isinstance(r.get(k), float) else str(r.get(k, ""))
                                  for k in keys) + "\n")
        print(f"wrote {a.tsv}")


if __name__ == "__main__":
    main()
