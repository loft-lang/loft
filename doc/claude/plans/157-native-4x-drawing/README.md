<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 157 — Native within 4× of Rust on the drawing pass

## Status

**FINISHED (closed 2026-09-25).**  The goal holds: on the x86-64 lane (`compare.py --repeat 3
--n-ref 500 --n-native 500`, 14/14 hashes agree) every judged row is under the 4× bar, median
1.42×, the highest `parse` at 2.63× after `(R-LazySplit)`; the arm64 lane agrees (median ≈1.71×,
every row under 4×).  The library's loft source is unchanged, every rewrite is a rule in
[formal/rewrites.md](../../formal/rewrites.md) with its switch and falsifier, and P5 made the
lane the per-library standard ([LIBRARY_CHECKLIST.md § Goal G](../../LIBRARY_CHECKLIST.md)).
loft#1426 is closed.  The rest of § Phase ordering's queue — frame-local record temporaries,
runtime ownership at calls, the vector append path, LTO into the rlib, range proofs for
sentinel elision, `parse`'s store traffic — is general performance work, not this pass's bar,
and moved to **@PLN158** (the release performance pass).  Performance work goes on; what this
plan leaves behind for it is the infrastructure — the same-hash reference lane and
`bench/stats.py`, `make perf-portal`, the native checkpoint profiler, the release-pass ceiling,
the emission audit, and one bisect switch plus one falsifier per rewrite.  What follows is the
record.

> **This is the one home for the scoreboard.**
> [loft#1426](https://github.com/loft-lang/loft/issues/1426) is the *surfaced report* —
> what crawler hit, and whether it is resolved for them — and it carries `status:planned`
> pointing here.  Its table is the FILED baseline, frozen as evidence and not
> maintained.  **Do not copy per-row numbers back into the issue**: they were in both places and
> drifted twice in one day (one taken on a branch, one on a join), and a wrong attribution had to
> be corrected in two places.  Report progress by editing this section; comment on the issue only
> to tell the consumer something they need — a row crossing the bar, or the class closing.

**The record** — the progress log as it was written (every unit, its numbers and what it
found), the `smooth` and `fronds` run-downs, the session hand-offs, the sub-arc table, the
joined-tree verification, the phase queue and the open design questions — is
[HISTORY.md](HISTORY.md), a pure move out of this file on 2026-09-28: history apart from the
state, so the state stays readable.  Nothing in it is maintained; the live performance work
is @PLN158 (`bench/portal/analysis/`) and @PLN174.

## Goal

loft-native within **4×** of plain Rust on every judged row of the drawing
performance pass, with the library's loft source unchanged and every row's
hash unchanged.

**And, since 2026-09-09, the FORMAL RULING of what the native rewrites assume
(owner's steer):** every rewrite the plan ships is a rule in
[formal/rewrites.md](../../formal/rewrites.md) — the hoist STATE they compose
through (`R-State`, `R-Refresh`, `R-Alias`) and one rule per rewrite, each with
its switch, its falsifier and its sites — and the emitted routines are VALIDATED
against those assumptions by two instruments: the checking forms at run time
(`LOFT_HOIST_VERIFY=1`) and the emission audit at emission time (§ V-r, queued
first).  The emitted Rust grows with every unit; a unit is not shipped until
its assumptions are written as a rule and checkable by both.

## Effort + design

- **Effort:** H total (P1 S · P2 S · P3 M · P4 L · P0/P5 XS)
- **Design:** ✓ — [DESIGN.md](DESIGN.md): per-phase invariant, code sites,
  claims + falsifying probes, predicted numbers
- **Last touched:** 2026-09-11 (§ V-t … § V-w)


## Composition matrix — Stage A

No new composition surface: every phase changes what the native backend
*emits or links* for programs that already run, never what they compute.  The
constitutive gate is therefore identity, not a new matrix: the drawing pass's
14 output hashes (P0) plus the full both-backend suite must be unchanged by
every phase, and each phase adds `--emit`-level probes (counts of the removed
form in the generated Rust) per the loft-codegen skill's byte-comparison
discipline.  P3/P4 change emitted forms of existing ops — their per-phase
sections in DESIGN.md name the cells (nullable × non-null operands, read ×
write, local × field-reached vectors) that must stay green on both backends.


## Cross-arc dependencies

- **PERFORMANCE.md § Design: P8** (store-effect classifier) — a sibling, not
  a blocker: P4 extends hoist.rs's own allow-list classification (the doctrine
  P8 endorses) rather than importing the parser's deny-list; P8's "one home
  for the leaf set" remains the longer-term convergence point.
- **PERFORMANCE.md § Design: N1/N2/N3/N4/N5** — this plan implements the
  N-class from a measured consumer workload; those design entries get
  status updates as phases land.  **N4 SHIPPED 2026-09-07** (structural
  leaf inference; `hash` −36 %, ~2.3× Rust; `LOFT_NO_LEAF_PRELUDE`
  bisects), alongside the lean-tier prelude (`cr_call_push_lean`).
- **loft#885 hoist** (`src/generation/hoist.rs`, `LOFT_HOIST_VERIFY`,
  `LOFT_NO_VECTOR_HOIST`, `LOFT_NO_ELEM_FUSE`) — P4 extends it; its
  verify/bisect switches are the safety instrument.
- **@PLN85** (finished) — the ownership/representation fact N1's old design
  waits on; P4's hoist-shaped scope is chosen to not need it.
- **@PLN140** profiler — `make profile PROFILE_FLAGS=--engine` attributes
  native time when a phase's number does not move as predicted.


## See also

- [loft#1426](https://github.com/loft-lang/loft/issues/1426) — the source
  issue; its comments carry the measured attribution (M0–M4) and baseline.
- [`@PLN157`](https://github.com/loft-lang/plans/issues/157) — the tracker
  issue for this plan.
- [PERFORMANCE.md](../../PERFORMANCE.md) — N-class designs, P8, `make speed`.
- [NATIVE.md](../../NATIVE.md) — the native backend's architecture.
- [LIBRARY_CHECKLIST.md](../../LIBRARY_CHECKLIST.md) — P5's home.
- `formal/draw.md` D-draw-2 (consumer side, loft-libs-graphics) — the
  deviation this closes.

