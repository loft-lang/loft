<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 14 — Store-resident REPL session

> **Identity:** `@PLN14` — [loft-lang/plans#14](https://github.com/loft-lang/plans/issues/14)
> (`status:future` — the issue is the source of truth for lifecycle state). Slug
> `store-resident-repl-session`.

## Status — **DONE 2026-07-24**

Shipped: the REPL's binding environment is **store-resident**. A bound value is
materialized into a session store, observing reads that record instead of
replaying the accumulated body, and the values survive an on-disk round-trip
behind a fail-closed layout gate. Store-backed observing is **on by default**
(`LOFT_NO_STORE_OBSERVE` opts out).

**Reference content now lives in [REPL.md § How session state works](../../REPL.md)** —
what the session store buys, the `text` exception, re-bind lifetime, resume, and
the `eval_value` embedding API. This file is the closure record.

| arc | shipped as |
|---|---|
| **A** env + session store | `ReplSession.session_store` + `env`, one detached `Store` adopted per run |
| **B** materialize | `Stores::materialize` — `copy_block` + `copy_claims` |
| **C** scalars at rest | boxed 1-field record, raw bytes (never the display literal) |
| **D** frame-seed | `seed_paused_frame` + `State::frame_slot_addr`, no codegen change |
| **E** observe reads the env | default-on; `env_value` (own-format) + `env_display` (display) |
| **F** resume | `save_session_image` / `load_session_image` + `layout_algo_hash` gate |
| **G** lifetime | re-bind frees the orphaned record; `:reset` drops the store |
| **H** embedding API | `ReplSession::eval_value` |

Tests: `tests/pln14_matrix.rs` (38) — the value-type matrix, the differentials,
and the guards.

## What the build actually taught

Kept because each one contradicted the plan as written, and would otherwise be
rediscovered:

1. **The named sibling was the wrong one.** The plan built arc B on
   `Store::snapshot_copy`. A loft value is a MULTI-store graph (a nested struct
   puts each `Reference` field in its own store), so copying one store cannot
   materialize one. The right primitive is the `OpCopyRecord` walk —
   `copy_block` + `copy_claims` — which is type-driven, crosses stores, and
   allocates each sub-record in the DESTINATION, so nothing is shared and nothing
   needs rebasing. `rebase_walk_record` (the par path's `StoreRebase` walk) would
   have been a silent trap: it returns early for a `Vector` record type and never
   descends into collection elements.
2. **A materialized value is self-contained in ONE store.** Freeing every other
   user store leaves the copy readable. That is what made arc F small — the image
   is one store, not a graph — and what lets arc A carry the env across throwaway
   `State`s by adopting a single store at whatever slot is free.
3. **Two renderings, both required.** Observing prints loft's *display* form
   while `value_of` returns the *own-format literal*, and they genuinely differ
   (`hi` vs `"hi"`, `{x:7,y:9}` vs `P{x:7,y:9}`, `3` vs `3.0`). Serving only one
   from the store would have silently changed what every session prints.
4. **Slot coalescing is real.** Two bindings can resolve to the SAME frame slot,
   because the compiler coalesces locals whose live ranges do not overlap. The
   frame-seed seeds each slot once and omits the skipped name from its report,
   so a collision is visible instead of quietly wrong.

## The instruments, and the two that lied

The differential caught every regression it was built for, and twice the fault
was in the *reading*, not the code. Two instruments were **vacuous** and had to
be rebuilt — both found by asking "can this fail?", not by it failing:

- The first value-type matrix green-lit two struct-enum cells that never built a
  value (tuple-style `Shape.Circle(5)` is not loft syntax), so it compared
  identical garbage. `assert_readable` now gates every cell, with
  `calibration_guard_catches_an_unreadable_root` as its positive control.
- The first arc-G growth guard measured the store's ARENA SIZE, which is
  pre-allocated and stays flat whether or not orphans are released — it passed
  with the free deliberately disabled. It now counts live records; with the free
  disabled it reports 82 records where one bind leaves 2.

`canvas.toDataURL()`-style blind instruments are the same class: validate the
instrument before trusting a reading.

## Known residuals (not blockers)

- **`text` observes still replay** — stored as a 1-element `vector<text>` (the
  @P293 work-around), and the display renderer quotes vector text elements.
  Output is correct; only the speed win is missing. Closing it needs either raw
  text on that path or a vector-element accessor.
- **The resume image is not wired to auto-resume.** `~/.loft_session` text-replay
  still owns that path; choosing precedence between them is a product decision.
- **loft#618** — `value_of(<name>)` crashes for a vector binding on `main`
  (fn-return copy of a borrowed local, the @P293 family). Pre-existing, filed
  not fixed; the store-backed read sidesteps it because it executes nothing.

## See also

- [REPL.md](../../REPL.md) — where the reference content lives now.
- [CONVERGENCE.md](CONVERGENCE.md) — the model this plan built.
- [DESIGN_DECISIONS.md § @C72](../../DESIGN_DECISIONS.md) — resume restores stored
  values but deliberately **not** RNG generator state.
- **Tracker:** [loft-lang/plans#14](https://github.com/loft-lang/plans/issues/14).
