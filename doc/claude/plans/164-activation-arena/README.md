<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 164 — Activation arenas, adopt-at-bind and build-in-place

Tracker: [@PLN164](https://github.com/loft-lang/plans/issues/164) · `status:future` ·
`subject:store-lifetime`.

## Status (REQUIRED)

Open — the evaluation is done (2026-09-15, below), the three tiers are named with the
invariant each rests on, and no phase is cut.  **Not ready:** the owner reviews
[§ Edge cases](#edge-cases-to-inspect-before-a-phase-is-cut) first — every row there is a
case that must be admitted, declined or given a rule before the first phase, and a plan
that starts without that list is the plan-51 shape again (a mechanism validated only where
its derivations happened to coincide).  Routes here per the docs-vs-plans rule: the
mechanisms belong to `LIFETIME.md` / `OWNERSHIP_MODEL.md` / `formal/ownership.md` once
shipped; this directory is the design and the matrix until then.

## Goal (REQUIRED)

**The principle this plan serves (owner, 2026-09-15; GOALS.md § Goal F):** the
programmer is not assumed to know the store model inside, so the most NATURAL form of a
program is its CANONICAL form and therefore the one the compiler optimises — not an idiom
the author learns from a survival guide.  **And the scoping (owner, same day):** this is
not "optimise everything" — each spelling is first asked *is this natural to write?*, the
mechanism goes to the spellings that pass, and a contrived spelling keeps today's correct
copy and is deferred with its row kept.  Most cells of the matrix below may be deferred
at any moment; the natural ones are where the gain sits.  The compiler removes the per-call temporaries a record-returning style mints — a store
per hidden buffer per activation, a deep copy at a first bind, at a last-use field
assignment and at an element overwrite, a copy for a read-only `?`-discharge — without a
line of the consumer's code changing, and the drawing library's `parse` row goes from
7.6× the Rust reference toward 3× as the measurement of it.

## Effort + design

- **Effort:** M (tiers 1–2) · MH (tier 3)
- **Design:** ~ — the invariants are named; the store-identity question (§ Edge cases E1)
  is open and decides tier 1's shape.
- **Last touched:** 2026-09-15

## The evaluation — what one parsed line costs, and why

The owner's premise, stated 2026-09-15: *knowing this fact of loft I would look at the
parse implementation to make fewer objects; but what I really want is a compiler that
determines this problem itself and makes efficient code for it.*  So the library
(`loft-libs-graphics`, `drawing/src/drawing.loft`, `parse_poly` and its callees) is the
FIXED input; it is written in the natural style — build a record, return it, put it in a
field — and the question is what the compiler mints around that style.

A census of the emitted Rust (`--native-emit` of the bench's `parse_only.loft`, counting
store mints, record mints, deep copies and frees per function and attributing each to its
loft line) says, for ONE `Poly` line:

| temporary | mechanism that mints it | needed by the program? |
|---|---|---:|
| 4 stores at `parse_poly` entry (`__ref_3..6`) | every record-returning callee gets a caller-minted return buffer, one STORE each, freed at exit | no — dead at exit |
| a store for `pp_raw` + a deep copy of the `PointList` (4 vectors) | the first bind of a call result copies: `OpBindOrCopy` adopts only a store the local already holds, and a fresh local holds none (`O-Buffer`) | no — the buffer could be adopted |
| a store per `?`-discharged record element (`ap_e = sc.elems[idx]?` in `acc_pts`) + a copy of the `Elem`, its `ename` text included | `?` on a record element materialises a record; `Elem` owns text, so § V-ad's all-scalar buffer does not apply | no — only read, field by field |
| a store for `Elem {…}` + a copy into `sc.elems[idx]` | an indexed overwrite from a literal builds in a temp and copies | no — the slot exists |
| a deep copy of `Paint` (with its `spec` vector) into the `Op` | `paint: pp_paint` copies at `pp_paint`'s last use | no — a move would do |
| the points vector copied into `Op.pts` and again into `Mark.pts` | `smooth_pts` builds in its own buffer; each destination copies | one copy at most |
| a default prefill of every minted record | a partial literal defaults the rest field by field (`set_default_value_nullable`) | no — a per-type image is one block write |

Per line that is a dozen store mint/free pairs, four deep copies and three copies of the
points, on a scene of thirteen lines.  The `Op` itself is ALREADY built in place in the
scene's store (`@FR-R-Mint`/`@FR-R-PushRec`), and the `Mark` already lands in a buffer
`parse_scene_at` mints once for the whole scene and hands down — so two of the objects
already live in a longer-lived container; everything below `parse_poly` does not.

The profile agrees (program samples only, 20 000 calls, this box):

| class | share | routines |
|---|---:|---|
| record and vector lifecycle | ≈ 57 % | `copy_claims` 8, `set_default_value_nullable` 7, `begin_write_inner` 4, `remove_claims_mode` 3.5, `store_mut` 4.7, `free_named` 3, `claim` 2.7, `Store::init` 1.8, `store_budget` 1.8, `close_file_handle` 1.2, `owned_walk` 1.4 … |
| the library's byte scan | ≈ 19 % | `matches_at`, `find_option` (rescans the line per key) — the library's own |
| the parse logic | ≈ 8 % | `parse_scene_at`, `acc_pts`, `fronds`, the trig |

## The three tiers — the invariant each rests on

Each tier is a situation the compiler PROVES (C120: no rewrite may change a value after a
fault; only situations we know), switchable and falsifiable in the @PLN157 style, and
lands on BOTH backends where it changes the IR (the interpreter is the values oracle, and
`O-NoDiverge` says the two translate the same `deps` facts).

**Tier 1 — the activation arena.**  *Invariant:* a hidden return buffer (`__ref_N`, a
`__ref_p2_N` discharge buffer, a literal's temp) never outlives the activation that minted
it (`O-Buffer`: the buffer is the caller's store, freed at frame exit; a result that must
survive is adopted, copied or moved OUT of it).  So every buffer of an activation may be a
RECORD in one store instead of a store each, allocated past a mark taken at entry and
released to that mark at every exit.  The store is the CALLER's, threaded down like the
buffer itself is today (`R-Callee`: "a return buffer may be a record the caller offered"),
so a loop such as `parse_scene_at`'s line loop resets one arena per iteration — the
compiler can see nothing minted since the loop head survives except what was appended to
`sc`.  Removes the store mint/free pair per temporary: `Store::init`, `database_named`,
`store_budget`, `close_file_handle`, `owned_walk`, `free_named` — about a fifth of the row.
*What it must keep:* every rule keyed on STORE IDENTITY (§ Edge cases E1).

**Tier 2 — adopt at first bind, move at last use.**  *Invariant:* `O-Move` — a returned
heap value's ownership transfers to the caller's binding.  Today `OpBindOrCopy` honours it
only when the local already holds a store; the first bind copies.  Adopting the buffer at
the first bind makes iteration one what iteration two already is.  And `B-Copy`'s copy at
`paint: pp_paint` is a copy of a value nobody reads again: when the ownership oracle says
the local OWNS its store and liveness says it is dead on every path after the assignment,
the field takes the record by MOVE (bytes relocate, heap handles never change store —
`@FR-R-MoveAppend`'s relocation, applied to one record).  Removes the deep-copy class, about
a sixth of the row.

**Tier 3 — build where it will live.**  *Invariant:* a value whose ONLY destination is
known at the site it is built may be built THERE (`@FR-R-ElemFirst` states it for a vector
consumed by one append; this generalises the destination).  Three shapes: an indexed
overwrite from a literal writes the slot's fields (the old element's owned heap released
first, a partial literal's omitted fields set to their defaults); a call whose result is
stored exactly once gets the destination slot as its return buffer, so `smooth_pts` fills
`Op.pts` directly; a `?`-discharged record element that is only read is a VIEW (`B-View`),
not a copy.  The per-type prefill image belongs here.  Removes the remaining copies and
most of the prefill.  The one copy that stays: a vector stored in TWO destinations
(`Op.pts` and `Mark.pts`) — a record field owns its data, and that is the right answer.

## Composition matrix — Stage A (REQUIRED)

The axes the tiers touch, each with the domain the language offers; every phase's cells
are drawn from these, hand-computed, run on both backends under `LOFT_STRICT_STORES=1`,
`LOFT_POISON=1`, `LOFT_POISON_CLAIM=1` and `LOFT_NATIVE_LEAK_CHECK=1`:

| axis | domain |
|---|---|
| callee kind | user fn · fn-ref (`O-Opaque`: empty deps) · `#rust` native · generic instance · twin (`__inv`) |
| result shape | all-scalar (a § V-aa value record) · record with vectors · record with text · nullable record · struct-enum · tuple |
| binding | first bind · rebind in a loop · a value-branch join (`O-Complete`) · a `?? default` discharge · `return f(…)` (§ V-an's phantom) |
| activation | plain · recursive · generator (`yield` inside) · `par` arm · a closure body |
| exit | fall-through · early `return` · `break`/`continue` · a fault (`LOFT_DEV_SOFT_HALT`) |
| destination | fresh local · existing local · record field · vector element (append / overwrite) · the function's own result · two destinations |
| aliasing | none · the source IS the destination's old value (`ename: ap_e.ename`) · a `&` view live across the write (`B-Disturb`) |

`python3 scripts/matrix_axes.py file <cells>` measures which values a cell corpus reaches;
`cross activation destination` is the pair the corpus must cross, since that is where the
arena and the placement meet.

## Edge cases to inspect before a phase is cut

Numbered for the review.  **Verdict** is the proposal; the owner confirms or moves it.
Every row is first judged on NATURALNESS: `natural` means a programmer writes it without
knowing the store model and the mechanism owes it the efficient code; `contrived` means
the spelling keeps today's copy and the row is deferred — a verdict that is always
available and never a defect.  The parse library is the reference for "natural": every
shape it uses (E7, E13, E15, E16, E17, E20) is natural by construction.

| # | naturalness |
|---|---|
| E1–E6, E18, E19 | mechanism-internal — not a spelling; they must hold whatever is admitted |
| E7, E13, E15, E16, E17, E20 | natural — the library writes them today |
| E8, E9, E10, E11, E14 | natural — a branch, a parameter, a partial literal are ordinary |
| E3 (generator), E4 (`par`), E12 (`OpDrop` types) | contrived for this plan — decline the mechanism there, keep the copy, defer |

| # | case | why it bites | proposed verdict | probe |
|---|---|---|---|---|
| E1 | **store identity is the free protocol's key.**  `OpFreeRefIfDistinct(v, __ref_N)` declines the local's free when `v.store_nr == __ref_N.store_nr` (`O-Buffer`), `OpDistinctStore` answers `store_nr !=`, and § V-af's witnesses are store identities | two buffers in one arena share a `store_nr`, so "same store" no longer means "same object": a free declines that should fire (leak) or a witness matches the wrong buffer | identity becomes `(store_nr, rec)`; `free_displaced` and the witness compare both; `emission_audit.py` R-State counts holders per record | count the sites (`grep -c store_nr !=` on an emission), then a hand-patched arena of `parse_poly`'s four buffers under `LOFT_STRICT_STORES` |
| E2 | recursion | a caller-threaded arena grows with depth; a mark/release at each activation exit keeps it a stack, but a result adopted from the arena by the CALLER's local (tier 2) sits above the callee's mark | release-to-mark only what the activation minted and did not hand up; a handed-up record is re-marked to the caller | a recursive record builder, depth 1 000, leak gate |
| E3 | a generator | the activation SUSPENDS; a local arena dies at the yield | the arena is a persistent field of the generator, or tier 1 declines a body with `yield` (as every hoist does) | the coroutine corpus under the switch |
| E4 | `par` arms | one arena, two threads | one arena per arm, marked and released by the arm | THREADING.md's corpus |
| E5 | records that own heap in an arena (`Elem.ename`) | release-to-mark is O(1) only for no-heap records; text handles must be released (`@FR-H-ClearRelease`) | the mark carries a heap-owner list; a walk over that list, never over the store | a cell with 1 000 text-owning temporaries, `LOFT_NATIVE_LEAK_CHECK` |
| E6 | an arena record's vector GROWS | growth relocates within the store; `@FR-R-Base` bases and `@FR-R-Header` headers into arena records | an arena mint counts as growth for the enclosing loop's `growth_free`, as a null-discharge buffer does today | `LOFT_HOIST_VERIFY=1` on the parse bench |
| E7 | adopt at first bind when the buffer is the CALLER's caller's (`ps_p = parse_poly(…, __ref_5)`) | the local and the buffer are one store; the steady state after iteration one already | admit — this IS the steady state; the pin shows iteration one equal to iteration two | `tests/scripts/141-…` (plan 51 cluster I) under the switch |
| E8 | adopt when the callee handed back a store it did not mint (`O-Opaque`: a fn-ref, a return that borrows a parameter) | adopting a borrowed store frees someone else's | decline unless deps say MINTED; a fn-ref call keeps the copy (its type cannot record) | plan 51 cluster V-c probe 53 |
| E9 | move at last use on ONE path only | `if c { s.p = pp } else { use(pp) }` — dead after on the then-path, live on the else | per-path liveness (`O-Complete`); the move only where dead on EVERY path after, else copy | a branch cell with the value read after on one arm |
| E10 | move from a `const` parameter, a view, an element | not owned; a move would steal | decline: the oracle's OWN verdict is the gate, `O-Proxy` is not enough here | cells over each source kind |
| E11 | the moved-from local at scope exit | its free must not run (the record is gone) | the move nulls the source (`@FR-R-MoveAppend` zeroes it); `LOFT_POISON` is the falsifier | poison run |
| E12 | a type with `OpDrop` (@PLN163 copy leases) | a move must not run the drop; a copy must take a lease | a moved record keeps its lease; interacts with @PLN163's rule — decide there | @PLN163's cells |
| E13 | **self-referential overwrite**: `sc.elems[idx] = Elem { ename: ap_e.ename, … }` where `ap_e` views the SAME slot | building in place overwrites a field the literal still reads | evaluate every field expression before the first write (a stage), or decline in-place when any field reads through the destination | the `acc_pts` shape as a cell, hand-computed |
| E14 | a partial literal overwrite | omitted fields must become defaults, not keep the old element's values | write the defaults (the prefill image) before the named fields | a cell with an omitted field that was non-default before |
| E15 | the destination as return buffer when the callee exits EARLY | `if len(pts) < 2 { return Mark{…} }` leaves the destination half-written | a callee that returns on more than one path writes the destination only at its single exit, or declines | `parse_poly`'s three returns |
| E16 | the destination as return buffer when the callee READS the destination (`v = f(v)`) | source and destination alias | decline when any argument reaches the destination's store | a cell `s.pts = grow(s.pts)` |
| E17 | a `?`-discharged read used as a view while the container is DISTURBED before the last read | `ap_e = sc.elems[idx]?` then `sc.elems += […]` then `ap_e.bx0` | `B-Disturb` — a view is admitted only when no disturbance stands between the bind and the last read | a cell with an append between |
| E18 | the interpreter | tier 1 and 3 change the IR (where buffers live, where a literal builds), so both backends must agree — and the interpreter's `OpDatabase` per buffer is the same IR | parse-time change, one IR; the emitter's hoists read the arena as a store | the whole `tests/scripts` corpus on both backends |
| E19 | `LOFT_STRICT_STORES` / the leak gate's accounting | a store that hosts many buffers is one store to the gate; a leaked RECORD inside it is invisible to a store-count gate | the gate counts records above the mark at exit, not stores | extend the gate first (a probe-first phase) |
| E20 | a buffer adopted by a local that is then RETURNED (`n = f(); …; n`) | § V-an's phantom: the result lands in the function's OWN buffer, which is the caller's arena, not this activation's | the phantom is above the mark by construction; pin it | `V-an-chain-cells` under the switch |

## Sub-arcs (REQUIRED)

| Item | Source | Verify | Status |
|---|---|---|---|
| **P0** — probe first: count the store-identity sites; hand-patch `parse_poly`'s four buffers into one arena store on the emitted Rust; price tier 1 rustc-first; extend the leak gate to records-above-mark (E19) | this README | the hand patch runs green under `LOFT_STRICT_STORES` + the leak gate, or names the E1 site that breaks — either answer cuts A1 | Open |
| **A1** — arena for one activation's own buffers (`__ref_N`, `__ref_p2_N`, literal temps), mark/release at every exit | § Tier 1 | `tests/scripts/164-arena-activation.loft` both backends; plan 51's ten graduated guards under the switch; `emission_audit.py` R-State per record | Blocked on P0 |
| **A2** — the caller-threaded arena, reset per loop iteration | § Tier 1 | parse row −20 %; E2/E3/E4 cells | Blocked on A1 |
| **B1** — adopt at first bind | § Tier 2 | `introspect` diff: iteration one's IR equals iteration two's; E7/E8 cells | Open |
| **B2** — move at last use into a field | § Tier 2 | E9–E12 cells under `LOFT_POISON`; the `paint: pp_paint` site emits no `OpCopyRecord` | Open |
| **C1** — element overwrite from a literal in place | § Tier 3 | E13/E14 cells; `acc_pts` emits no temp store | Open |
| **C2** — the destination as return buffer | § Tier 3 | E15/E16 cells; `smooth_pts` writes `Op.pts` | Blocked on A1 (the destination is an arena or scene record) |
| **C3** — read-only `?`-discharge as a view | § Tier 3 | E17 cells; `acc_pts` copies nothing | Open |
| **C4** — per-type prefill image | § Tier 3 | `set_default_value_nullable` leaves the parse profile; the `Op` literal's cells | Open |

Every phase: a switch (`LOFT_NO_<unit>=1`), a falsifier, cells in
`bytecode-comparisons/`, a guard in `tests/scripts/` with its `@falsified-at:` receipt,
the pins in `tests/<unit>.rs`, `scripts/test_subjects.sh` extended — the @PLN157 shape.

## Phase ordering

1. P0 — it prices tier 1 for the cost of a hand patch and answers E1 before anything is
   built on it.
2. B1 and B2 — small, independent of the arena, each removes a copy class today.
3. A1 then A2 — the largest class, the largest change; A2 is where the parse row moves.
4. C1, C3, C4 — each a local mechanism; C2 last, it needs A1's destinations.
5. Re-measure the 14-row bench after each tier (`compare.py`, 14/14 hashes).

## Open design questions

1. **E1 — what is an object's identity once buffers share a store?**  `(store_nr, rec)`
   is the proposal; it touches `free_displaced`, `OpDistinctStore`, the § V-af witness
   and the audit.  Or: keep one store per buffer and make the STORE cheap (a pooled
   `Store` with no file handle, no budget row) — a smaller change that keeps identity as
   it is and takes less of the fifth.  The owner picks; P0 measures both.
2. **Is the arena a parse-time IR fact (both backends) or an emission fact?**  The
   buffers are IR (`OpDatabase(__ref_N)`), so tier 1 is IR-level unless the interpreter
   keeps stores and only native pools them — which `O-NoDiverge` allows for a lifecycle
   detail with no value, but the leak gate must then read both.
3. **Where does a moved-from local's `deps` go?**  Tier 2's move needs the scopes pass to
   record "handed off", the same predicate the double-move lint counts.

## Cross-arc dependencies

- **@PLN157** — the parse row's bar is that plan's; this plan is its parse unit, split
  out because it is a memory-model change, not a rewrite of a loop.
- **plan 51 (hidden-buffer aliasing, finished)** — its five clusters are the shapes an
  arena must not reopen; its ten graduated guards run under every switch here.
- **@PLN163 (copy leases)** — E12: a moved `OpDrop` record and its lease.
- **loft#1336 (owner witness)** — a witness is a store identity; E1 changes it.

## See also

- `LIFETIME.md` (`OpBindOrCopy`, the adopt-or-copy delivery), `OWNERSHIP_MODEL.md`,
  `formal/ownership.md` (`O-Buffer`, `O-Move`, `O-Complete`), `formal/binding.md`
  (`B-Copy`, `B-View`, `B-Disturb`), `formal/rewrites.md` (`R-Callee`, `R-ElemFirst`,
  `R-MoveAppend`, `R-Mint`, `R-PushRec`), `formal/heap.md` (`H-ClearRelease`).
- @PLN157 `DESIGN.md` § V-an (the parse table this plan starts from) and § V-ao (the
  census).
- The tracker issue: [@PLN164](https://github.com/loft-lang/plans/issues/164).
