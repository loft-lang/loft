<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Remaining open work — the release gate

Consolidated, prioritised view of what is left, surfaced through the @PLN90 copy/borrow
work and its neighbours. **P0 = genuine correctness/soundness issues that must close before
a wide release. P1 = important optimisations (real wins, but correct-as-is) — lower
priority, done after the P0 issues.** Each item points at its canonical home.

> **Concrete build steps (verified 2026-07-06, decomposed into landable increments W1–W9):
> [CLOSEOUT.md](CLOSEOUT.md).** This file is the prioritised *view*; CLOSEOUT is the *how*.

> **SHIPPED 2026-07-06 (PR #514, squash `46ecd3dc`): phase B — the last-use MOVE-elision, DEFAULT
> ON.** The store-transfer half of the north-star (build a dead-after owned source directly into its
> destination instead of copy-then-free) now runs for every proven-safe shape (Record; Construct
> field-append / fresh construction / `a.field=base`; flat + nested), `LOFT_NO_MOVE_ELIDE` opts out.
> That is the *elimination* direction. What remains below is the **borrow direction** (return an
> alias, don't copy OUT — A1b/A2), the **user-facing lint** (the plan's namesake — Phase 2), and
> **Phase 3** (explicit-copy syntax).
>
> **CLOSED 2026-07-07 (PR #516, all 9 items resolved): @PLN90 is DONE.** The P0 soundness blocker
> — the A1b temporary-subject borrow-return UAF — is fixed on both backends (default on), the
> user-facing copy lint is enforced, and the par-native fixes landed. @PLN90 the tracking issue is
> closed (`status:finished`). Any explicit-copy-syntax work beyond this is a future enhancement,
> not a release blocker. The P0/P1 detail below is kept as the record of that close-out.

## P0 — genuine issues (must close before release)

### 1. Heap-ownership soundness — loft's stated #1 weakness (wide-release blocker)
[STABILITY_ROADMAP.md § wide-release bar](../../STABILITY_ROADMAP.md) is explicit: the
return-bind-ownership class must be **closed, not merely quiet** (reopened 2026-06-21 after a
dogfood UAF). Three concrete open pieces:

- ~~**A1b — temporary-subject borrow UAF.**~~ **FIXED (2026-07-06, default ON, `LOFT_NO_A1B`
  opts out).** A `-> vector` fn returning a borrow of a temporary subject it constructs
  (`h() { g(Filled{..}) }`) dangled once the temp was freed at scope exit. Root: the
  `one_buffer_chain` collapsed the subject store, `g`'s buffer, and `__retbuf` onto ONE work-ref
  (subject `Bind{substitute}`'d into the buffer, buffer `Rename`'d to `__retbuf`). Fix: a
  coordinated verdict change in `classify_ret_promotion` — skip the subject ref (distinct local,
  freed after the copy), materialise the buffer ref into a separate `__retbuf` — three distinct
  stores. Matrix green both backends under `LOFT_POISON`; exposure sweep clean. Guard
  `tests/scripts/85-temp-subject-borrow-return-uaf.loft`. [borrow-return/DESIGN.md § A1b gate-5](borrow-return/DESIGN.md).
- ~~**#462 — residual native record leak.**~~ **CLOSED** (verified 2026-07-06). The GitHub
  issue is closed and the compact repros run clean on both backends — subsumed by the @PLN85
  `store/adopt-free` siblings (#306/#464/#494/#504), *not* by A1b (a different representation:
  record vs vector-buffer). No open issue remains; re-open only if a crawler-scale record leak
  resurfaces (no in-repo corpus tests it now). Off the P0 list.
- **Analysis completeness (`O-Complete`).** The `deps`/`ownership_of` analysis is not proven
  **total** — and "an incomplete fact is not a compile error, it is a miscompile or a leak"
  ([OWNERSHIP_MODEL.md § Internal and invisible](../../OWNERSHIP_MODEL.md)). The over-free
  class is being retired (local_source / elem_accumulate done, the match_return crashes P2–P4
  fixed) but soundness is not yet *closed*. This is the umbrella the two items above live
  under. **Effort: L (ongoing).**

### 2. ~~`par`-dispatch over `vector<fn-ref>`~~ — **FIXED (2026-07-06, @PLN90 W6)**
The interpreter hang was already gone; the remaining native E0308 (par-fnref delivery emitting a
bare `DbRef` where `(u32, DbRef)` is expected) is **fixed**: `tuple_arg_prep`
(`src/generation/ops/parallel.rs`) gained a `Type::Function` arm reading the `i32` fn-index at
offset 0 and pairing it with a NULL closure. Refinement found while fixing: **`par` compiles its
worker to native under *both* backends**, so this blocked the interpret par-path too, not native
only. Guard: `tests/scripts/507-par-vector-fnref.loft` (both backends). **Done.**

### 3. ~~`&` write-back from a call RHS~~ — **RESOLVED** (`@PLN87 #1`)
Gate-1 re-check: `o = mk()` (call RHS) **works on both backends** (`check()` writes back 9);
no parse rejection remains, and the lock-in test `pln87_amp_writeback_from_call_writes_back`
is **un-ignored** (passes). The ownership-transfer machinery already routes a call RHS
through a transferable owned temp. **Done — not a blocker.**

### 4. ~~Parser formatting-sensitivity~~ — DEMOTED (gate-1: not a correctness blocker)
The same source parses to **different IR tree shape by formatting**: single-line →
`else ;` (Null), multi-line → `else { block }` (a real empty block). **Gate-1 capture shows
both lower to IDENTICAL native (`else { DbRef::NULL }`) and identical runtime behaviour** —
post-A1/P3/P4 both the null-else and the empty-block paths emit the correct typed-null. So
this is a **latent fragility, not an active miscompile**: it is what made P2/P3/P4
layout-sensitive, but with those fixed there is no behaviour difference left to gate a
release. Normalising the empty-arm parse so the IR tree is identical is a **robustness**
improvement (moved to P1 below), not a P0 blocker. **Effort: S–M.**

## P1 — important optimisations (lesser priority — correct as-is, do after P0)

These eliminate copies that are *correct but wasteful*; the north-star is the compiler
automatically not copying ([COPY_DIAGNOSTICS.md § North-star](../../COPY_DIAGNOSTICS.md)).

- ~~**A2 — struct-field `b.rows` copy→alias.**~~ **RESOLVED — WON'T DO (2026-07-06).** Three
  matrix-guarded prototypes established that A2 is **invalid**: a struct-field return **must COPY**
  per the decided value-semantics contract (`tests/scripts/85-store-lifetime-field-read-copy.loft`,
  the #415 guard: *"binding or returning a heap vector FIELD must COPY (establish a new owner)"*;
  @C86: *"#415 is the SEMANTIC not a stopgap"*). Aliasing `f(b){ b.v }` violates value semantics (a
  later `b.v += …` would change the result). The copy is FORCED, not avoidable; the current
  behaviour is correct. W3 (drop the buffer) is moot. [CLOSEOUT.md § W2](CLOSEOUT.md).
- **The user-facing copy lint.** Indicate every **unbound** structure copy — the copy's
  source is a still-live pre-existing structure that was *duplicated* (**Avoidable**: warn,
  with the `&`/restructure hint — the worklist; **Forced**: informational). Stay silent only
  on **bound** results — scalars, moves (`src` consumed here), and literal sources
  (**Implicit**). **PREREQUISITE — the source-survival split:** today `construct_copy` /
  `record_copy` are blanket-classified `Implicit`, so a construction/slot-set that duplicates a
  *live* source is wrongly silenced (it should be Avoidable/Forced). Land
  [unbound-copy-lint.md](unbound-copy-lint.md). **Update (verified 2026-07-06):** the survival
  split IS built — `survival_class` (`src/use_analysis.rs:855`) keys Implicit/Avoidable/Forced on
  source survival, and the user-facing report `report_copies` (`--report-copies`, #510) ships,
  both gated on `report_copies_enabled()`. What remains is only the **enforced** channel: route
  Avoidable rows through the existing `Level::Warning` diagnostics path (`data.rs` `diags.add_at`)
  as a default lint, and resolve `VerdictRow.loc` to real spans. Steps: CLOSEOUT W5.
  [COPY_DIAGNOSTICS.md § bound vs unbound](../../COPY_DIAGNOSTICS.md). **Effort: S–M.**
- **Drain bucket 2 (grow the auto-elision set).** Extend the `Borrow`→`ElidePlan` engine to
  more avoidable copies (var-buffer conservative cases the analysis can't yet prove,
  construction where the source provably out-lives a non-escaping record). Each avoidable row
  in the worklist is a candidate. **Effort: L (incremental).**
- **Explicit copy-intent syntax** (phase 3) — the inverse of `&`: opt into an independent
  copy and silence the lint. **Effort: S (design) + M.**
- **Wasted empty-buffer alloc.** A borrowed return still receives a `__retbuf` it ignores (a
  tiny empty-vector alloc). Optimise away once A2/the ABI settles. **Effort: S.**
- **Normalise the empty-arm parse** (was P0.4). `_ => { [] }` should parse to one canonical
  IR tree regardless of formatting (single-line `else ;` vs multi-line `else { block }`).
  No behaviour difference today (both lower to `DbRef::NULL`), but the divergence is the
  latent fragility that made P2/P3/P4 layout-sensitive — robustness, not correctness.
  **Effort: S–M.**

## Sequencing note

P0.1 (heap-ownership soundness) is the release blocker and the umbrella; **A1b before A2**
(a borrow without the temp-subject guard is the very UAF A1b fixes). The lint (P1) wants the
analysis honest first but does not block release. The parser fix (P0.4) is independent and
cheap.
