<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 189 — Stored references without refusals: a record lives as long as its holder's store

Tracker: [@PLN189](https://github.com/loft-lang/plans/issues/189).

## Status

**Needs approval — design ready, nothing built.**  `(H-RefHome)` (formal/heap.md) moves a named
local's store out to a holder of an enclosing block of the same function.  It refuses nine
shapes, the ones no variable of the frame can own.  Their guard is
`tests/scripts/a-stored-reference-cannot-outlive-the-record-it-names.loft`.  This plan admits all
nine by lowering them to operators both backends already have, chosen by the analysis that
refuses them today.  Approval is needed because the rule changes: from *"does not outlive the
home"* to *"lives as long as the store of its holder"*.  A second decision is needed on phase D:
whether to reclaim records that are left without an owner.

## Goal

Every holder of a `reference<T>` reads the record it was given for as long as the holder lives.
No program is refused for where it stores a reference, and nothing is added on the read path.

## Effort + design

- **Effort:** M overall: C small, A and B medium each, D small to medium, E small.
- **Design:** ✓ for A–C and E; ~ for D, whose semantics are the owner's call.
- **Last touched:** 2026-10-11.

## The shapes, grouped by what is missing

| Group | Refused shape (guard function) | What is missing |
|---|---|---|
| **A** no owner | `literal_built_in_a_loop`, `literal_assigned_in_a_loop`, `call_result_in_a_loop`; also a literal or call result stored into a parameter or a returned holder | the record exists only for the reference; its temporary is refilled each pass or dies with the frame |
| **B** owner dies with the frame | `into_parameter`, `returned_holder`, `copied_into_a_parameter`, `copied_into_a_returned_record`, `assigned_to_a_returned_record` | a named local is referenced from a holder the caller owns |
| **C** refilled own record | `the_record_of_a_loop_pass` (`p = P{…}; h.r = p` in a loop) | the next pass mints the local's store again at the position the reference names (the store root, record 1) |

## Design

All of it runs where the refusal runs now.  `scopes::stored_reference_homes`
(`src/scopes/stored_refs.rs`) judges each outlived hold in `Walk::carry`.  Today `Walk::hoist`
admits group 0 (a named local held from an enclosing block), and every other case is refused.  A
resolution replaces the refusal for each group.  The parser applies it in
`Parser::check_reshape_under_reference` before `scopes::check` runs, the way it applies
`insert_hoists` today.

### C — the refilled local's own record: place it in the holder's store

The local `p` is moved out by the existing hoist.  Its mint then changes:
- **Today:** `OpDatabase(p, P)` mints the store again at record 1.
- **Now:** release the previous pass's record, then place a fresh one in the holder's store:
  `if !OpRefIsNull(p) { OpFreeRecordIn(p, P) }; p = OpPlaceRecord(holder, P)`.

The release goes through `Store::delete`, and the record was referenced, so it leaves a
tombstone.  A holder kept from an earlier pass therefore reads null, as for a refilled vector
(`(H-Removed)`).  `p` becomes a borrow of the holder (its type's deps name the holder's root), so
the scope pass does not free it.  The last pass's record stays in the holder's store, which the
holder names anyway.

### A — a literal or a call result: build it in the holder's store

- **A literal for the field:** `h.r = P{…}`, or `H { r: P{…} }` built into a holder.  Its
  temporary's mint becomes `OpPlaceRecord(holder, P)`, and the field writes follow unchanged.
- **A call result:** the call's `__ref_N` return buffer is minted the same way.  This is
  `(R-Place)`'s claim with a new kind of destination: the record a pointer field names, rather
  than a field or element stored inline.  The store into the field stays `OpSetDbRef`, not a
  relocation.  The callee rules of `(R-Place)` hold unchanged: a callee that may hand back a store
  it did not mint declines, and stays refused.
- **The temporary becomes a borrow** of the holder's root (deps), so nothing frees it at the end
  of the pass.

Each pass's record is then a record of its own in the holder's store: `10 11 12`, not the last
pass's value three times.

### B — a named local escaping the frame: copy the named record out at the local's end

When the local's store is freed, each holder that outlives it and is known to hold one of its
records gets:

```
old = OpGetDbRef(holder, fld)
if !OpDistinctStore(old, home) {          // the holder still names into the dying store
  c = OpPlaceRecord(holder, T); OpCopyRecord(old, c, T); OpSetDbRef(holder, fld, c)
}
```

`OpCopyRecord` is the deep copy: text and vectors come along into the holder's store.  Copying
at the local's end, rather than at the write, keeps aliasing exact for the rest of the function
(`h.r = items[1]; items[1].k = 9` reads 9 through `h.r`).  Nothing names the local after the
frame, so the copy cannot be observed.

Where it is emitted:
- **The free is the chokepoint.** The parser marks the home variable with the holder places to
  copy out.  The scope pass's free emission (`get_free_vars`) puts the sequence in front of that
  variable's `OpFreeRef`, on every path that frees it.
- **The holder's place must be a path valid at that point:** a parameter's field, or a field of
  the return value.  A holder copied into a collection element (`o.hs += [h]`) has no stable
  place there, so it is copied at that line instead, from the copy's destination element.
- **Two holders naming one record get one copy:** before copying, compare the old reference with
  the ones already copied (`OpEqRef`) and re-point to the same copy.
- **A copied record's own references:** a record whose type holds a reference into the same
  dying local would need a recursive copy.  It stays refused, by a type test on the record type's
  reference fields.

### D — records without an owner (optional; the owner decides)

A placed (A, C) or copied (B) record has no field that owns it.  When its reference is re-pointed,
or its holder is removed from a collection, it stays in the holder's store until that whole store
is freed.  For a short-lived holder that costs nothing.  A long-lived one leaks a record per
update, for example `fn update(state: State) { tmp = […]; state.cur = tmp[0] }` called every
frame.

The minimal fix is a per-store `placed` set (a `Claims` bitset, like `referenced`):
- `OpPlaceRecord` sets the bit when called for a field.
- `Stores::set_db_ref` (`OpSetDbRef`, both backends) releases the old target when it is placed
  and lives in the holder's store.  It was referenced, so it leaves a tombstone.
- Removing a holder record (`remove_claims`) releases the placed targets of its reference
  fields.  Only types with reference fields take that walk.

Cost: one bit test per reference write, nothing on a read, nothing for a program without these
shapes.  The semantic it brings: a record created for a reference ends when that reference
moves on, and a copy of the holder made before then reads null.  The alternative is no D: accept
the growth, with an advice-level diagnostic where a loop re-points a holder declared outside it.

### E — the rule and the docs

- **`(H-RefHome)` becomes:** *a record a stored reference names lives as long as the store of its
  holder.*  The home moves out (group 0), is built in the holder's store (A, C), or is copied
  there at its end (B).
- **The refusal guard shrinks** to what stays refused: a callee that answers a store it did not
  mint, and a copied record holding a reference into the same dying local.
- **Other pages:** C145's catalogue line (DESIGN_DECISIONS_OWNERSHIP.md), the `reference<T>` row
  in LOFT.md, and the `LOFT_NO_REF_HOME` entry in BOTH_BACKEND_SWITCHES.md.  The switch then
  withholds the resolutions as well as the refusal.

## Composition matrix — Stage A

The cells below are probes before any phase, `--interpret` first, then both backends.  Every cell
builds a three-element vector after the holder's write, so a freed store reused would read
777–779.  Each asserts the value, the length of every collection involved, and a clean
`LOFT_STRICT_STORES=1` run.

| Cell | Shape | Expected |
|---|---|---|
| A1 | `for i in 0..3 { b.hs += [H { r: P { k: 10 + i } }] }` | `10 11 12` |
| A2 | `for i in 0..2 { b.hs[i].r = P { k: 20 + i } }` | `20 21` |
| A3 | `for i in 0..2 { b.hs[i].r = mk(30 + i) }` | `30 31` |
| A4 | `fn f(h: H) { h.r = P { k: 5 } }`, the caller reads `h.r.k` | `5` |
| A5 | `fn f() -> H { H { r: P { k: 6 } } }` | `6` |
| A6 | one holder outside a loop re-pointed to a literal 1000 times | the last value; holder store usage 1 live record with D, 1000 without |
| A7 | `h2 = h; h.r = P { k: 2 }`, then `h2.r` | the old record without D, null with D |
| A8 | a callee that may answer a store it did not mint, into a loop holder | still refused |
| B1 | `fn fill(h: H) { items = [P{k:1}, P{k:2}]; h.r = items[1] }` | `2` |
| B2 | B1 with `items[1].k = 9` after the write, read through `h.r` in and after the call | `9`, `9` |
| B3 | `fn f() -> H { items = [P{k:1}]; H { r: items[0] } }` | `1` |
| B4 | `fn f(o: Bag) { items = …; h = H { r: items[0] }; o.hs += [h] }` | the element's value |
| B5 | `-> Out { …; Out { h: h } }` | the record's value |
| B6 | `-> H { …; t = H { r: items[0] }; h = t; h }` | the record's value |
| B7 | two holders naming the same record, a write through one after return | visible through the other |
| B8 | the copied record holds a reference into the same local | still refused |
| B9 | the holder re-pointed to a parameter's record before the local ends | no copy; names the parameter's record |
| B10 | a nullable holder field null at the local's end | no copy, null |
| B11 | a vector, `sorted`, `hash` and `index` local as the home | each holder's value |
| C1 | `for i in 0..2 { p = P { k: i }; h.r = p }` | `1` |
| C2 | C1 with a second holder set in pass 0 | `-1` (null) |
| C3 | the holder declared inside the loop | that pass's value |

Axes the cells move: holder kind (local, element, parameter, returned value), home kind
(temporary, named struct local, vector, the keyed kinds), loop or not, a copy of the holder, the
reference's nullability.  Run `python3 scripts/matrix_axes.py file` on the graduated guard before
calling the matrix done.

## Sub-arcs

| Item | Source | Verify | Status |
|---|---|---|---|
| **C** — place a refilled local's record | § C | cells C1–C3, both backends; `LOFT_NO_REF_HOME=1` reads garbage | Open |
| **A** — place a literal or call result | § A | cells A1–A5 and A8, both backends; `make rewrite-census` drops nothing | Open |
| **B** — copy the named record out at the local's end | § B | cells B1–B11, both backends; the tombstone A/B (`LOFT_NO_TOMBSTONE=1`) moves nothing | Open |
| **D** — reclaim placed records (if approved) | § D | A6 store usage bounded, A7 per the decision; no read-path op added (`make ops-census`) | Needs decision |
| **E** — rule, guards, docs | § E | `rule_tags.py check`, `doc_lint.py`, the refusal guard reduced to A8 and B8 | Open |

## Phase ordering

1. **C first.** It is the smallest and reuses the hoist that is already built.  It proves the
   `OpPlaceRecord` mint plus borrow deps, which A needs too.
2. **A.** The literal half, then the call half (`place_result.rs` gains the pointer-field
   destination).
3. **B.** It needs the holder places carried from the analysis to the scope pass's free emission.
4. **D**, only after the decision, as its own commit, so its semantics can be judged alone.
5. **E** lands with each phase for the cells it admits.  The rule text changes with the first
   phase that admits a shape.

## Open design questions

1. **The contract:** approve *"lives as long as the store of its holder"* as the new
   `(H-RefHome)`.
2. **D's semantics:** reclaim on re-point (copies made before read null), or accept the growth with
   an advice.  Recommended: reclaim, because it bounds memory and never reads freed memory.
3. **Group A's direction:** should it stay refused anyway, since the message's rewrite (the
   collection beside the holder) is the clearer program?  If so, only C and B are built, and A
   keeps its refusal.

## See also

- [formal/heap.md](../formal/heap.md) — `(H-RefHome)`, `(H-Removed)`, `(H-RefSet)`; the code sites
  each phase changes.
- [formal/rewrites.md](../formal/rewrites.md) — `(R-Place)`, whose claim A reuses.
- [DESIGN_DECISIONS_OWNERSHIP.md](../DESIGN_DECISIONS_OWNERSHIP.md) — C145, no back references.
- Guards: `tests/scripts/a-stored-reference-cannot-outlive-the-record-it-names.loft` (the shapes
  this plan admits) and `tests/scripts/a-record-a-stored-reference-names-lives-as-long-as-its-holder.loft`
  (group 0, the hoist).
- loft#1973, the issue `(H-RefHome)` closed.
