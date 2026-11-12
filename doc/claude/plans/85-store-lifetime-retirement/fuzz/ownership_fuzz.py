#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""
ownership_fuzz.py — @PLN85 fuzz-proof gate, first increment (the GENERATOR).

The cross-backend value/exit/leak ORACLE already exists (tests/differential_oracle.rs,
divergences()) and the leak signal is the same "stores not freed" string on both backends.
What was missing is a program-level GENERATOR feeding it over the ownership-composition axes.
This is that generator, scoped to the cheapest axis that exercises the store-lifetime class:
the CHURN / cardinality axis (loop + pressure counts), because the over-free class only
corrupts once a freed slot is REUSED (probes/over-free-sweep/README.md).

Two stages, per the perf split (native compiles via rustc — too slow for a tight loop):
  fast loop  : run each program on --interpret with leak detection  (catches interp crash + leak)
  replay     : re-run every FLAGGED program (+ a sample) on --native (cross-backend divergence + native leak)

Violation modes (the four the invariant can break into):
  CRASH       : a backend exits non-zero (139 = SIGSEGV → UAF/over-free)
  LEAK        : "stores not freed" on a backend's stderr
  DIVERGENCE  : interp stdout != native stdout, or success disagrees (silent two-owner corruption)
  WRONG       : a cell's OWN assertion failed — the answer is wrong even where the two
                backends agree about it (added 2026-08-18, loft#974: aligning the backends
                removed the disagreement that had been the only way this was visible)

Positive control (--self-test) — CONTROL PAIRS, re-pinned 2026-07-03 after the join_own
default-ON flip (and this week's ungated fixes) cleaned the P14 probe file on BOTH
configurations; re-pinned again 2026-07-04 when the #497 reassignment deep-copy closed
every crash cell even under LOFT_NO_JOIN_OWN=1 — the buggy config now disables BOTH
preservation gates (LOFT_NO_JOIN_OWN + LOFT_NO_REASSIGN_COPY).  The controls anchor on
GENERATED cells that still reproduce on the preserved raw path, one per detector channel:
  wrong/divergence : elem_accumulate__struct__heavy — gate-OFF the cell computes the wrong
                     answer; clean on the default gate.  It anchored the DIVERGENCE
                     spelling until 2026-08-18, when loft#974 made native stop masking the
                     corruption with a defensive copy: both backends then failed the same
                     assertion, the disagreement vanished, and the channel read CLEAN on a
                     cell that still reproduces.  The cell was right and the ORACLE was
                     short a channel — so `WRONG` was added rather than the control
                     re-pinned a third time;
  leak             : local_source__struct__none — both-backends LEAK gate-OFF,
                     clean on the default gate.
Each is a PAIR: (1) the buggy config must be FLAGGED (the detector can fire — not vacuous);
(2) the default gate must be CLEAN (pins the join_own fix; a flip regression fails here).
(engineering-rigor: prove the harness can fail — and that the fix it validated stays landed.)

Usage:
  ownership_fuzz.py --self-test                       # prove the harness can fail (P14)
  ownership_fuzz.py --corpus <dir>...                 # baseline: run every .loft through the oracle
  ownership_fuzz.py --corpus <dir>... --mutate 8      # + 8 churn-mutants per seed
  ownership_fuzz.py ... --native-replay               # replay flagged programs on --native too
"""
import argparse, os, re, subprocess, sys, tempfile, pathlib

LOFT = os.environ.get("LOFT", "target/release/loft")
TO_INTERP = os.environ.get("FUZZ_TIMEOUT_INTERP", "60")
TO_NATIVE = os.environ.get("FUZZ_TIMEOUT_NATIVE", "120")
LEAK_SIGNAL = "stores not freed"          # identical on both backends (src/state/mod.rs, generation/mod.rs)
# A store freed twice, as the runtime reports it on every backend (`@FR-H-FreeTwice`: on by
# default in the interpreter, and in the C output since it fails at a re-free too).  The memory
# defect this gate exists for, caught by the runtime itself rather than by a crash.
REFREE_SIGNAL = "a store was freed twice"
# The cell's own value oracle failing — loft renders it `error: assertion failed: <msg>`.
ASSERT_SIGNAL = "assertion failed"
RANGE_RE = re.compile(r"\b0\.\.(\d+)\b")  # `0..N` loop/pressure bounds — the churn axis


def normalise(s: str) -> str:
    return "\n".join(line.rstrip() for line in s.replace("\r\n", "\n").rstrip().split("\n"))


def run(mode: str, path: str):
    """Run one program on one backend; return (exit_code, normalised_stdout, leaked, wrong,
    refreed).

    `wrong` is the cell's OWN value oracle: every generated cell asserts what its shape
    must compute, so a failed assertion is a WRONG ANSWER, not a program that merely
    exited non-zero (a compile error does that too, and the grammar produces some)."""
    env = dict(os.environ)
    env["LOFT_TIMEOUT"] = TO_INTERP if mode == "--interpret" else TO_NATIVE
    # Gate hygiene: the bytecode cache key does NOT include semantic env gates
    # (LOFT_JOIN_OWN et al) — a gate-ON sweep otherwise poisons a later gate-OFF
    # sweep of the SAME probe files with stale-gate bytecode (observed: a 6/54
    # gate-OFF map read 0/54 right after a gate-ON run).  Never trust the cache.
    env["LOFT_NO_CACHE"] = "1"
    if mode == "--native":
        env["LOFT_NATIVE_LEAK_CHECK"] = "1"
    try:
        p = subprocess.run([LOFT, mode, path], capture_output=True, text=True,
                           env=env, timeout=int(env["LOFT_TIMEOUT"]) + 15)
    except subprocess.TimeoutExpired:
        return (-99, "", False, False, False)  # hang = a violation (CRASH to the caller)
    both = (p.stdout or "") + (p.stderr or "")
    # The interpreter renders it as `error: assertion failed: <msg>`; native raises it as a
    # Rust panic carrying the same message, which exits 101.  Either is the cell saying its
    # own answer is wrong.
    wrong = ASSERT_SIGNAL in both or (mode == "--native" and p.returncode == 101)
    return (p.returncode, normalise(p.stdout), LEAK_SIGNAL in p.stderr, wrong,
            REFREE_SIGNAL in both)


def is_crash(rc: int) -> bool:
    """A MEMORY crash — killed by a signal (UAF/double-free/abort), NOT a clean error exit.
    Python reports a signal as a negative returncode; a subprocess shell maps it to 128+sig.
    A clean `exit=1/2` (compile error, assert failure) is NOT a crash on its own — only a
    DIVERGENCE if the two backends disagree about it (one errors, one succeeds)."""
    return rc < 0 or rc in (132, 133, 134, 135, 136, 137, 139)  # SIGILL/TRAP/ABRT/BUS/FPE/KILL/SEGV


def judge(path: str, native_replay: bool):
    """Return a list of violation strings for `path` (empty = clean / backends-agree).

    Four channels, and the fourth was added 2026-08-18 (loft#974): a cell that fails its
    OWN assertion is WRONG, whether or not the two backends agree about it.  Until then
    the map read "both backends wrong in the same way" as CLEAN — the corruption was only
    visible while ONE backend still masked it (native's defensive copy), so ALIGNING the
    backends silenced the control cell that had anchored this channel since 2026-07-04.
    A wrong answer is the thing the whole gate exists to catch; it must not depend on a
    disagreement to be seen."""
    viol = []
    ie, iout, ileak, iwrong, irefree = run("--interpret", path)
    if is_crash(ie):
        viol.append(f"CRASH(interp signal={-ie if ie < 0 else ie - 128})")
    # A re-free the runtime reports is a memory defect on its own: once both backends report
    # it the same way, their exits agree and DIVERGENCE no longer sees it (the C backend's
    # re-free check silenced the crash control exactly so).
    if irefree:
        viol.append("REFREE(interp)")
    if ileak:
        viol.append("LEAK(interp)")
    if iwrong:
        viol.append("WRONG(interp assertion)")
    # Run native to cross-check whenever interp did anything non-clean, or when sampling.
    # (A clean interp success is only cross-checked under --native-replay — the slow stage.)
    if ie != 0 or ileak or native_replay:
        ne, nout, nleak, nwrong, nrefree = run("--native", path)
        if is_crash(ne):
            viol.append(f"CRASH(native signal={-ne if ne < 0 else ne - 128})")
        if nrefree:
            viol.append("REFREE(native)")
        if nleak:
            viol.append("LEAK(native)")
        if nwrong:
            viol.append("WRONG(native assertion)")
        if (ie == 0) != (ne == 0):
            viol.append(f"DIVERGENCE(interp exit={ie} ≠ native exit={ne})")
        elif ie == 0 and ne == 0 and iout != nout:
            viol.append("DIVERGENCE(interp stdout ≠ native stdout)")
    return viol


def mutants(src: str, n: int):
    """Yield up to n churn-mutants: scale every `0..N` bound (the slot-reuse axis)."""
    bounds = RANGE_RE.findall(src)
    if not bounds:
        return
    factors = [2, 3, 4, 6, 8, 5, 7, 10][:n]
    for f in factors:
        # cap so a fuzz run stays bounded (don't explode a 0..1000 into millions)
        yield RANGE_RE.sub(lambda m: f"0..{min(int(m.group(1)) * f, 200)}", src)


def collect(dirs):
    out = []
    for d in dirs:
        for p in sorted(pathlib.Path(d).rglob("*.loft")):
            # real files only — skip the `.loft/` cache dir (and a dir literally named `.loft`)
            if p.is_file() and not p.name.startswith(".") and "/.loft/" not in str(p):
                out.append(str(p))
    return out


def main():
    # Force UTF-8 on our own streams: the DIVERGENCE messages carry `≠` (U+2260),
    # which Windows' default cp1252 console codec cannot encode — without this the
    # positive-control self-test dies with UnicodeEncodeError instead of running.
    for stream in (sys.stdout, sys.stderr):
        reconfigure = getattr(stream, "reconfigure", None)
        if reconfigure:
            reconfigure(encoding="utf-8")
    ap = argparse.ArgumentParser()
    ap.add_argument("--corpus", nargs="*", default=[])
    ap.add_argument("--mutate", type=int, default=0)
    ap.add_argument("--native-replay", action="store_true")
    ap.add_argument("--poison", action="store_true",
                    help="run both backends under LOFT_POISON=1 (arena poison-on-free, @PLN54 S3) — "
                         "turns a SILENT store-internal use-after-free into a loud crash; strictly "
                         "stronger (caught elem_accumulate-none, which the differential alone missed)")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()

    if args.poison:
        os.environ["LOFT_POISON"] = "1"   # run() copies os.environ → both backends inherit it

    if not os.access(LOFT, os.X_OK):
        sys.exit(f"no loft binary at {LOFT} (build: cargo build --release)")

    if args.self_test:
        # Generate the two control cells fresh (deterministic grammar output).
        gen = os.path.join(os.path.dirname(os.path.abspath(__file__)), "grammar_gen.py")
        with tempfile.TemporaryDirectory() as td:
            subprocess.run([sys.executable, gen, "--out", td], check=True,
                           capture_output=True, text=True)
            crash_cell = os.path.join(td, "elem_accumulate__struct__heavy.loft")
            leak_cell = os.path.join(td, "local_source__struct__none.loft")
            # (1) buggy config — the preserved raw path (join_own opt-out) still
            # carries the class; each channel's detector MUST fire.
            # #497 added a second preservation gate (LOFT_NO_REASSIGN_COPY —
            # the static reassignment deep-copy that closed every crash cell
            # even with join_own off); the buggy config disables BOTH so the
            # raw class stays reproducible.
            os.environ["LOFT_NO_JOIN_OWN"] = "1"
            os.environ["LOFT_NO_REASSIGN_COPY"] = "1"
            # loft#1336 added a third: the owner witness closes the local_source leak on
            # its own, so the leak control must hold it off too or it reads CLEAN.
            os.environ["LOFT_NO_OWNER_WITNESS"] = "1"
            v_crash = judge(crash_cell, native_replay=True)
            v_leak = judge(leak_cell, native_replay=True)
            del os.environ["LOFT_NO_JOIN_OWN"]
            del os.environ["LOFT_NO_REASSIGN_COPY"]
            del os.environ["LOFT_NO_OWNER_WITNESS"]
            # loft#974 — the channel this cell anchors is "the program computed the wrong
            # thing", which CRASH and DIVERGENCE are only two spellings of.  WRONG is the
            # third, and it is what remained once both backends started reporting the same
            # corruption instead of one of them masking it.
            crash_fires = any(
                "CRASH" in x or "DIVERGENCE" in x or "WRONG" in x or "REFREE" in x
                for x in v_crash
            )
            leak_fires = any("LEAK" in x for x in v_leak)
            print(f"crash-control (raw path: NO_JOIN_OWN+NO_REASSIGN_COPY+NO_OWNER_WITNESS) elem_accumulate/struct/heavy: "
                  f"{v_crash or 'CLEAN'}")
            print(f"leak-control  (raw path: NO_JOIN_OWN+NO_REASSIGN_COPY+NO_OWNER_WITNESS) local_source/struct/none: "
                  f"{v_leak or 'CLEAN'}")
            # (2) fixed config — the default gate must be CLEAN on both cells.
            f_crash = judge(crash_cell, native_replay=True)
            f_leak = judge(leak_cell, native_replay=True)
            print(f"fixed-config  (default gate) elem_accumulate: {f_crash or 'CLEAN'}; "
                  f"local_source: {f_leak or 'CLEAN'}")
        ok = crash_fires and leak_fires and not f_crash and not f_leak
        if ok:
            print("SELF-TEST PASS — both detector channels fire on the preserved bug "
                  "AND the default fix holds")
        elif not (crash_fires and leak_fires):
            print("SELF-TEST FAIL — harness is VACUOUS "
                  f"(crash channel fires={crash_fires}, leak channel fires={leak_fires})")
        else:
            print("SELF-TEST FAIL — the DEFAULT gate regressed (a control cell flagged "
                  "without the opt-out)")
        sys.exit(0 if ok else 1)

    seeds = collect(args.corpus)
    print(f"# corpus: {len(seeds)} seeds  mutate={args.mutate}  native_replay={args.native_replay}")
    flagged = 0
    total = 0
    with tempfile.TemporaryDirectory() as td:
        for s in seeds:
            programs = [(s, s)]
            if args.mutate:
                src = pathlib.Path(s).read_text()
                for i, mut in enumerate(mutants(src, args.mutate)):
                    mp = os.path.join(td, f"{pathlib.Path(s).stem}.m{i}.loft")
                    pathlib.Path(mp).write_text(mut)
                    programs.append((f"{s}~m{i}", mp))
            for label, path in programs:
                total += 1
                v = judge(path, args.native_replay)
                if v:
                    flagged += 1
                    print(f"VIOLATION {label}: {', '.join(v)}")
    print(f"# done: {flagged}/{total} flagged")
    sys.exit(1 if flagged else 0)


if __name__ == "__main__":
    main()
