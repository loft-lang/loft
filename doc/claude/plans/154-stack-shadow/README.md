<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 154 — A stack shadow that knows a slot is uninitialised, the wrong width, or stale

Tracker: [@PLN154](https://github.com/loft-lang/plans/issues/154) — **CLOSED**.

## Status — SHIPPED 2026-09-06

`LOFT_VERIFY_STACK=1` reports, at the READ, a frame slot nothing wrote, a handle read as a
value (or a value read as one), and a handle whose record has since MOVED.  One tag word per
stack byte, carried by the stack `Store` itself — **tag low, check high**: the tag goes on at
`Store::addr_mut::<T>`, the check sits at `get_stack` / `get_var`, and never at `Store::addr`,
which is what the debugger and the frame renderer read stale slots with on purpose.

Calibrated both ways: silent on all **1106** runnable corpus programs at HEAD, and reporting on
the build each named control was written to catch.  `LOFT_VERIFY_STACK_INJECT=1` is the positive
control — 14 distinct sites on a three-line program, none without it.  Armed in the nightly at
~2× the in-process interpreter corpus.

**The reference now lives outside this plan:**

- [DEBUG.md](../../DEBUG.md) § the detector table — the three env levers
  (`LOFT_VERIFY_STACK`, `_TRACE`, `_INJECT`), what each finding means, and the
  `LOFT_UAF_GEN` / `LOFT_STRICT_STORES` / `LOFT_POISON` division of labour this one joins.
- [CI_BUDGET.md](../../CI_BUDGET.md) — the `stack-shadow` nightly job, its cost, and the
  `needs` rule that put it in the failure-notify list.

The phase docs below stay as the closure record: each holds its own measurement, including the
three phases whose target moved under them.

## What shipped, per phase

| # | Phase | Outcome |
|---|---|---|
| **0** | [bypass census](phase0-census.md) | **RED, as a design probe should be.**  `put_stack` carries 74.5 % of stack bytes and is 1 of 33 write sites; **no** corpus program is covered by it alone.  A phase-1 check keyed to the accessor would have reported `OpPutInt` — the commonest assignment in the language — as a read of an unwritten slot.  The tag moved down to `Store::addr_mut::<T>`, which 32 of the 33 sites already call and which carries the type phase 2 needs |
| **1** | [`uninit`](phase1-uninit.md) | **GREEN, target moved.**  Neither loft#1386 nor loft#1254 is in the `uninit` state; the witness is the nullable-local pre-init defect instead (4 sites on control `64437246`).  An eval slot is stepped to 8 bytes while a `boolean` writes 1, so a slot is nearly always recycled with *something* in it and pure absence is a narrow state — the `Partial` reads it declines to report became phase 2's queue |
| **2** | [`width + kind`](phase2-width-kind.md) | **GREEN on 2 of 3, on the HANDLE axis.**  loft#1028's control reports `handle 12` read as `i64`; loft#1016's reports four sites.  The width axis is **counted, not reported** — a strict rule fired on 43 of the first 180 corpus programs and every class was the frame's own composite layout.  loft#1070 is **out of reach, measured**: its control reproduces and the shadow is silent, because the wrong layout is in a heap RECORD and the slot holds a correct handle |
| **3** | [`stale-on-grow`](phase3-stale.md) | **GREEN.**  loft#1373 / #1377 / #1384 each report exactly ONE site, the stale view.  `Store::resize` logs the move and the dispatch loop walks only the slots the shadow already says are a handle base, so the scan is exact rather than a guess at which aligned words are references.  Five-probe matrix ([probes/](probes/README.md)) with a no-relocation negative control |
| **4** | [yield vs. the falsification corpus](phase4-yield.md) | **Driver shipped; the full run DEFERRED** — see below.  The GATE direction is established on every row the sample produced: not one report on a build its own guard calls clean |
| **5** | [arming it](phase5-nightly.md) | **Armed and green.**  `stack-shadow` in `.github/workflows/miri.yml`, in the `notify` list on the stated rule — a gate absent from `needs` reads as green and auto-closes its issue, so a gate whose finding means *the language is broken* goes in with the gate |

## Deferred — phase 4's full yield run

The 264 guards carrying a real `@falsified-at:` ref span **200 distinct builds**: hours of
machine time, not a session.  What the plan closes without is the **yield** number — how many
already-known defects the shadow would have caught.  That is a REPORT, never a threshold, and
the sample already established why it is thin:

> **The corpus's biggest ref-clusters are TYPING defects.**  The two largest — `8498fdf1` (ten
> guards) and `964bab93` (nine) — are the nullable-model and value-position-`match` families,
> where the control build REFUSES the program and no operator ever runs.  A memory-state shadow
> cannot speak about a program that does not execute.

The **gate** direction — the one that protects — needs no full run: phase 5's nightly enforces
it continuously, and a false positive there is red.

⚠ **The driver's ordering is known to be wrong for this purpose and was left that way on
purpose.**  `phase4-yield.sh` takes refs by coverage so a partial run is reproducible and
explainable; but coverage order puts the refusal families first, so it buys the *least*
evidence.  Anyone resuming this should re-order by *guards that actually RUN, whose defects are
memory-shaped* before spending the machine time.  Three vacuity traps are recorded in
[phase4-yield.md](phase4-yield.md) — the wrong entry point (16 of the first 48 rows scored on a
run that never happened), a cached binary pointed at a deleted worktree, and the missing
positive control — and all three are fixed in the driver.

## Not delivered — the Stage A composition matrix

The plan opened with a five-axis composition matrix (slot zone · write path · residence · value
kind · origin) and claimed it would also close
[STABILITY_SWEEP.md](../../STABILITY_SWEEP.md) F5's recorded deferral, *"full odd-size adjacency
matrix"*.  **It was never built** — phases 1–3 were validated by corpus sweeps, control builds
and phase 3's own five-probe matrix instead.  **F5's deferral therefore stands**, and the
zone-1 slot-reuse and `LOFT_ALIGN` padding cells named there are still unmeasured from this
side.

## See also

- [shadow-control.sh](shadow-control.sh) — build a control tree WITH the shadow on it, which is
  what phase 4 needs and what `make falsify` cannot do.
- [SLOTS.md](../../SLOTS.md) — the frame layout the shadow mirrors.
- [TESTING.md](../../TESTING.md) § *A guard that never failed is not a guard* — `make falsify`
  and the `@falsified-at:` corpus phase 4 measures against.
- [@PLN154](https://github.com/loft-lang/plans/issues/154) — the tracker issue, which carries
  the evidence per state and the out-of-scope list (a static bytecode verifier; owner-vs-view
  tagging).
