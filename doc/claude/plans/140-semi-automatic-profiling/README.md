<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN140 — Semi-automatic profiling

**Status — SHIPPED.** All four arcs: **A** allocation hot spots at the peak · **B** a
loft-level CPU profiler · **C** allocation paths · **D** the driver and its honesty
guards. Tracker: [loft-lang/plans#140](https://github.com/loft-lang/plans/issues/140).

`make profile` answers *which loft function or line is hot*, *which loft line allocated
the heap*, and *what path reached each* — and declines to present a profile it can tell
is dishonest. **That refusal is the load-bearing half:** every trap this tooling hit
produced a *plausible* profile, not an obviously broken one, so ranking faster without
checking honesty only reaches the wrong conclusion sooner.

## Where the reference content lives now

| What | Home |
|---|---|
| The user-facing guide — every switch, both profilers, the two ways a profile can lie, `--mem`, sample counts, the five choices that make a profile honest | [PERFORMANCE.md § Profiling a run](../../PERFORMANCE.md) |
| What each instrument **must** say, per corpus row, and what the corpus cannot prove | [PROFILE_ORACLE.md](../../PROFILE_ORACLE.md) |
| The sampling mechanism and its stated blind spots | `src/profiler.rs` module doc |

The instruments have grown since this plan closed and the docs above are the current
account, not this file: `LOFT_PROFILE_EVERY` and the signal flush
([loft#1089](https://github.com/loft-lang/loft/issues/1089)) made a program whose only
exit is a `kill` profilable, and the network profiler joined the same flush
([loft#1088](https://github.com/loft-lang/loft/issues/1088)).

## The result that decided the plan

The interpreter's CPU cells were called **structurally impossible** when this plan
opened, and that reasoning was right about `perf` while being wrong about the
conclusion. A loft call creates no machine frame, so perf's stack walk yields the
interpreter's own path — identical for every program ever run. No sampling frequency
fixes it: it is the **wrong stack**, not a truncated one. The answer was never to sample
it better but to sample the stack **loft already keeps** (`State::call_stack`).

Two design answers came out of that and are the reason the instrument is trusted:

- **The op counter chooses *when* to sample; a wall clock says *how much*.** Each sample
  carries the nanoseconds since the previous one, so a millisecond inside one heavy
  native op arrives as a millisecond. Measured: nothing when off, +7–11 % armed.
- **The period must be JITTERED.** A fixed one samples a single phase of a periodic
  program — the arc C oracle allocates down two paths in a known 9:1 ratio, and a fixed
  every-16th sampler put **100 %** on one and never once saw the other. Not noisy;
  confidently wrong, and no sample count would have shown it.

## Backend × instrument — every cell green or explicitly declined

| | `--interpret` | `--native` |
|---|---|---|
| CPU hot spot names a loft fn | ✓ | ✓ |
| Path to the CPU hot spot | ✓ over loft frames | ~ real, truncates above `start_thread` |
| Allocation site names a loft line | ✓ live, at the peak, in bytes | **declined** |
| Path to an allocation | ✓ sampled | **declined** |
| Peak vs exit | ✓ captured at the peak | **declined** |

**The decline is enforced, not merely stated.** The allocation site *is* the
interpreter's bytecode position, republished per op by the dispatch loop; a native binary
has no dispatch loop, so every store carries site 0 and the report would be a single row
reading `line 0` — a table with the shape of an answer and none of the content. So
`--mem` **refuses** on a native run and points at `--interpret`, which allocates from the
same loft lines.

## What is not done

- **The usability half of the bar.** The corpus proves the instruments are *correct*;
  only a grown consumer proves the reports are *readable* — that a thousand-site report
  still fits on a screen and ranks the right thing on top. The ranking and roll-up are
  built for it (top 12 sites then a by-function roll-up, innermost-8 path folding, a
  stated `MAX_PATHS` cap that reports what it dropped) but they have not met a program
  that needs them.
- **`par` worker threads are not sampled** — a worker runs its own `State`. The report
  says so rather than presenting the main thread's share as the whole.
- **Text buffers are not in the memory ledger** — they are Rust `String`s, not stores, so
  `bench/07_string_build`'s 5.9 MB is invisible. The report says so.

`bench/06_newton_sqrt` was on this list and has come off it: it could not run under
`--interpret` at all, because `guess = (guess + x / guess) / 2.0` was refused as a
`float` → `float?` type change and every cure the error named failed. That was a language
issue found while picking oracles, not a profiling one, and closing
[loft#859](https://github.com/loft-lang/loft/issues/859) gave `06` an oracle row like
every other program.

A **retention** report — *why is this store still alive* — is loft's stronger analogue to
a heap walker's paths-to-GC-root, because ownership is recorded rather than inferred. It
belongs to [OWNERSHIP_MODEL.md](../../OWNERSHIP_MODEL.md) /
[LIFETIME.md](../../LIFETIME.md), and is noted here so it is not rebuilt inside a
profiling plan.

## See also

- [STACKTRACE.md](../../STACKTRACE.md) — `call_stack` / `CallFrame`, the vector arc B samples.
- [DEBUG.md](../../DEBUG.md) — the per-op debugger hook arc B piggybacks on.
- [RUN_BOUNDS.md § Store-memory ceiling](../../RUN_BOUNDS.md) — the at-peak trigger arc A copies.
