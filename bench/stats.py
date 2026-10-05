#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""stats.py — the benchmark suite as STATISTICS: is each routine within its bar of Rust?

    python3 bench/stats.py [--only 01,08] [--lanes native,rust] [--samples 7]
                           [--target-ms 400] [--bar 2.0] [--tsv out.tsv] [--no-pin]
                           [--package DIR[=NAME]]... [--routine [BENCH/]NAME[,…]]...

`--routine` measures a routine, or a set, without the rest of the portal: alone it builds
only the programs `portal/routines.tsv` places them in, calibrates `--n` on the picked
routines (capped by `--max-run-ms` for the whole run) and reports only their rows.  The
program still runs its other routines — the twins take no filter — so their cost is what
the cap bounds.

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

The programs are BUILT before they are measured, in sets of `--batch` (20): a set compiles
`--build-jobs` (5) at a time at low priority, then is measured `--measure-jobs` (3) programs
at a time, each on its own fastest cores, so no build ever runs beside a measurement.  A
program that declares `// bench-threads: N` gets N cores and is measured alone.  Every build step's wall-clock and CPU seconds and peak
memory are printed and, with `--build-tsv`, written out (bench/README.md § Statistics).

A REPORT, never a gate: timings are machine-bound.  Exit 1 on a hash disagreement, a lane
that fails to run, or a program that fails to build (after the others are measured); exit 0
otherwise, whatever the ratios say.
"""
import argparse
import os
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from concurrent.futures import ThreadPoolExecutor, as_completed

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)


def host_key():
    """The name a machine's results are filed under (`portal/results/<host>.tsv`).  A box's
    node name follows the network it is on (`firewall02.lan.betterbe.com` at the office,
    `mac.home` at home), which would split one machine's history in two: `LOFT_PERF_HOST`
    pins it."""
    return os.environ.get("LOFT_PERF_HOST") or platform.node() or "unknown"


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


def top_tier_cpus():
    """The first hardware thread of every core in the FASTEST tier (max frequency within
    10 % of the fastest core's): the cores a ratio measured on one of them compares with.  A
    hybrid CPU's efficiency cores are a different machine for this purpose."""
    base = "/sys/devices/system/cpu"
    freq = {}
    for cpu in fastest_cpus(os.cpu_count() or 1):
        try:
            with open(f"{base}/cpu{cpu}/cpufreq/cpuinfo_max_freq") as f:
                freq[cpu] = int(f.read())
        except OSError:
            return []
    top = max(freq.values(), default=0)
    return [cpu for cpu, fr in freq.items() if fr >= 0.9 * top]


def pin_prefix(threads, enabled):
    if not enabled or not shutil.which("taskset"):
        return []
    cpus = fastest_cpus(threads)
    if len(cpus) < threads:
        return []
    return ["taskset", "-c", ",".join(str(c) for c in cpus)]


# ── building the lanes ───────────────────────────────────────────────────────────────────
class BuildFailed(Exception):
    """A lane that did not build.  Raised in a build worker, so it cannot `fail()` the
    run: the other programs are still built and measured, and the failures are listed at
    the end (exit 1)."""


NICE = ["nice", "-n", "10"] if shutil.which("nice") else []


def mem_available_gb():
    try:
        with open("/proc/meminfo") as f:
            for line in f:
                if line.startswith("MemAvailable:"):
                    return int(line.split()[1]) / (1024 * 1024)
    except OSError:
        pass
    return None


def run_checked(cmd, what, times, step, reserve_gb=0.0, **kw):
    """Run one build step at low priority and record what it cost into `times`: wall-clock
    seconds, CPU seconds (user + system, the child's and every descendant it waited for —
    the figure contention does not inflate) and peak resident memory (the largest process
    of the tree).  Waits first while the machine has less than `reserve_gb` available."""
    while reserve_gb and (mem_available_gb() or reserve_gb) < reserve_gb:
        time.sleep(1.0)
    with tempfile.TemporaryFile() as out, tempfile.TemporaryFile() as err:
        t0 = time.monotonic()
        p = subprocess.Popen([*NICE, *cmd], stdout=out, stderr=err, **kw)
        _, status, ru = os.wait4(p.pid, 0)
        p.returncode = os.waitstatus_to_exitcode(status)
        wall = time.monotonic() - t0
        times.append(dict(step=step, wall_s=wall, cpu_s=ru.ru_utime + ru.ru_stime,
                          peak_mb=ru.ru_maxrss / 1024.0))
        if p.returncode != 0:
            err.seek(0)
            raise BuildFailed(f"{what} failed ({p.returncode}):\n"
                              f"{err.read().decode(errors='replace')[-3000:]}")


def build(bench, lanes, loft, lib_dir, ref_flags, times, reserve_gb):
    """The command that runs each requested lane of `bench`, built where it needs building."""
    d = os.path.join(HERE, bench)
    out = os.path.join(d, ".loft")
    os.makedirs(out, exist_ok=True)
    cmds = {}
    if "native" in lanes:
        rs = os.path.join(out, "stats_native.rs")
        exe = os.path.join(out, "stats_native")
        run_checked([loft, "--native-emit", rs, "--lean", "--path", ROOT + "/",
                     os.path.join(d, "bench.loft")], f"{bench}: loft --native-emit", times,
                    "native-emit", reserve_gb)
        run_checked(["rustc", "-C", "opt-level=3", "-C", "codegen-units=1", "--edition=2024",
                     "--extern", f"loft={lib_dir}/libloft.rlib", "-L", f"{lib_dir}/deps",
                     "-o", exe, rs], f"{bench}: rustc (native lane)", times, "native-rustc",
                    reserve_gb)
        os.remove(rs)
        cmds["native"] = [exe]
    if "rust" in lanes and os.path.exists(os.path.join(d, "bench.rs")):
        exe = os.path.join(out, "stats_rs")
        run_checked(["rustc", *ref_flags, "-o", exe, os.path.join(d, "bench.rs")],
                    f"{bench}: rustc (reference)", times, "rust-rustc", reserve_gb)
        cmds["rust"] = [exe]
    if "interp" in lanes:
        cmds["interp"] = [loft, "--interpret", "--path", ROOT + "/", os.path.join(d, "bench.loft")]
    if "python" in lanes and os.path.exists(os.path.join(d, "bench.py")):
        cmds["python"] = [sys.executable, os.path.join(d, "bench.py")]
    return cmds


def build_package(pkg_dir, lanes, loft, ref_flags, times, reserve_gb):
    """The lanes of a library's own bench.  The native lane is what `loft --native-release`
    builds and caches for `bench/bench.loft`; the newest binary in that cache IS the binary.
    A binary is named `native-<key>` (by what it compiles to, so one program at two paths is
    one entry); a cache written by an older loft still names it `bench-<key>`."""
    bench_dir = os.path.join(pkg_dir, "bench")
    src = os.path.join(bench_dir, "bench.loft")
    if not os.path.exists(src):
        raise BuildFailed(f"{pkg_dir}: no bench/bench.loft")
    cmds = {}
    if "native" in lanes:
        run_checked([loft, "--native-release", "bench/bench.loft", "--n", "2"],
                    f"{pkg_dir}: loft --native-release", times, "native-release", reserve_gb,
                    cwd=pkg_dir)
        cache = os.path.join(bench_dir, ".loft", "cache")
        built = [os.path.join(cache, f) for f in os.listdir(cache) if f.startswith(("native-", "bench-"))] \
            if os.path.isdir(cache) else []
        if not built:
            raise BuildFailed(f"{pkg_dir}: loft left no cached native binary under bench/.loft/cache")
        cmds["native"] = [max(built, key=os.path.getmtime)]
    if "rust" in lanes and os.path.exists(os.path.join(bench_dir, "bench.rs")):
        out = os.path.join(bench_dir, ".build")
        os.makedirs(out, exist_ok=True)
        exe = os.path.join(out, "stats_rs")
        run_checked(["rustc", *ref_flags, "--edition=2021", "-o", exe,
                     os.path.join(bench_dir, "bench.rs")], f"{pkg_dir}: rustc (reference)",
                    times, "rust-rustc", reserve_gb)
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


def calibrate(cmd, pin, target_us, what, cwd=None, keep=None, max_run_us=None):
    """The even `--n` at which the SLOWEST routine of this lane runs for about the target.
    Two probe runs: the first finds the scale, the second corrects a first op that carried
    one-time costs (a buffer's growth, a cold cache).

    With `keep` (a `--routine` selection) the target is the slowest KEPT routine's — a
    program runs every routine `--n` times, and one calibrated on a slow neighbour times a
    fast pick over a few microseconds.  Routine costs inside one program differ by up to
    ~30000x (`drawing`), so the whole run is held to `max_run_us`: past it the pick is timed
    over less than the target, and a region under `--coarse-us` is flagged as usual."""
    n = 2
    for _ in range(2):
        rows = rows_of(cmd, n, pin, what, cwd)
        picked = [r for name, r in rows.items() if keep is None or keep(name)] or list(rows.values())
        per_op = max(max(r["us"], 1) / r["iters"] for r in picked)
        want = int(target_us / per_op)
        if keep is not None and max_run_us:
            run_per_op = sum(max(r["us"], 1) / r["iters"] for r in rows.values())
            want = min(want, int(max_run_us / run_per_op))
        n = max(2, min(want - want % 2, 2_000_000))
    return n


def parse_selection(specs):
    """`--routine` values -> a list of (bench or None, routine).  A value is comma-separated
    `routine` or `bench/routine`; the qualified form tells apart the names more than one
    program uses (`hash`, `parse`, `lock`, `byte_at`)."""
    sel = []
    for spec in specs:
        for part in spec.split(","):
            part = part.strip()
            if part:
                bench, _, name = part.rpartition("/")
                sel.append((bench or None, name))
    return sel


def selected(sel, bench, name):
    return any(name == r and (b is None or b == bench) for b, r in sel)


def resolve_selection(sel):
    """The units holding the selected routines, read from the routine registry
    (`portal/routines.tsv`): in-repo benches by directory, library benches by their
    checkout (`portal.library_packages`).  Fails on a routine the registry does not know,
    or one whose library has no checkout."""
    registry = []
    with open(os.path.join(HERE, "portal", "routines.tsv")) as f:
        for line in f:
            if line.strip() and not line.startswith("#"):
                bench, name = line.split("\t")[:2]
                registry.append((bench, name))
    sys.path.insert(0, os.path.join(HERE, "portal"))
    from portal import library_packages  # noqa: E402
    checkouts = {}
    for spec in library_packages()[1::2]:
        pkg_dir, _, name = spec.rpartition("=")
        checkouts[name] = pkg_dir
    benches, packages, unknown = set(), {}, []
    for b, r in sel:
        hits = [bench for bench, name in registry if name == r and (b is None or b == bench)]
        if not hits:
            unknown.append(f"{b}/{r}" if b else r)
        for bench in hits:
            if bench[:2].isdigit():
                benches.add(bench)
            elif bench in checkouts:
                packages[bench] = checkouts[bench]
            else:
                fail(f"--routine {r}: its bench `{bench}` has no checkout (make perf-libs)")
    if unknown:
        fail(f"--routine: not in bench/portal/routines.tsv: {', '.join(unknown)}")
    return sorted(benches), sorted(packages.items())


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
        "host": host_key(),
        "arch": f"{platform.machine()}-{platform.system().lower()}",
        "date": datetime.date.today().isoformat(),
        "commit": git("rev-parse", "--short", "HEAD"),
        # The portal's own outputs are not the tree under measurement: a partial run is
        # always taken over the results file the previous run left modified.
        "dirty": "yes" if git("status", "--porcelain", "--untracked-files=no", "--", ".",
                              ":!bench/portal/results", ":!doc/claude/PERF_PORTAL.md") else "no",
        "rustc": subprocess.run(["rustc", "--version"], capture_output=True, text=True).stdout.strip(),
        "samples": a.samples,
        "target_ms": a.target_ms,
        "ref_flags": a.ref_flags,
        "pinned": "no" if a.no_pin or not pin_prefix(1, True) else "yes",
        "measure_jobs": a.measure_jobs,
    }


def build_batch(batch, lanes, a, build_rows, failed):
    """Build every program of `batch`, `a.build_jobs` at a time: each worker takes the next
    program the moment its last one is built.  Answers {bench: lane commands} for the ones
    that built; a failure goes into `failed` and the rest carry on."""
    def one(bench, pkg_dir):
        times = []
        try:
            if pkg_dir is None:
                cmds = build(bench, lanes, a.loft, a.lib_dir, a.ref_flags.split(), times,
                             a.build_mem_reserve_gb)
            else:
                cmds = build_package(pkg_dir, lanes, a.loft, a.ref_flags.split(), times,
                                     a.build_mem_reserve_gb)
            return cmds, times, None
        except BuildFailed as e:
            return None, times, str(e)

    built = {}
    with ThreadPoolExecutor(max_workers=max(1, a.build_jobs)) as pool:
        futures = {pool.submit(one, bench, pkg_dir): bench for bench, pkg_dir in batch}
        for fut in as_completed(futures):
            bench = futures[fut]
            cmds, times, err = fut.result()
            build_rows.extend(dict(bench=bench, **t) for t in times)
            if err:
                failed.append((bench, err))
                print(f"# build FAILED {bench}: {err.splitlines()[0]}", flush=True)
                continue
            built[bench] = cmds
            cost = " · ".join(f"{t['step']} {t['wall_s']:.1f}s (cpu {t['cpu_s']:.1f}s, "
                              f"{t['peak_mb']:,.0f} MB)" for t in times)
            print(f"# built {bench}: {cost}", flush=True)
    return built


def measure_unit(bench, pkg_dir, cmds, pin, a, lanes, target_us, stamp):
    """Measure one built program: calibrate each lane, one discarded warm-up round, then
    `a.samples` interleaved rounds.  Answers its printed lines, its rows, its ratios and
    their verdicts — nothing shared is touched, so several can run at once."""
    lines, recs, ratios, verdict_list = [], [], [], []
    cwd = pkg_dir
    unit_target = target_us if pkg_dir is None else max(target_us, a.package_target_ms * 1000.0)
    keep = (lambda name: selected(a.selection, bench, name)) if a.selection else None
    n_of = {lane: calibrate(cmd, pin, unit_target, f"{bench} [{lane}]", cwd, keep,
                            a.max_run_ms * 1000.0) for lane, cmd in cmds.items()}
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
    names = [n for n in next(iter(samples.values())) if keep is None or keep(n)]
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
            verdict_list.append(verdict)
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
        lines.append(line)
        if a.show_samples:
            for lane in lanes:
                xs = samples.get(lane, {}).get(name)
                if xs:
                    lines.append(f"    {lane:7} n={n_of[lane]:<7} " + " ".join(f"{x:,}" for x in xs))
        recs.append(rec)
    return lines, recs, ratios, verdict_list

def measure_batch(jobs, a, lanes, target_us, stamp):
    """Measure the built programs of one batch, `a.measure_jobs` at a time, each on its own
    fastest cores (a program never shares a core with another one).  Answers each program's
    result in the batch's order, so the table reads the same however many ran at once."""
    if a.measure_jobs <= 1:
        for bench, pkg_dir, cmds in jobs:
            yield measure_unit(bench, pkg_dir, cmds, pin_prefix(threads_of(bench, pkg_dir), not a.no_pin),
                               a, lanes, target_us, stamp)
        return
    import threading
    free = top_tier_cpus() if not a.no_pin and shutil.which("taskset") else []
    pinning = bool(free)
    order = list(free)
    lock = threading.Condition()

    def one(bench, pkg_dir, cmds):
        threads = min(threads_of(bench, pkg_dir), len(order)) if pinning else 0
        # A threaded program takes EVERY core of the tier — it waits for the others to finish
        # and nothing starts beside it — and runs on the first `threads` of them.
        want = len(order) if threads > 1 else threads
        cpus = []
        if pinning:
            with lock:
                lock.wait_for(lambda: len(free) >= want)
                cpus, free[:] = free[:want], free[want:]
        pin = ["taskset", "-c", ",".join(map(str, cpus[:threads]))] if cpus else []
        try:
            return measure_unit(bench, pkg_dir, cmds, pin, a, lanes, target_us, stamp)
        finally:
            if cpus:
                with lock:
                    free[:0] = cpus
                    free.sort(key=order.index)
                    lock.notify_all()

    with ThreadPoolExecutor(max_workers=a.measure_jobs) as pool:
        futures = [pool.submit(one, *job) for job in jobs]
        for fut in futures:
            yield fut.result()


def threads_of(bench, pkg_dir):
    """How many threads the program runs its routines on, as its own `bench.loft` declares
    in a `// bench-threads: N` line (bench/README.md § The row protocol); 1 when it declares
    none.  It is pinned to that many cores, and a program above 1 is measured ALONE."""
    src = os.path.join(pkg_dir, "bench", "bench.loft") if pkg_dir else os.path.join(HERE, bench, "bench.loft")
    try:
        with open(src) as f:
            for line in f:
                m = re.match(r"\s*//\s*bench-threads:\s*(\d+)", line)
                if m:
                    return max(1, int(m.group(1)))
    except OSError:
        pass
    return 1


def require_compiled_stdlib(loft):
    """Refuse to time an interpreter whose compiled standard library is declined (@PLN181).

    The binary bakes the stdlib's compiled bodies against a hash of `default/*.loft`; run
    against a different `default/` (a stale `src/compiled_stdlib_gen.rs`, or a binary built
    before the stdlib moved) it declines them all and interprets — correct, and 5-23x slower on
    text routines.  Measured 2026-10-02, that read as a +2213 % "regression" of an unrelated
    change.  The binary itself says how many it dispatched (`LOFT_TIMING=1`); none is a refusal
    here, not a number in the table."""
    probe = os.path.join(ROOT, "target", "stdlib_probe.loft")
    os.makedirs(os.path.dirname(probe), exist_ok=True)
    with open(probe, "w") as f:
        f.write('fn main() { println("x"); }\n')
    out = subprocess.run([loft, "--interpret", probe], cwd=ROOT, capture_output=True, text=True,
                         env={**os.environ, "LOFT_TIMING": "1"}, timeout=120)
    m = re.search(r"compiled stdlib: (\d+) function", out.stderr)
    if not m or int(m.group(1)) == 0:
        fail(f"{loft} declines its compiled standard library here: it was not compiled from this "
             "tree's default/*.loft, so the interpreter lane would time the slow path.  Run "
             "`make compiled-stdlib` and rebuild that binary (or pass --allow-declined-stdlib to "
             "measure the declined state on purpose).")


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
    ap.add_argument("--build-jobs", type=int, default=5,
                    help="programs compiled at the same time, each at low priority")
    ap.add_argument("--batch", type=int, default=20,
                    help="programs built before they are measured, one at a time")
    ap.add_argument("--measure-jobs", type=int, default=3,
                    help="programs measured at the same time, each pinned to its own cores of "
                         "the fastest tier; a threaded program is always measured alone")
    ap.add_argument("--build-mem-reserve-gb", type=float, default=3.0,
                    help="a build step waits while the machine has less memory available")
    ap.add_argument("--build-tsv", default="",
                    help="write each build step's wall, CPU and peak memory here")
    ap.add_argument("--package", action="append", default=[], metavar="DIR[=NAME]",
                    help="a library's own bench (DIR/bench/bench.loft + bench.rs); repeatable")
    ap.add_argument("--no-suite", action="store_true", help="measure only the --package lanes")
    ap.add_argument("--routine", action="append", default=[], metavar="[BENCH/]NAME[,…]",
                    help="report only these routines (repeatable); alone, it also picks the "
                         "programs that hold them from portal/routines.tsv")
    ap.add_argument("--max-run-ms", type=float, default=10000.0,
                    help="with --routine: the longest one run of a program may take, all its "
                         "routines together, once --n is calibrated on the picked ones")
    ap.add_argument("--no-pin", action="store_true")
    ap.add_argument("--show-samples", action="store_true", help="print every sample under its row")
    ap.add_argument("--loft", default=os.environ.get("LOFT_BIN", os.path.join(ROOT, "target/release/loft")))
    ap.add_argument("--lib-dir", default=os.environ.get("LOFT_LIB_DIR", os.path.join(ROOT, "target/release")))
    ap.add_argument("--allow-declined-stdlib", action="store_true",
                    help="measure the interpreter even when its compiled stdlib is declined")
    a = ap.parse_args()
    # The janitor runs beside the builds this starts, detached (RUN_BOUNDS.md § Scratch hygiene).
    subprocess.run([os.path.join(ROOT, "scripts", "disk_janitor.sh"), "--background"], check=False)

    lanes = [x for x in a.lanes.split(",") if x]
    unknown = set(lanes) - {"native", "rust", "interp", "python"}
    if unknown:
        fail(f"unknown lane(s): {', '.join(sorted(unknown))}")
    if not os.access(a.loft, os.X_OK):
        fail(f"no loft binary at {a.loft} (cargo build --release)")
    if "interp" in lanes and not a.allow_declined_stdlib:
        require_compiled_stdlib(a.loft)
    benches = sorted(b for b in os.listdir(HERE)
                     if b[:2].isdigit() and os.path.exists(os.path.join(HERE, b, "bench.loft")))
    a.selection = parse_selection(a.routine)
    if a.selection and not a.only and not a.package and not a.no_suite:
        picked, picked_packages = resolve_selection(a.selection)
        a.only = ",".join(picked)
        a.no_suite = not picked
        a.package = [f"{d}={n}" for n, d in picked_packages]
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
    failed, build_rows = [], []
    build_wall = measure_wall = 0.0
    batch_size = max(1, a.batch)
    for start in range(0, len(units), batch_size):
        batch = units[start:start + batch_size]
        t0 = time.monotonic()
        built = build_batch(batch, lanes, a, build_rows, failed)
        build_wall += time.monotonic() - t0
        t0 = time.monotonic()
        jobs = []
        for bench, pkg_dir in batch:
            if bench not in built:
                continue
            cmds = built[bench]
            jobs.append((bench, pkg_dir, cmds))
        for lines, recs, unit_ratios, unit_verdicts in measure_batch(jobs, a, lanes, target_us, stamp):
            for line in lines:
                print(line, flush=True)
            out_rows.extend(recs)
            ratios.extend(unit_ratios)
            for v in unit_verdicts:
                verdicts[v] += 1
        measure_wall += time.monotonic() - t0

    missed = [f"{b}/{r}" if b else r for b, r in a.selection
              if not any(selected([(b, r)], x["bench"], x["routine"]) for x in out_rows)]
    if missed:
        failed.append(("--routine", f"no measured program printed {', '.join(missed)}"))
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
    if build_rows:
        cpu = sum(r["cpu_s"] for r in build_rows)
        peak = max(r["peak_mb"] for r in build_rows)
        print(f"# built {len({r['bench'] for r in build_rows})} program(s) in {build_wall:.0f} s wall "
              f"({a.build_jobs} at a time, batches of {batch_size}), {cpu:.0f} s CPU, the largest "
              f"step {peak:,.0f} MB · measured in {measure_wall:.0f} s")
    if a.build_tsv and build_rows:
        with open(a.build_tsv, "w") as f:
            for k, v in run_metadata(a).items():
                f.write(f"# {k}={v}\n")
            f.write("bench\tstep\twall_s\tcpu_s\tpeak_mb\tjobs\tcommit\tdate\n")
            for r in build_rows:
                f.write(f"{r['bench']}\t{r['step']}\t{r['wall_s']:.2f}\t{r['cpu_s']:.2f}\t"
                        f"{r['peak_mb']:.0f}\t{a.build_jobs}\t{stamp['commit']}\t{stamp['date']}\n")
        print(f"wrote {a.build_tsv}")
    if failed:
        for bench, err in failed:
            sys.stderr.write(f"stats.py: {bench}: {err}\n" if bench == "--routine"
                             else f"stats.py: {bench} did not build — {err}\n")
        sys.exit(1)


if __name__ == "__main__":
    main()
