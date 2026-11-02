<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN91 strand 1b — the reference improved: what the comparison found in the Rust interpreter

A child document of [1b-interpreter.md](1b-interpreter.md).  Status: **planned**, after B-1
([1b-s3.md](1b-s3.md) § Binds and frees) lands.

## Why (owner, 2026-10-30)

*"Should we fix the rust interpreter side for some enhancements that we found?  So we can create
a better/more efficient interpreter?"* — yes, as part of the plan.  And on the result: *"This is
exactly as planned.  The second implementation shows us what fell through the cracks with the
first one."*  That is the method: the reimplementation is a differential instrument on the
reference, and every difference it shows is either a port defect (fixed in the port) or a
crack in the reference (fixed here).  The strict comparison
([1b-compare.md](1b-compare.md)) makes every store event of the Rust interpreter visible beside
the loft one's, and porting B-1 found places where the Rust interpreter decides a store event
inconsistently or wastefully.  Each is an improvement to the reference, made THERE.

## The rule

**The formal rules first (owner, 2026-10-30: "this will only work if we also keep the formal
rules up to date").**  A difference the comparison shows is a crack in the reference only when
a rule says what the right answer is; without one, "fix the reference" would mean "make the
Rust side do what the loft side does".  So every finding, a port defect or a reference one,
goes through `doc/claude/formal/` and the store model's register before any code:

1. **The rule.**  Find the rule that decides the event (`scripts/rule_tags.py`, the chapter's
   text).  If it settles the case, the side that disagrees is the deviation.  If it cannot
   express the case, the RULE is extended first — a design decision, the owner's when it
   changes a contract — and only then the code.
2. **The register.**  An implementation that breaks a rule is an entry in the chapter's
   `## Deviations` (`D-<chapter>-N`, `OPEN` count updated), opened when a probe measures it on
   `main`.  A difference between the interpreter's store model and the C runtime's is a row of
   [STORE_MODEL.md](../../STORE_MODEL.md) § Divergences (@PLN206).  Where the rule is silent on
   that pair, STORE_MODEL's own rule decides the direction: *the C runtime's model is the
   standard, unless the interpreter's is measurably better.*
3. **The change**, on the losing side, as an ordinary fix: a `LOFT_NO_*` switch for the A/B, a
   guard falsified on the build before (`make falsify`), both Rust backends, `make perf-check`
   on the rows it touches; the C runtime made to agree where it decides the same event.
4. **The loft interpreter follows the Rust one**, never the other way round (goal constraint
   5): `selfhost/compare/compare.py` on the kernels, the sample and the census is `DIFFERENT`
   exactly where the step changed the reference, and its port is the step's last commit, the
   layers `EQUAL` again as the receipt.  The entry or row then moves to its history companion
   with the commit, the probe staying as its guard.

B-1 lands first, matching the reference as it is today, so each step below is measured against
a known-equal baseline.

## The steps, in order

- **E-1 — one wrapper position (offset 8 vs 12)** — **BUILT** (D-heap-52).  The null-init of
  a vector local (`gen_owned_vector_store`) rewrote its slot to `pos 12` while `OpDatabase` and
  `OpGetField(…, 0)` name 8, so a caller-minted return buffer was cleared through a field its
  callee never filled.  The cell answered right on `main` (the callee's `OpDatabase` resets the
  store after every such clear; a corpus sweep found no clear over a live vector without one), so
  the cost was the capacity carry and the `(1, 8)` root tests, not a value.  The null-init now
  leaves the slot at 8 and `(H-ClearRelease)` states one position; guard
  `tests/reset_capacity.rs` `a_buffer_is_cleared_where_its_callee_filled_it`.
- **E-2 — the store exchange for a record that holds a keyed collection.**  Rule:
  `(H-SwapIn)` keeps the copy for "the keyed collections", which `(H-SwapKeyed)`'s own
  reasoning contradicts (a keyed collection's pointers are record numbers inside its store);
  the rule is extended first — the owner's call, as it widens a contract — and the C, which
  already exchanges, is a STORE_MODEL row until it is.
  `tree_holds_no_stored_refs` answers no for any keyed part, so `@FR-H-SwapIn` copies a
  returned record holding a hash where it could exchange the stores; the keyed clause
  (`try_swap_keyed`) already asks the right question — the collection's ELEMENT tree holds no
  stored reference.  The C's `swap_safe` has exchanged such records all along, which is
  evidence, not proof: the guard is a record with a hash field rebound from a call, both
  backends, `LOFT_STRICT_STORES=1`.
- **E-3 — clearing an empty vector at a store's root** (STORE_MODEL **D19f**) — **BUILT**
  (f491592ba).  `(H-ClearRelease)`
  is a leak rule and indifferent, so STORE_MODEL's rule decides: the C skips the reset, so the
  Rust interpreter moves to the C unless its reset measures better.  `clear_vector_release` resets the
  store (an init and two claims) even when the vector holds nothing; the C's inline fast path
  skips it.  The Rust interpreter skips the reset when nothing is held (the measurement on a
  buffer-reusing bench row decides only if its reset proves better).  B-1 first made the C reset
  as the Rust side does; that went the wrong way under STORE_MODEL's rule and is undone, and the
  loft interpreter, on the C runtime, agrees once this step lands.
- **E-4 — the swap gates, decided once** (STORE_MODEL **D8**).  The protection half is built:
  `(H-SwapIn)`'s *"a free protection refuses only the side the exchange releases"* holds on
  both backends (`loft_swap_in` refuses `a->protect`, the released source, as
  `swap_stores_in_as` does).  The live-state gates agree between the backends
  (`loft_swap_blocked_hook` mirrors them) and not with the rule, which is the open half and
  the owner's call: `(H-SwapIn)` pins a constant and a lazy binding per STORE, where both
  backends refuse every exchange while ANY lazy source or error exists, and refuse while any
  scratch walk is live, which no rule states.  Either the rule states the gates (a store a live
  walk reads is pinned, as a lazy binding's is) and the code narrows to them, or the gates go.

Each step is one PR-sized unit; a step whose cell shows a defect on `main` is that defect's fix
and follows the bug-filing policy.
