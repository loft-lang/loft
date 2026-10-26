#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The C backend against the interpreter, over the corpus (@PLN200 B5).

    scripts/c_sweep.py                  # every program scripts/corpus_files.sh lists
    scripts/c_sweep.py FILE...          # these programs
    scripts/c_sweep.py --verbose        # also every differing program's two outputs
    scripts/c_sweep.py --asan           # build with AddressSanitizer and UBSan: a report is a defect
    scripts/c_sweep.py --all            # every refusal reason, not the top 25
    scripts/c_sweep.py --mode default   # one macro mode (strict, default, verify) instead of all
    scripts/c_sweep.py --shard K/N      # every N-th program from offset K (CI splits the sweep)
    scripts/c_sweep.py --stderr         # also compare stderr: a fault's message, its `-->` line,
                                        # excerpt and call chain (the oracle runs --no-warnings)
    scripts/c_sweep.py --poison         # the store poison modes instead: the C built with
                                        # -DLOFT_POISON / -DLOFT_POISON_CLAIM against the
                                        # interpreter run under LOFT_POISON=1 / LOFT_POISON_CLAIM=1
    scripts/c_sweep.py --wasm           # the C built for wasm32-wasi by clang and run under
                                        # wasmtime instead (LOFT_WASM_CC names another clang)
    LOFT_BIN=<loft> scripts/c_sweep.py  # another build (default target/release/loft)

Every run also files each program's times (bench/portal/local/c_sweep/<modes>.tsv) and lists
the programs much slower than the last sweep of the same modes, and the compiled ones slower
than the interpreter — a report beside the verdict, never part of it.  Those times also set
each run's time limit: three times its last (and at least ten seconds more); a run past it is
killed and counted `slow`, so a hang shows in seconds.  `--no-limit` runs everything to the
300 s bound.

For each program: `loft --emit-c` writes it as C, `cc` builds it in each macro mode of
src/c/loft_cc.h — `-DLOFT_STRICT` (pure C99, `-pedantic -Werror`), the default, and
`-DLOFT_VERIFY` (every proof the generator relied on checked again) — and each build's stdout
and exit status are compared with `loft --interpret`'s (a printed `store_memory()` report's
lines aside: they name the interpreter's own stores); a program without `main` is its tests,
and the C's exit status is compared with `loft --tests`'s.  A mode changes speed, never a
value, so a difference between modes is a defect of a macro's fallback.  Every program lands
in one bucket:

  match     the backend carries it and the C answers what the interpreter answers
  differ    carried, and the answers differ — a defect of the C backend
  cc-error  carried, and the C does not compile — a defect of the C backend
  sanitizer (--asan) carried, and the C program trips a sanitizer — a defect of the C backend
  refused   not carried yet; the reason is counted, and the ranking is the next work

Exit 1 when any program differs or fails to compile, else 0.  Needs `cc` on PATH.
"""
import concurrent.futures
import os
import re
import subprocess
import sys
import tempfile
import time
import shutil
import zlib
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LOFT = os.environ.get("LOFT_BIN") or str(ROOT / "target" / "release" / "loft")
CC = ["cc", "-std=c99", "-O1", "-DLOFT_CALL_CHAIN", "-pthread"] + (["-ldl"] if sys.platform.startswith("linux") else [])
MODES = {
    "strict": ["-DLOFT_STRICT", "-pedantic", "-Werror"],
    "default": ["-w"],
    "verify": ["-DLOFT_VERIFY", "-w"],
}
# The store's own falsifiers, which a sanitizer cannot be: a store is one allocation, so a read
# of a freed store, of a record a growth moved, or of a claim nothing wrote is valid memory to
# it.  Each mode is the C's build flag beside the switch the interpreter honours the same way.
POISON_MODES = {
    "poison": (["-DLOFT_POISON", "-w"], {"LOFT_POISON": "1"}),
    "poison-claim": (["-DLOFT_POISON_CLAIM", "-w"], {"LOFT_POISON_CLAIM": "1"}),
}
ASAN = ["-g", "-fsanitize=address,undefined", "-fno-omit-frame-pointer", "-fno-sanitize-recover=all"]
# `--wasm`: the same unit for wasm32-wasi.  The runtime's recoverable faults use setjmp, which
# wasm has through the exception-handling proposal (clang's `-wasm-enable-sjlj` in its standard
# encoding — clang 22's default is the legacy `try`, which wasmtime refuses — wasmtime's
# `exceptions`, wasi-libc's libsetjmp); `clock` and `signal` are WASI's emulation libraries.  Needs clang, lld,
# wasi-libc and the wasm32 compiler-rt (Ubuntu: clang lld wasi-libc libclang-rt-dev-wasm32).
WASM_CC = [os.environ.get("LOFT_WASM_CC") or "clang", "--target=wasm32-wasi", "-std=c99", "-O1", "-w",
           "-mllvm", "-wasm-enable-sjlj", "-mllvm", "-wasm-use-legacy-eh=false",
           "-D_WASI_EMULATED_PROCESS_CLOCKS", "-D_WASI_EMULATED_SIGNAL"]
# wasm-ld reserves a 64 KiB stack by default; a native program has Linux's 8 MiB, and a deep
# recursion that fits there ran off the bottom of the wasm one (a fault at an address past the
# end of linear memory).
WASM_LIBS = ["-Wl,-z,stack-size=8388608", "-lm", "-lsetjmp", "-lwasi-emulated-process-clocks",
             "-lwasi-emulated-signal"]


def wasm_run(wasm: str, cwd: Path, tmpdir: str) -> list[str]:
    """wasmtime over `wasm`, with the program's directory reachable both as `.` and under its
    own absolute path — resolved, as the runtime normalises the directory it anchors a relative
    path at — the shared temporary directory and the program's own (`tmpdir`, its TMPDIR)
    under their own paths, and the host's environment, as a native run reaches them."""
    tmp = os.path.realpath(tempfile.gettempdir())
    cwd = cwd.resolve()
    # The host's environment, every variable inherited as a native run inherits it.
    env = [a for k in sorted(os.environ) if k not in ("TMPDIR", "LOFT_SOURCE_DIR") for a in ("--env", k)]
    return ["wasmtime", "-W", "exceptions=y", "--dir=.", f"--dir={cwd}::{cwd}",
            f"--dir={tmp}::{tmp}", f"--dir={tmpdir}::{tmpdir}", "--env", f"TMPDIR={tmpdir}",
            "--env", f"LOFT_SOURCE_DIR={cwd}",
            *env, wasm]
HAS_MAIN = re.compile(r"^\s*(pub\s+)?fn\s+main\s*\(", re.M)


def corpus() -> list[str]:
    r = subprocess.run([str(ROOT / "scripts" / "corpus_files.sh")], capture_output=True,
                       text=True, cwd=ROOT)
    return [l for l in r.stdout.split("\n") if l.strip()]


# The interpreter is the oracle, and the slowest of the runs: a program near a minute on it
# (tests/scripts/2177-…, ~60 s alone) timed out under a full sweep's load and read as a differ.
ORACLE_TIMEOUT = 300


def run(cmd, timeout, cwd=None, env=None):
    try:
        r = subprocess.run(cmd, capture_output=True, timeout=timeout, cwd=cwd,
                           env={**os.environ, "LOFT_TIMEOUT": str(timeout), **(env or {})})
        return r.returncode, r.stdout, r.stderr
    except subprocess.TimeoutExpired:
        return "timeout", b"", b""


def timed(cmd, timeout, cwd=None, env=None):
    """`run`, and the wall seconds it took (the whole timeout for one that ran out)."""
    t0 = time.monotonic()
    out = run(cmd, timeout, cwd=cwd, env=env)
    return out, (timeout if out[0] == "timeout" else time.monotonic() - t0)


def reason(stderr: bytes) -> str:
    """The refusal without the function's name: `name: why` → `why`, numbers folded."""
    line = stderr.decode(errors="replace").strip().split("\n")[-1]
    why = line.split("yet: ", 1)[-1]
    why = why.split(": ", 1)[-1] if ": " in why else why
    return re.sub(r"\d+", "N", why)


REPORT = re.compile(rb"^stores: \d+ live \|.*(\n  #\d+ .* bc:\d+)*$", re.M)


def unreported(out: bytes) -> bytes:
    """A program's stdout without the lines of a printed `store_memory()` report: they name
    the interpreter's own stores (its value stack, its constants) and its bytecode positions,
    which no compiled program has.  The programs that read the report's counts check those
    themselves."""
    return REPORT.sub(b"<store report>", out)


# The lines only the interpreter's process writes: Rust's panic preamble around a fault it
# reports through a panic (the store ceiling), the backtrace hint after it, and the compile-time
# `advice:` notes the driver prints before a program runs (a `--native-c` run prints them too,
# from the driver, never from the binary the sweep runs).
INTERP_ONLY = re.compile(
    rb"^(thread '.*' \(\d+\) panicked at .*|note: run with `RUST_BACKTRACE.*|advice: .*)\n", re.M)


def scrubbed(err: bytes) -> bytes:
    return INTERP_ONLY.sub(b"", err).strip(b"\n")



# How long a run may take: three times what the same program took in the same mode in the
# last sweep of the same modes (the times report_times files), and never less than that time
# plus SLOW_FLOOR seconds — a full sweep runs one program per core, and a 20 ms program is not
# hung at 60 ms.  A run killed at that limit is bucketed `slow`, apart from a wrong answer,
# and its time is not filed, so the next sweep keeps the old limit.  No history, or
# `--no-limit`: ORACLE_TIMEOUT.
SLOW_FACTOR = 3.0
SLOW_FLOOR = 10.0
LAST: dict = {}
KILLED: set = set()
# The sweep's scratch (scripts/run_scratch.py): a directory per program — its unit, binary and
# TMPDIR — kept after the sweep for the programs that did not match, removed by the next sweep.
SCRATCH = None


def limit(path: str, mode: str) -> float:
    t = LAST.get((path, mode))
    return ORACLE_TIMEOUT if t is None else min(ORACLE_TIMEOUT, max(SLOW_FACTOR * t, t + SLOW_FLOOR))


def killed(path: str, mode: str, out) -> str:
    """The `slow` reason for a run its limit stopped, or "" for any other outcome."""
    if out[0] != "timeout" or limit(path, mode) >= ORACLE_TIMEOUT:
        return ""
    KILLED.add((path, mode))
    return f"[{mode}] killed at {limit(path, mode):.1f}s; its last sweep took {LAST[(path, mode)]:.2f}s"


def one(path: str, asan: bool = False, modes=tuple(MODES), stderr: bool = False):
    src = ROOT / path
    # A directory per program, named for it; `main` drops the ones that matched or were
    # refused and reports the others' (a slow or differing program's C, binary and files).
    tmp = os.path.join(SCRATCH or tempfile.gettempdir(),
                       f"{Path(path).stem[:60]}-{zlib.crc32(path.encode()):08x}")
    shutil.rmtree(tmp, ignore_errors=True)
    os.makedirs(tmp)
    # The program's own temporary directory, the same for every run of it: a program
    # that writes to TMPDIR then neither sees nor leaves files in the shared one — whose
    # size a WASI run pays for on every new name (`@FR-Path-Case` lists the directory a
    # missing name would go in, and wasmtime stats each entry it lists).
    tmpdir = os.path.realpath(os.path.join(tmp, "tmp"))
    os.mkdir(tmpdir)
    tenv = {"TMPDIR": tmpdir}
    c = os.path.join(tmp, "p.c")
    exe = os.path.join(tmp, "p")
    code, _, err = run([LOFT, "--emit-c", c, str(src)], 60, cwd=src.parent)
    if code == 2:
        return path, "refused", reason(err), None, {}
    if code != 0:
        return path, "refused", "the program does not compile in loft", None, {}
    unit = Path(c).read_text(encoding="utf-8", errors="replace")
    # A `[c] shim` of several sources is a static archive the unit names (`/* link: … */`).
    links = re.findall(r"^/\* link: (.*) \*/$", unit, re.M)
    # A shared library the unit links is found at run time in its own directory.
    links += sorted({f"-Wl,-rpath,{os.path.dirname(l)}" for l in links
                     if l.endswith((".so", ".dylib")) or ".so." in l})
    # A wasm32 build compiles the shims' sources, which the unit names, instead of linking
    # the host archives; a declared system library has no wasm32 build.
    shim_sources = re.findall(r"^/\* shim-source: (.*) \*/$", unit, re.M)
    if "wasm" in modes and any(not l.endswith(".a") and not l.startswith("-Wl,") for l in links):
        return path, "refused", "a declared C library has no wasm32 build", None, {}
    tests = not HAS_MAIN.search(src.read_text(encoding="utf-8", errors="replace"))
    # A program without `main` is its tests: the C runs them as `loft --tests` does,
    # and the two exit statuses say whether every test passed.
    interp = [LOFT, "--tests" if tests else "--interpret"] + \
        (["--no-warnings"] if stderr else []) + [str(src)]
    i_plain = None
    times = {}
    for mode in modes:
        flags, env = POISON_MODES.get(mode, (MODES.get(mode), None))
        if env:
            i_out, times["interp"] = timed(interp, limit(path, "interp"), cwd=src.parent,
                                           env={**env, **tenv})
        else:
            i_out = i_plain
        if i_out is None:
            i_out, times["interp"] = timed(interp, limit(path, "interp"), cwd=src.parent, env=tenv)
            i_plain = i_out
        why = killed(path, "interp", i_out)
        if why:
            return path, "slow", why, None, times
        if mode == "wasm":
            code, _, err = run(WASM_CC + [c] + shim_sources + ["-o", exe + ".wasm"] + WASM_LIBS, 120)
        else:
            code, _, err = run(CC + flags + (ASAN if asan else []) + [c, "-o", exe, "-lm"] + links, 120)
        if code != 0:
            first = err.decode(errors="replace").strip().split("\n")[0]
            return path, "cc-error", f"[{mode}] {first}", None, times
        # The program's directory, as the driver hands it to every run (`LOFT_SOURCE_DIR`).
        c_out, times[mode] = timed(wasm_run(exe + ".wasm", src.parent, tmpdir) if mode == "wasm" else [exe],
                                   limit(path, mode), cwd=src.parent,
                                   env={**tenv, "LOFT_SOURCE_DIR": str(src.parent.resolve())})
        why = killed(path, mode, c_out)
        if why:
            return path, "slow", why, None, times
        if asan and (b"Sanitizer" in c_out[2] or b"runtime error:" in c_out[2]):
            line = next((l for l in c_out[2].decode(errors="replace").split("\n")
                         if "Sanitizer" in l or "runtime error:" in l), "")
            return path, "sanitizer", f"[{mode}] {line.strip()[:200]}", None, times
        same = c_out[0] == i_out[0] if tests else \
            (c_out[0], unreported(c_out[1])) == (i_out[0], unreported(i_out[1]))
        if same and stderr and not tests:
            same = scrubbed(c_out[2]) == scrubbed(i_out[2])
            # A stderr naming what changes per run (a temporary file built from the time
            # and the pid) cannot match: when the interpreter's own second run says
            # something else, the stream is not compared.
            if not same:
                again = run(interp, ORACLE_TIMEOUT, cwd=src.parent, env={**(env or {}), **tenv})
                same = scrubbed(again[2]) != scrubbed(i_out[2])
        if not same:
            return path, "differ", f"[{mode}]", (i_out, c_out), times
    return path, "match", "", None, times


TIMES = ROOT / "bench" / "portal" / "local" / "c_sweep"


def load_times(label: str) -> dict:
    """The times the last sweep of these modes filed, by (program, mode)."""
    path = TIMES / f"{label}.tsv"
    last = {}
    if path.exists():
        for line in path.read_text().splitlines()[1:]:
            parts = line.split("\t")
            if len(parts) == 3:
                last[(parts[0], parts[1])] = float(parts[2])
    return last


def report_times(timings: dict, label: str) -> None:
    """Every program's run times, filed per mode set in the checkout's local results
    (`bench/portal/local/c_sweep/<modes>.tsv`, never committed), and two lists read off them —
    a REPORT, never a verdict: the programs whose C run got much slower than the last sweep
    of the same modes ran it (3x and a second more), and the compiled programs slower than
    the interpreter (3x and two seconds past it), which a compiled build should never be.  A
    timeout counts as the whole limit.  A sweep over a subset of files keeps the other
    programs' last times."""
    path = TIMES / f"{label}.tsv"
    last = load_times(label)
    now = dict(last)
    for prog, times in timings.items():
        for mode, secs in times.items():
            if (prog, mode) not in KILLED:
                now[(prog, mode)] = secs
    slower, past_interp = [], []
    for prog, times in timings.items():
        interp = times.get("interp")
        for mode, secs in times.items():
            if mode == "interp":
                continue
            before = last.get((prog, mode))
            if before is not None and secs > 3 * before and secs > before + 1.0:
                slower.append((secs / max(before, 1e-3), prog, mode, before, secs))
            if interp is not None and secs > 3 * interp and secs > interp + 2.0:
                past_interp.append((secs / max(interp, 1e-3), prog, mode, interp, secs))
    TIMES.mkdir(parents=True, exist_ok=True)
    path.write_text("program\tmode\tseconds\n" + "".join(
        f"{p}\t{m}\t{v:.3f}\n" for (p, m), v in sorted(now.items())))
    for title, rows in (("slower than the last sweep", slower),
                        ("slower than the interpreter", past_interp)):
        if rows:
            print(f"{title} ({len(rows)}):")
            for factor, prog, mode, a, b in sorted(rows, reverse=True)[:15]:
                print(f"  {factor:7.1f}x  [{mode}] {prog}  {a:.2f}s -> {b:.2f}s")


def main(argv: list[str]) -> int:
    if "-h" in argv or "--help" in argv:
        print(__doc__)
        return 0
    verbose = "--verbose" in argv
    modes = tuple(POISON_MODES) if "--poison" in argv else tuple(MODES)
    if "--wasm" in argv:
        modes = ("wasm",)
    if "--mode" in argv:
        i = argv.index("--mode")
        modes = (argv[i + 1],)
        del argv[i:i + 2]
    shard = None
    if "--shard" in argv:
        i = argv.index("--shard")
        k, _, n = argv[i + 1].partition("/")  # `K/N`, not a path
        k, n = int(k), int(n)
        shard = (k, n)
        del argv[i:i + 2]
    files = [a for a in argv[1:] if not a.startswith("--")] or corpus()
    if shard:
        # Every n-th program from offset k of the sorted list: each program in exactly one
        # shard, so n runners together sweep the whole corpus.
        files = [f for i, f in enumerate(sorted(files)) if i % shard[1] == shard[0]]
    global SCRATCH
    sys.path.insert(0, str(ROOT / "scripts"))
    from run_scratch import run_root
    SCRATCH = run_root("c-sweep")
    label = "-".join(modes) + ("-asan" if "--asan" in argv else "")
    if "--no-limit" not in argv:
        LAST.update(load_times(label))
    buckets = Counter()
    reasons = Counter()
    bad = []
    timings = {}
    with concurrent.futures.ThreadPoolExecutor(max_workers=os.cpu_count() or 4) as ex:
        asan = "--asan" in argv
        stderr = "--stderr" in argv
        for path, bucket, why, outs, times in ex.map(lambda f: one(f, asan, modes, stderr), files):
            timings[path] = times
            buckets[bucket] += 1
            if bucket in ("match", "refused"):
                shutil.rmtree(os.path.join(SCRATCH, f"{Path(path).stem[:60]}-{zlib.crc32(path.encode()):08x}"),
                              ignore_errors=True)
            if bucket == "refused":
                reasons[why] += 1
            elif bucket in ("differ", "cc-error", "sanitizer", "slow"):
                bad.append((path, bucket, why, outs))
    total = len(files)
    print(f"C backend over {total} programs: " + ", ".join(
        f"{buckets[b]} {b}" for b in ("match", "differ", "cc-error", "sanitizer", "slow", "refused")))
    for path, bucket, why, outs in sorted(bad):
        print(f"  {bucket:8} {path}" + (f" — {why}" if why else ""))
        print(f"           {os.path.join(SCRATCH, f'{Path(path).stem[:60]}-{zlib.crc32(path.encode()):08x}')}")
        if verbose and outs:
            (ic, io, ie), (cc, co, ce) = outs
            print(f"      interpreter exit {ic}: {io[:300]!r}")
            print(f"      C           exit {cc}: {co[:300]!r}")
            if "--stderr" in argv:
                print(f"      interpreter stderr: {scrubbed(ie)[:400]!r}")
                print(f"      C           stderr: {scrubbed(ce)[:400]!r}")
    report_times(timings, label)
    if reasons:
        print("refused, by reason:")
        for why, n in reasons.most_common(None if "--all" in argv else 25):
            print(f"  {n:5}  {why}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
