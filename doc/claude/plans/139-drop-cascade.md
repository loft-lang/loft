<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN139 — a droppable's owner drops it

**Status — SHIPPED 2026-08-12.** All seven stages landed; this file is the closure record.
**The contract lives in [INTERFACES.md § `OpDrop`](../INTERFACES.md)** — read that, not this,
for what a drop does. Guards: `tests/owns_droppable.rs`, `tests/scripts/139-drop-cascade.loft`
(the 14-cell matrix below, as a running test), `tests/scripts/849-move-copy-source-drop.loft`,
`tests/double_move.rs`.

**Canonical:** [loft-lang/plans#139](https://github.com/loft-lang/plans/issues/139) ·
**Bug:** [loft#849](https://github.com/loft-lang/loft/issues/849) ·
**Scope decision:** [DESIGN_DECISIONS.md § @C111](../DESIGN_DECISIONS.md)

## The rule

A droppable copied into a container is a **MOVE**: the container's copy becomes the owner, the
source no longer drops, and the container's death drops what it owns. Decided over the refusal
alternative because the sanctioned workaround (`disown` after constructing the container)
requires the droppable to BE a field — a declaration-site refusal rejects its own cure, and with
collections in scope it would have to cover `vector<T>` too.

Scope boundary decided with it and recorded as **@C111**: the cascade reaches a container's
DEATH, not an element's REMOVAL. `v.remove(i)` / `v[i] = x` do not drop the displaced element.

> A drop runs when the value's OWNER dies. Taking a value out of its owner does not.

## Where it started

Two halves of loft#849 were already fixed on `tuxedo-decisions` when the plan opened:

- `fc3fb2c3` — a consumed MOVE source no longer drops. `OpCopyRecord(src, dst, kt|0x8000)` frees
  the source store, and the lift temp kept naming it, so a vector literal of two droppables
  closed the second's resource twice and the first's never (`LOFT_STRICT_STORES`: USE AFTER FREE,
  4 violations → 0).
- The narrowing on the issue: the source's early drop is only VISIBLE when the container outlives
  the scope, which is why a same-scope test reads as working.

What remains is the cascade itself, plus the field-side half of the move.

## The architectural finding — why this is a plan and not a patch

**The drop is emitted where the layout is not available.** `scopes::check(data: &mut Data)` is
where free/drop sites are computed, and it has `Data` only. Field offsets live in the schema
(`Stores`, via `database.position(kt, name)`), which `Data` does not carry and `Attribute` does
not store. So `OpGetField` — the one node a field cascade needs — cannot be built there at all.

Three routes were evaluated against that:

1. **Pass `&Stores` into `scopes::check`.** Mechanical (~8 call sites) and it unblocks struct
   fields and enum-variant dispatch. It does NOT unblock vector elements: those need a loop
   (`_vector_N` alias + index var + break conditions), which means minting variables and a loop
   scope *after* the scan, where slot allocation and the loop-scope bookkeeping already ran.
2. **Synthesize a per-type cascade FUNCTION from generated loft source**, so fields, enum match
   and the element loop all fall out of normal parsing. Blocked: `parse_str` resets `Data` and
   re-parses the whole text, so there is no snippet-injection path — it would need a third full
   parse, against a two-pass contract that is already load-bearing (the H5 guard).
3. **Synthesize the cascade function programmatically**, the `synth_nullable_par_wrapper`
   precedent (build the def, the var table, and a `Value` body). Field access comes free via
   `get_val`'s offset coercion, and enum dispatch is an `If` on the discriminator. The element
   loop is hand-built IR either way — this is the only route that reaches all three, and it is
   the recommended one.

So the vector half needs a designed mechanism, not an extra argument, and that is the piece that
makes this plan-sized rather than a session-tail change.

## Staging

Each stage lands against the matrix below, on BOTH backends, with the full gate green.

- **A — the query. SHIPPED.** `Data::owns_droppable(T)`: does T have a hook, or transitively
  contain a member type that does. Cycle-guarded, and deliberately a different fact from
  `drop_hook_nr` — a wrapper with no hook of its own around a type that has one answers `false`
  there and `true` here, which is exactly loft#849. Landed INERT: 78 pure additions to `data.rs`
  with zero callers, so it cannot change emitted output by construction (the stronger claim than
  a byte-identical diff, which only samples). Tests in `tests/owns_droppable.rs`: every `true`
  cell has a `false` twin differing in ONE axis, so a query that answered `true` for every record
  type fails half of them. Two cycle cases — a self-referential node, and a two-type cycle where
  the hook is reachable only through the back edge, asserted from BOTH ends (a guard that cached
  its `false` for a revisited def would answer differently depending on which end asked).
- **B — struct fields. SHIPPED.** `Parser::synth_drop_cascades` gives every type that owns a
  droppable FIELD a synthesized `t_<LEN><Type>_OpDropAll`: own hook first, then fields in
  reverse declaration order, each calling the field type's own cascade so nesting needs no
  special case. Declared in two phases (all names, then all bodies) so a nested chain does not
  depend on definition order. Runs at the five pass-2 tails beside `check_reshape_under_reference`
  — where every type and every hook are known — and is idempotent. `Data::drop_cascade_nr` is
  what a drop site calls: the cascade when one exists, the bare hook otherwise, so a program
  with no containers is unchanged.
- **C — the field-side move. SHIPPED.** A copy that hands its source off no longer leaves the
  source dropping: the `0x8000` move into an element (already in `fc3fb2c3`) and now a copy whose
  DESTINATION is a container field. **Scoped to containers that actually have a cascade**
  (`Data::has_drop_cascade`) — a copy into an enum payload or a collection element is not yet
  cascaded, so it is not treated as a transfer and its source keeps dropping. That scoping is
  what keeps the staging honest; without it stages D/E's shapes turn today's early release into
  a silent leak. The source also peels through a BLOCK tail, since an `Object` construction
  reaches the copy as the block that builds it (`Nest { s: S { … } }` double-released without it).
  Cells c1/c2/c4/c5/c6/c9 now match; guard: `tests/scripts/139-drop-cascade-fields.loft`.
- **D — enum payloads. SHIPPED.** The value is a record of the VARIANT with the discriminator
  at its head, so the enum's cascade reads it once and dispatches to the variant present; each
  variant gets its own field cascade (a variant is a record whose attributes are its payload).
  A variant with nothing to release gets no arm, so a unit-only enum synthesizes nothing. Two
  things it needed beyond the dispatch: the drop SITE widened to `Type::Enum(_, true, _)` — a
  struct-enum binding is a heap record exactly as a `Reference` one is, and reading only
  `Reference` is why the cascade was synthesized and then never called — and the CONSTRUCTION
  work-ref marked as having handed its record to the binding, since a struct-enum literal
  always takes the work-ref path (declared type is the enum, constructed type the variant, so
  it cannot be built in place) and `w: W = WH { h: c }` otherwise cascaded twice. Cell c3.
  **This is @PLN138's shape.**
- **E — collection elements. SHIPPED.** A per-element loop in the container's cascade,
  check-then-read (no user body can shrink the vector — a drop receives only `self`), length
  re-read each iteration, element var `skip_free` because it is a VIEW into the container's own
  storage. A LOCAL vector needed no special case: it is backed by a wrapper record whose field
  holds the elements, so the field cascade reaches it. Keyed collections (`hash`/`sorted`/…) are
  deliberately excluded — they share records with the collections they are indexed from, so
  releasing through one would release somebody else's element. Cells c7/c8.
  It also forced the last ownership hole closed: a copy into an element from a NAMED local sets
  no `0x8000` bit (the source stays live), so once elements release, the local releasing too is
  one resource released twice. An element append is now a transfer on the same terms as a field.
- **F — the contract. SHIPPED.** INTERFACES.md § `OpDrop` rewritten to the owner rule, with a
  worked container example and a "three things a drop does NOT do" section; LOFT.md's one-line
  summary follows it; the CAVEATS entry is cut down to the residual surprises and points at the
  contract. Every claim in the new prose was RUN before it was written — the documented example
  and all three non-guarantees (removal/overwrite leaks, keyed collections do not release, two
  containers release twice) are verified on both backends, so the doc states measured behaviour
  rather than intended behaviour.

- **G — the hazard the cascade created. SHIPPED.** The cascade turned "a droppable moved into
  TWO containers" from a leak into a DOUBLE CLOSE, so it ships with the diagnostic that catches
  it: `warning[double-move]`, `LOFT_NO_DOUBLE_MOVE` to opt out. `use_analysis::warn_double_move`
  counts hand-offs per source variable using the SAME predicate the drop suppression uses
  (`scopes::copy_hands_off` / `appends_to_element`), so the lint and the mechanism cannot drift.
  `warning` not `advice`, per the tier rule — ignoring it produces a wrong result.
  Because a warning gates a library's CI it is deliberately an UNDER-approximation, firing only
  where both hand-offs certainly run: opposite `if` arms release once however the branch goes,
  a reassignment between them makes two distinct resources (and a reassignment on only ONE
  path retires the pending hand-off), and a terminator between them means the second never
  runs. What it cannot see is the iteration count — one hand-off in a loop body that runs twice
  — and a hand-off on only one branch, which LEAKS rather than double-releasing; both need a
  CFG loft does not build, and both are false NEGATIVES, the safe direction here. Verified
  silent across the whole corpus (1756 `.loft` files, 0 hits) before defaulting on, the sweep
  @PLN107 established. Guard: `tests/double_move.rs`, 15 cells, each asserting the verdict AND
  the runtime release count — a verdict-only test cannot tell a correct silence from a missed
  defect, and the two cells that release twice without warning are pinned as the known boundary.

**The plan is complete.** All seven stages shipped, the 14-cell cascade matrix matches
hand-computed expectations on both backends, `LOFT_STRICT_STORES` clean, full suite green.
Guards: `tests/owns_droppable.rs` (stage A), `tests/scripts/139-drop-cascade.loft` (B–E, 18
cells), `tests/scripts/849-move-copy-source-drop.loft` (the collection half of loft#849),
`tests/double_move.rs` (stage G).

## Closed, found while building

- **The two "concurrency" flakes were one bug, and not concurrency.** Filed here as counting
  assertions that moved under full-suite load; the read COUNT turned out to move because the
  STORE did. A hash's bucket seed is drawn per store and decides the bucket order, so records
  land at different offsets on every run and adjacent ones coalesce into one read: the identical
  program cost 330..594 four-byte reads over 40 fresh writes (median 378). The `< 500` bound sat
  inside that spread, so it failed on roughly the top decile — the suite just runs it often
  enough to hit the tail. `LOFT_HASH_SEED` is loft#710's control for exactly this, and
  `paged_browser.rs` already set it — in HEX, which `parse::<u64>()` rejects, so the pin fell
  through to random in silence and that test kept flaking for the reason its comment said it had
  fixed. loft#856: hex accepted, an unreadable value now says so, and the reproducibility guard
  extended to assert that the hex and decimal spellings of one value give the same BYTES (it
  only ever covered decimal — the working member hid the omission).

## The matrix

Hand-computed from the rule, captured against today's build so every cell has a before. Failing
today: c4 (drops before the container is alive), c6 (inner before outer), c7 (no drop at all),
c9 (the reported early close). Passing cells are anchors — c1/c2/c3/c5/c8/c13 pass today only
because the SOURCE happens to die at the same scope end, so they must keep passing for the NEW
reason.

| cell | shape | expected |
|---|---|---|
| c1 | struct field, source local | one drop, after `(alive)` |
| c2 | struct field, inline temp | one drop, after `(alive)` |
| c3 | enum payload | one drop, after `(alive)` |
| c4 | nested struct-in-struct | one drop, after `(alive)` |
| c5 | two droppable fields | reverse-declaration: 6 then 5 |
| c6 | container with its OWN hook | outer:7 then 70 |
| c7 | `vector<H>` two elements | 8 then 9 |
| c8 | `vector<S>` of containers | 10 |
| c9 | container returned, dies in caller | one drop, after `(alive)` |
| c10 | plain local, no container — CONTROL | unchanged |
| c11 | no droppable anywhere — CONTROL | nothing |
| c12 | enum unit variant, no payload — CONTROL | nothing |
| c13 | container in a loop | per iteration |
| c14 | two droppables, never containered — CONTROL | 51 then 50 |

All fourteen match on BOTH backends. The probe is committed as
`tests/scripts/139-drop-cascade.loft`; it lived in this file only while four cells still failed.

## The stage-G matrix

The lint's own cells, hand-computed before a line of it was written. Each asserts the verdict
AND the runtime release count, because a verdict alone cannot tell a correct silence from a
missed defect. Committed as `tests/double_move.rs`.

| cell | shape | expected |
|---|---|---|
| m1 | two fields from one local | WARN — released twice |
| m6 | `vector<H> = [c, c]` | WARN — released twice |
| m7 | field then element, one source | WARN — released twice |
| m10 | two hand-offs inside ONE `if` arm | WARN — released twice |
| m2 | one hand-off — CONTROL | silent, released once |
| m3 | opposite `if` arms | silent, released once |
| m4 | reassigned between hand-offs | silent, two distinct values |
| m5 | two distinct sources — CONTROL | silent, two distinct values |
| m9 | no droppable anywhere — CONTROL | silent, nothing released |
| m11 | two inline temps — CONTROL | silent, two distinct values |
| m12 | nested container | silent, released once |
| m14 | conditional reassignment retires the pair | silent, two distinct values |
| m8 | one hand-off in a LOOP body | silent, released TWICE — blind spot |
| m13 | second hand-off inside an `if` | silent, released TWICE — blind spot |

The two blind-spot cells are the boundary, not an oversight: both need a control-flow graph
loft does not build, and both fail in the direction a gating tier must fail. Sweep before
defaulting on: 1756 corpus `.loft` files, 0 hits.
