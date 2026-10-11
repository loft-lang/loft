<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 189 — Stored references without refusals: a record lives as long as its holder's store

Tracker: [@PLN189](https://github.com/loft-lang/plans/issues/189).

## Status

**Built — steps 0–6, 8, 9 and 11; step 7 kept refused; step 10 not built (needs a decision).**
`(H-RefHome)` now reads *"a record a stored reference names lives as long as the store of its
holder"* (formal/heap.md).  Guards:
`tests/scripts/a-record-built-for-a-stored-reference-lives-in-its-holders-store.loft` (29 cells,
both backends) and the refusal guard
`tests/scripts/a-stored-reference-cannot-outlive-the-record-it-names.loft` (9 shapes).

Where the build departs from the steps below:

- **Step 3** admits a callee whose every exit is a literal minted behind the offered-record
  test (`place_result::callee_keeps_an_offered_record`); a callee that rebinds or hands its
  buffer on is refused with A8.
- **Step 6** copies out only through a parameter's fixed place: the parameter itself or a
  constant field of it (`Walk::fixed_place`).  An element of a parameter's collection has no
  fixed place at the free and stays refused.
- **Step 7 (B4) stays refused.**  The element `_elm` names may move when its collection grows
  before the home's free, and a copy right after the append would split the aliasing the
  language promises for the rest of the function: a write into the local after the append would
  not show through the element.
- **Step 8 (B7)** shares one copy only between holders in ONE store (two fields of one
  parameter).  Two parameters naming one record are refused: they may be two stores of the
  caller, and a copy each would split what one write showed through both; a single copy in one
  of them would dangle once that store is freed before the other.
- **Step 4** also admits a second holder of the same block (C2 reads `1 0`); holders of two
  blocks are refused.
- **Step 10 is not built.**  Freeing a placed record when its reference is re-pointed is unsound
  as specified: a variable can still name the record — `p` in `p = P {…}; h.r = p; h.r = q;
  p.k`, or a view the copy analyses elided — and a variable read does not check the removal
  mark, so it would read a freed record.  Without step 10 a holder re-pointed in a loop keeps
  every earlier record in its store until the store is freed (A6: 1000 records after 1000
  re-points).  Open question 2 below needs the owner's answer before it is built.
- Found on the way and fixed: a write into a keyed collection's element was never judged
  (loft#1978); the guard's `a_keyed_element_holder` cell keeps it.

## Goal

Every holder of a `reference<T>` reads the record it was given for as long as the holder lives.
No program is refused for where it stores a reference, and nothing is added on the read path.

## Effort + design

- **Effort:** M overall; every step is S.
- **Design:** ✓ for steps 0–9 and 11; ~ for step 10, whose semantics are the owner's call.
- **Last touched:** 2026-10-11 (built, see Status).

## The shapes, and what each lowers to today

| Group | Refused shape (guard function) | Today's IR, with the refusal withheld (`LOFT_NO_REF_HOME=1`) |
|---|---|---|
| **A** no owner | `literal_built_in_a_loop`, `literal_assigned_in_a_loop`; a literal into a parameter or returned holder | `OpSetDbRef(holder, f, {OpDatabase(__ref_p2_1, P); …fields…; __ref_p2_1})`: a function-level temporary minted again each pass |
| **A** no owner | `call_result_in_a_loop` | `__lift_1 = n_mk(…, __ref_1); OpSetDbRef(holder, f, __lift_1); OpFreeRef(__lift_1)` |
| **B** owner dies with the frame | `into_parameter`, `returned_holder`, `copied_into_a_parameter`, `copied_into_a_returned_record`, `assigned_to_a_returned_record` | the local's store (`__vdb_1`) freed at the exit by `OpFreeRef`, or by `OpFreeRecordIn` where `(R-Place)` placed it in the return buffer |
| **C** refilled own record | `the_record_of_a_loop_pass` | `p = null; OpDatabase(p, P); …; OpSetDbRef(h, f, p); OpFreeRef(p)` each pass |

## Design

Every decision is made where the refusal is made today.  `scopes::stored_reference_homes`
([stored_refs.rs:63](../../../src/scopes/stored_refs.rs#L63)) judges each outlived hold in
`Walk::carry` ([stored_refs.rs:520](../../../src/scopes/stored_refs.rs#L520)).  It admits a named
local held from an enclosing block through `Walk::hoist`
([stored_refs.rs:636](../../../src/scopes/stored_refs.rs#L636)), and refuses every other case.
`Parser::check_reshape_under_reference` applies what the walk answers before `scopes::check` runs
([control.rs:11463](../../../src/parser/control.rs#L11463)).

- **Placement (A, C).** The record is built in the holder's store:
  `OpPlaceRecord(host, tp)` ([01_code.loft:1760](../../../default/01_code.loft#L1760),
  `Stores::place_record_prefilled`, [database/mod.rs:2894](../../../src/database/mod.rs#L2894)).
  The variable that holds it becomes a borrow of the holder's root through its type's deps.  That
  is load-bearing: `OpFreeRef` frees a whole store by number (`Stores::free`,
  [allocation.rs:997](../../../src/database/allocation.rs#L997)).  A free emitted for a placed
  record would release the holder's store.
- **Copy out (B).** When the local's store is freed, each holder that outlives it and still names
  one of its records gets a copy in the holder's own store, and is re-pointed at the copy.  It is
  one new helper operator, `OpEvacuate` (step 5).  It composes code that exists: the store test of
  `OpDistinctStore` ([01_code.loft:1314](../../../default/01_code.loft#L1314)),
  `place_record_prefilled`, the deep copy of `OpCopyRecord`
  ([01_code.loft:1754](../../../default/01_code.loft#L1754)), and `Stores::set_db_ref`
  ([structures.rs:3259](../../../src/database/structures.rs#L3259)).  Copying at the end, not at
  the write, keeps aliasing exact for the rest of the function.
- **Owner-less records (step 10, optional).** A placed or copied record has no owning field.
  When its reference is re-pointed, or its holder removed, it stays in the holder's store until
  that store is freed.  A per-store `placed` bitset, tested on reference writes only, reclaims it.

## Steps

Every step lands as its own commit with the cells it admits, on both backends, with
`LOFT_STRICT_STORES=1` clean.  Each falsifies the guard against the commit before it and moves the
admitted cells from the refusal guard to the admitted guard.  A step that cannot admit a site
leaves it refused: a refusal is never wrong, only a wrong answer is.

| # | Step | Code it touches | Verify |
|---|---|---|---|
| 0 | **Probes.** Write the Stage-A cells below as `/tmp` probes and record today's answers: refused, and with `LOFT_NO_REF_HOME=1`. | none | every cell either refused or reading 777–779 or garbage |
| 1 | **Resolution, still refusing.** `Walk::hoist` becomes `Walk::resolve`, answering `Hoist`, `Place`, `Evacuate` or `Refuse`; `StoredRefs` ([stored_refs.rs:45](../../../src/scopes/stored_refs.rs#L45)) carries the place and evacuate lists.  The parser applies only `Hoist`, and refuses the others as today. | [stored_refs.rs:520](../../../src/scopes/stored_refs.rs#L520), [:636](../../../src/scopes/stored_refs.rs#L636) | no IR changes anywhere (`LOFT_LOG=static` diff over every file using `reference<`); both guards unchanged |
| 2 | **Place a literal (A1, A2, A4, A5).** For a site whose value is an object block minting `OpDatabase(t, tp)`: take a fresh copy of `t` (`Function::copy_variable`, [variables/mod.rs:2905](../../../src/variables/mod.rs#L2905)) typed with deps on the holder's root (`set_type`, [:5872](../../../src/variables/mod.rs#L5872)).  Inside that site's value only, replace the mint with `Set(t', OpPlaceRecord(root, tp))`.  The site is found by its ordinal among `OpSetDbRef`s in walk order, the way `insert_hoists` ([stored_refs.rs:101](../../../src/scopes/stored_refs.rs#L101)) finds blocks.  A fresh copy, because `__ref_p2_1` may serve another literal of the function. | [stored_refs.rs:427](../../../src/scopes/stored_refs.rs#L427) `write`, [control.rs:11463](../../../src/parser/control.rs#L11463) | A1 `10 11 12`, A2, A4, A5; `LOFT_POISON=1` clean; no `OpFreeRef` of `t'` in the dump |
| 3 | **Place a call result (A3).** The site's call takes a fresh copy of its `__ref_N` buffer, set to `OpPlaceRecord(root, tp)` in front of the statement, typed as in step 2.  Admitted only where `(R-Place)`'s callee clause admits the callee: its buffer written on every exit (`callee_writes_buffer_at_every_exit`, [place_result.rs:406](../../../src/place_result.rs#L406), made `pub(crate)`), and minted behind the offered-record test (`OpRefIsNull` / `OpConvBoolFromRef` in front of its `OpDatabase`, as `(R-Place)` emits for `__retbuf`).  Any other callee stays refused (A8): an unguarded `OpDatabase` on a placed record re-initialises the WHOLE holder store, since `Stores::clear` ([allocation.rs:2137](../../../src/database/allocation.rs#L2137)) runs for every type but a vector wrapper (`mint_at_place`, [:1213](../../../src/database/allocation.rs#L1213)). | [stored_refs.rs:303](../../../src/scopes/stored_refs.rs#L303) `home`, [place_result.rs:406](../../../src/place_result.rs#L406) | A3 `30 31`; A8 refused; strict clean |
| 4 | **Place a refilled local's own record (C1–C3).** `Walk::hoist` declines `refilled && root` today ([stored_refs.rs:636](../../../src/scopes/stored_refs.rs#L636)); resolve it to `Place` instead.  The local's in-loop `OpDatabase(p, tp)` becomes `Set(p, OpPlaceRecord(root, tp))`, and `p` takes deps on the holder's root, so its per-pass `OpFreeRef(p)` is not emitted.  The previous pass's record stays in the holder's store (step 10 reclaims it). | [stored_refs.rs:636](../../../src/scopes/stored_refs.rs#L636) | C1 `1`, C2 `0`, C3 per pass; strict clean |
| 5 | **The helper, no caller yet.** `fn OpEvacuate(holder: reference, fld: const u16, home: reference, tp: const u16)` in `default/01_code.loft` beside `OpPlaceRecord`, with `#rust"stores.evacuate(…)"`, the interpreter arm in `src/fill.rs`, and `Stores::evacuate` beside `set_db_ref` ([structures.rs:3259](../../../src/database/structures.rs#L3259)).  When the field names a record in `home`'s store: place a `tp` record in the holder's store, deep-copy into it, re-point the field.  Otherwise do nothing.  Run `make surface-gen` (a new operator renumbers the surface). | [01_code.loft:1760](../../../default/01_code.loft#L1760), [structures.rs:3259](../../../src/database/structures.rs#L3259) | a Rust unit test: same store copied and re-pointed, other store untouched, null untouched; `make ops-census` lists it unexercised |
| 6 | **Copy out at the local's frees (B1, B3, B5, B6, B9–B11).** For a named home held by a holder whose place is a path rooted at a parameter or the return buffer (`__retbuf`), the walk answers `Evacuate { owner, place, fld, tp }`.  The owner is marked (`mark_ref_home`, [variables/mod.rs:4058](../../../src/variables/mod.rs#L4058)), and `(R-Place)` declines to place a marked owner (`place_result::admits`, [place_result.rs:161](../../../src/place_result.rs#L161)).  That keeps the home's store its own, so the store test is exact.  A pass after the scope pass, before `compute_function_intervals` ([scopes.rs:845](../../../src/scopes.rs#L845)), puts `OpEvacuate(place, fld, owner, tp)` in front of every `OpFreeRef(owner)` and `OpFreeRecordIn(owner, _)`, whoever emitted it.  Still refused (B8): a record type holding a reference field whose target type the home holds. | [stored_refs.rs:520](../../../src/scopes/stored_refs.rs#L520), [scopes.rs:845](../../../src/scopes.rs#L845), [place_result.rs:161](../../../src/place_result.rs#L161) | B1 `2`, B2 `9 9`, B3, B5, B6, B9 no copy, B10 null, B11; B8 refused; `LOFT_NO_TOMBSTONE=1` A/B moves nothing |
| 7 | **Copy out at a copy into an element (B4).** A holder copied into a collection element (`o.hs += [h]`, `Walk::copy`, [stored_refs.rs:566](../../../src/scopes/stored_refs.rs#L566)) has no stable place at the local's end.  The parser puts `OpEvacuate(_elm, fld, owner, tp)` right after that copy, where `_elm` is in scope. | [stored_refs.rs:566](../../../src/scopes/stored_refs.rs#L566), [control.rs:11463](../../../src/parser/control.rs#L11463) | B4; the element names a record in `o`'s store |
| 8 | **One copy for two holders (B7).** `Stores::evacuate` remembers `(old, copy)` pairs for the store being freed, and answers the same copy for the same old record.  The memory is cleared when that store is freed (`free_named`). | [structures.rs:3259](../../../src/database/structures.rs#L3259), [allocation.rs:997](../../../src/database/allocation.rs#L997) | B7: a write through one holder after return shows through the other |
| 9 | **Rule and refusal guard.** `(H-RefHome)` becomes *"a record a stored reference names lives as long as the store of its holder"*.  The refusal guard keeps A8 and B8.  Update C145's catalogue line, the `reference<T>` row in LOFT.md, and the `LOFT_NO_REF_HOME` entry (it then withholds the resolutions too). | formal/heap.md, DESIGN_DECISIONS_OWNERSHIP.md, LOFT.md, BOTH_BACKEND_SWITCHES.md | `rule_tags.py check`, `doc_lint.py` |
| 10 | **Reclaim owner-less records (if approved).** (a) A `placed: Claims` set on `Store` beside `referenced` ([store.rs:352](../../../src/store.rs#L352)).  A new `OpPlaceFor(host, tp)` sets it, and steps 2–4 and `Stores::evacuate` switch to it.  (b) `Stores::set_db_ref` releases the old target when it is placed and lives in the holder's store, through `free_record_in` ([database/mod.rs:2864](../../../src/database/mod.rs#L2864)).  That goes through `Store::delete` ([store.rs:2101](../../../src/store.rs#L2101)), which leaves a tombstone.  (c) `remove_claims` ([allocation.rs:4270](../../../src/database/allocation.rs#L4270)) releases the placed targets of a removed holder's reference fields; only types with reference fields take the walk. | as listed | A6 holder store holds 1 live record after 1000 re-points; A7 null; `make ops-census` shows no op on the read path |
| 11 | **Graduate.** Guards falsified against the commit before step 2; `python3 scripts/matrix_axes.py file` on the admitted guard; reach survey over every file using `reference<` (today: the stdlib and loft's own tests only). | tests/scripts | both guards green on both backends |

## Composition matrix — Stage A

Every cell builds a three-element vector after the holder's write, so a freed store reused would
read 777–779.  Each asserts the value, the length of every collection involved, and a clean
`LOFT_STRICT_STORES=1` run.

| Cell | Shape | Expected |
|---|---|---|
| A1 | `for i in 0..3 { b.hs += [H { r: P { k: 10 + i } }] }` | `10 11 12` |
| A2 | `for i in 0..2 { b.hs[i].r = P { k: 20 + i } }` | `20 21` |
| A3 | `for i in 0..2 { b.hs[i].r = mk(30 + i) }` | `30 31` |
| A4 | `fn f(h: H) { h.r = P { k: 5 } }`, the caller reads `h.r.k` | `5` |
| A5 | `fn f() -> H { H { r: P { k: 6 } } }` | `6` |
| A6 | one holder outside a loop re-pointed to a literal 1000 times | the last value; 1 live record in the holder store with step 10, 1000 without |
| A7 | `h2 = h; h.r = P { k: 2 }`, then `h2.r` | the old record without step 10, null with it |
| A8 | a callee without the offered-record guard, into a loop holder | refused |
| B1 | `fn fill(h: H) { items = [P{k:1}, P{k:2}]; h.r = items[1] }` | `2` |
| B2 | B1 with `items[1].k = 9` after the write, read through `h.r` in and after the call | `9 9` |
| B3 | `fn f() -> H { items = [P{k:1}]; H { r: items[0] } }` | `1` |
| B4 | `fn f(o: Bag) { items = …; h = H { r: items[0] }; o.hs += [h] }` | the element's value |
| B5 | `-> Out { …; Out { h: h } }` | the record's value |
| B6 | `-> H { …; t = H { r: items[0] }; h = t; h }` | the record's value |
| B7 | two holders naming the same record, a write through one after return | visible through the other |
| B8 | the copied record holds a reference into the same local | refused |
| B9 | the holder re-pointed to a parameter's record before the local ends | no copy; the parameter's record |
| B10 | a nullable holder field null at the local's end | no copy, null |
| B11 | a vector, `sorted`, `hash` and `index` local as the home | each holder's value |
| C1 | `for i in 0..2 { p = P { k: i }; h.r = p }` | `1` |
| C2 | C1 with a second holder set in pass 0 | `0` without step 10, null with it |
| C3 | the holder declared inside the loop | that pass's value |

Axes the cells move: holder kind (local, element, parameter, returned value), home kind
(temporary, named struct local, vector, the keyed kinds), loop or not, a copy of the holder, and
the reference's nullability.

## Open design questions

1. **The contract:** approve *"lives as long as the store of its holder"* as the new
   `(H-RefHome)`.
2. **Step 10's semantics:** reclaim on re-point (copies made before then read null), or accept
   the growth with an advice where a loop re-points a holder declared outside it.  Recommended:
   reclaim; it bounds memory and never reads freed memory.
3. **Group A's direction:** keep it refused, since the message's rewrite (the collection beside
   the holder) is the clearer program?  Then steps 2 and 3 are skipped, and A1–A5 stay in the
   refusal guard.

## See also

- [formal/heap.md](../formal/heap.md) — `(H-RefHome)`, `(H-Removed)`, `(H-RefSet)`, and the code
  sites each step changes.
- [formal/rewrites.md](../formal/rewrites.md) — `(R-Place)`, whose claim and callee clause steps
  2–4 reuse.
- [DESIGN_DECISIONS_OWNERSHIP.md](../DESIGN_DECISIONS_OWNERSHIP.md) — C145, no back references.
- Guards: `tests/scripts/a-stored-reference-cannot-outlive-the-record-it-names.loft` (the shapes
  this plan admits) and `tests/scripts/a-record-a-stored-reference-names-lives-as-long-as-its-holder.loft`
  (the hoist already built).
- loft#1973, the issue `(H-RefHome)` closed.
