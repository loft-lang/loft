<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# formal/collections-history.md — the deviation register for [collections.md](collections.md)

> **The rules are next door.**  [collections.md](collections.md) states what must always be true of the
> language; this file is its TIMELINE — every place the code was measured not to do it, when,
> what it cost, and what closed it.  The two are apart because a contract a reader has to skim
> past its own history stops being a contract they can skim.  The rules doc carries the CURRENT
> state (how many are open, and which); everything below is the record behind it.

> **D-col-5 — CLOSED (2026-09-22; opened 2026-09-21, loft#1576) — a displacement in a linked
> group was decided per MEMBER.**  `(Col-Group-Dup)` says a repeated key displaces the older record from EVERY keyed
> member, and `(Col-Group)` that a record leaving through any member leaves every member.  Each
> keyed arm of `Stores::insert_record` displaces only what collided in ITS OWN key, and every
> member of a group only unlinks.  Two symptoms, both backends: members keyed on different
> fields disagree about the record set (`hash<B[bar]>` + `index<B[note]>`: `B{1,"x"}` then
> `B{1,"y"}` leaves `x` in the index after it left the hash), and a group with no vector member
> releases a displaced record nowhere (two records per repeated key).  The home for both is
> `record_finish`, after the fan-out; it needs every keyed arm to report what it displaced.
> Found by loft#1572's matrix, whose `children_live` cell had pinned the first symptom as
> design.
>
> **Closed** at the chokepoint the entry named.  Each keyed arm of `insert_record` (and
> `dedup_keyed`) now answers the record an unlink-only insert displaced, `link_siblings` collects
> them across the fan-out, and `Stores::settle_displaced` — reached from `record_finish` and from
> `link_record_siblings`, the element-write route — looks each one up under its OWN key in every
> keyed member and unlinks it where that answer is the record itself; with no vector member it
> is released.  The measure that found the third route: a whole-vector write reaches the members
> one `OpIndexGroup` per view, so a record displaced in one view stayed in every other; the op
> now carries the struct and the primary's field, and `settle_group` applies the per-record
> rule's final state after each view.  The rule gained its release clause, since "never a free"
> had been written for a group with a vector member.  Guarded by
> `1576-a-displaced-record-leaves-every-member-of-its-group` (both backends; values, lengths and
> record counts).
>
> **D-col-4 — OPENED AND CLOSED (2026-09-21, loft#1572) — a `sorted` stored by reference kept a
> repeated key.**  `(Col-Insert)` and C68 keep one record per key, latest insert wins.  A
> `sorted<T[k]>` whose element type any `hash` / `index` in the program also holds is the
> `ordered` layout, and `vector::ordered_finish` kept only the POSITION of its search, so the
> repeated key was shifted in beside the older record: an unrelated declaration changed what
> `+=` did, on both backends and in groups.  Closed by the insert's own search taking the found
> flag — the new record takes the older one's slot, as `sorted_finish` does inline — and
> `insert_record` releasing the displaced record unless a group sibling holds it.  Nothing
> registered it while it was open: `children_live` and `OBJECT_MAPPING.md` had recorded the
> stacking as the measured reason a `sorted` takes the SQL ordinal.  Guarded by
> `1572-an-ordered-collection-displaces-a-repeated-key` (nine cells, falsified at eb8702054 and
> by patch).

> **D-col-3 — OPENED AND CLOSED (2026-09-06, loft#1402) — a by-INDEX removal kept what the
> element OWNED.**  `(Col-Remove)` deletes one element and LOFT.md says `v#remove` "removes
> exactly one element, releases what that element owned".  `Stores::remove_vector_at`'s UNLINKED
> branch shifted the bytes and released nothing, so `v.remove(i)` and `e#remove` retained one
> record per removal: a constant population cost a record count that grew with the number of
> removals, without bound, on both backends.  The by-RECORD twin (`remove_owned`, reached by
> `c[key] = null`) released them and so did the LINKED layout, so one `sorted` leaked through
> `#remove` and not through `[key] = null`.
>
> The branch's own doc said why it thought it needn't: *"a `vector`/`sorted` holds its elements
> INLINE, so a slot is as wide as an element and there is no separate record to free"* — true of
> the element's own record, and false of its CLAIMS, which each live in a record of their own.
> Closed by walking them before the shift, through `get_vector` — the same index→element map
> `remove_vector` walks, answering `rec == 0` for exactly the indices that one removes nothing
> for, so the guard and the removal cannot disagree about which indices name an element.
> Release BEFORE the shift, because an inline element IS the slot; the linked branch can unlink
> first only because the record it names survives the unlink.
>
> **It could not close alone, and that is the entry's lesson.**  While a `??`-discharged binding
> stayed a live view of the removed element ([binding.md](binding.md) `D-bind-24` / loft#1401),
> releasing the element's children emptied a value the program was still reading —
> `445-generic-tree-walk.loft` measured it, and it was RIGHT to fail.  A leak that is
> load-bearing for a correctness bug is not an independent defect, and ordering the two was the
> whole of the work.
>
> Guarded by `a-vector-removal-releases-what-the-element-owned` (10 cells, both backends,
> falsified at 6609b01b — loft#1401's fix, i.e. this tree with only the release missing, which
> is the honest control since at any earlier commit its interaction cell would fail for the
> other reason).  Its oracle is FLATNESS, not a count: the absolute record count differs between
> the backends, so each cell runs one workload at two sizes and asserts the two agree.
> `collect_store_leaks` cannot see this at all — the records are retained inside a LIVE store,
> so nothing is unfreed at exit.  Found in the `@FR-Col-Remove` walk (QUALITY-history.md B8f).
>
> Filed as `D-col-2`, which loft#1385 had already taken and closed the same day; renumbered
> here.  The same collision happened to this issue's sibling in [binding.md](binding.md),
> twice — a deviation number is picked from the rules doc, which carries only the OPEN ones,
> so the closed ones it cannot see are exactly the ones a new entry collides with.

- **`C-Order`** (hash bucket-walk) — already a decided edge in concurrency.md; `Col-Order` references it.
- **`D-key-1`** (keyed slice = iterator) — a shipped decided edge (the value-position crash was fixed to a
  clean diagnostic, RELEASE.md 2026-07-04); formalized as `INV-KeyedSlice`, not an open deviation.
- **INV-Superset** — a deliberate design decision (raw Morton interval), not a deviation; record as an edge
  with a DESIGN_DECISIONS cross-link.
- **Candidate OPEN (verify):** the per-query scratch-vector allocation for spatial slices (CAVEATS.md notes
  it as the next efficiency lever) — a performance note, likely NOT a formal deviation.

OPEN: **1** — the chapter's own count, `collections.md` § Deviations, is the live one, and it
carries `D-col-6` (loft#1664, opened 2026-09-24: a write through a LINK to a group member
reaches only that member).  `D-col-5`
opened 2026-09-21 and CLOSED 2026-09-22 (loft#1576, above).  `D-col-4` opened and CLOSED 2026-09-21 (loft#1572,
above); `D-col-lookup` opened 2026-09-07 and CLOSED 2026-09-08 (loft#1450, below);
`D-col-null` was opened and CLOSED the same day (2026-08-28, below).

### `D-col-lookup` — OPENED 2026-09-07, CLOSED 2026-09-08 (loft#1450): the rule's cited anchor was a lint switch, not a type

`(Col-Lookup)` states `Γ ⊢ c[key] ⇒ τ?` — *"a keyed point lookup is NULLABLE — an absent key
yields the null record, discharged by `?? d` / `match` like any τ?"* — and cites
`fields.rs:700-706` as its anchor.  That site clears `expr_not_null`, which is the input to the
redundant-check and redundant-coalesce LINTS.  It is not the type.  So a lookup that misses in a
PRESENT collection binds into a non-null slot with nothing said:

```loft
h: hash<It[k]> = [];
e: It = h[1];        // silent; `e` typed non-null `It`, holds the null record
take(h[1]);          // and the same at the call-argument seam (loft#583's (N-Store) site)
```

All four keyed kinds answer alike (`hash`, `sorted`, `index`, `trie`), and the two arms in
`fields.rs` — `Hash|Radix|Trie` and `Sorted|Index` — each do only the clear.

**Closed** by giving both arms one home — `wrap_keyed_lookup_nullable` — which wraps the element
type for a POINT lookup only.  A spatial or trie RANGE slice answers the COLLECTION for an
enclosing `for`, and iterating one is total, so wrapping it would demand a discharge for an
absence that cannot occur; that control is the cell which says the widening did not over-reach.
The vector arm's two guards come with it: `pln25_dn1_enabled`, and the `tagged_pointer_type`
check that stops `@FR-N-Idem`'s `τ??` where the element is already a `__nullable<S>` slot.  An
UNRESOLVED element takes no marker either — `Optional(Unknown)` is not a type the compiler can
name, and a first-pass lookup is exactly that.

⚠ **"Deferred on cost (351 corpus sites)" was the wrong reading of its own measurement, and the
error is worth keeping visible.**  Swept file by file, the widening adds **zero** corpus
failures: those sites emit WARNINGS, which no run fails on, and the single file that did break
was this fix wrapping an unresolved element rather than a program needing migration.  What
actually blocked the leg was never migration — it was that the materialise/copy for a nullable
element view did not happen (loft#1456, five sites, both backends), which was invisible from
here because it is not a collections question.  The cost estimate was honest and measured; it
counted the wrong thing, and it read as a reason not to start.

That is also why the two were one fix: with loft#1456 closed, enabling this widening leaves
`146-keyed-rekey-through-view` green on both backends, every keyed write byte-identical, and the
slice controls unmoved.

⚠ **This is why this register could read `OPEN: 0` over it**, and the reason is worth stating
because it is not the usual one.  The rules were complete and the enforcement was not missing —
what was wrong is that the RULE'S OWN ANCHOR pointed at a lint switch, so a reader checking the
code against the rule would find a cited, existing, correct-looking site and stop.  A citation
resolving is not a citation enforcing.  This is a THIRD failure mode beside the two
[types-history.md](types-history.md) records (an incomplete rule; a complete rule nothing
re-measures), and the cheapest check for it is to ask what an anchor's site actually decides —
here, whether it writes a `Type`.

Measured before it was deferred: closing it costs **351 corpus sites** across 21 files, where the
receiver-absent half (`D-Null-Recv` in [types-history.md](types-history.md), closed 2026-09-07)
cost none.  It is deferred for that reason and for nothing else — the direction is not in doubt.
⚠ Whoever takes it: the receiver's `?` must NOT reach a keyed WRITE.  `h[k] = v` parses its target
through the same `parse_index`, and carried into the place the write is no longer recognised as
one and lowers to a READ, losing the write in silence — `(Col-Insert-Absent)` makes that write
total.  `parse_assign_op` is the chokepoint that peels it, keyed on `OpGetRecord`.

### `D-col-null` — OPENED AND CLOSED (2026-08-28, loft#1120): two answers to *"is this collection null?"*

`(Col-Lookup)` and `(N-Index)` make an absent element that type's null, and `(E-Coalesce)` makes
`e ?? d` yield `d` for exactly that null.  One value, one null, one answer — and the tree carried
two, each right about the half the other got wrong.

`??` asked `OpConvBoolFromRef` (`rec != 0`).  That reads the encoding a MISSED LOOKUP uses and
nothing else, so a nullable collection FIELD — whose read is a sub-reference carrying the HOLDER's
record — was "present" whatever the slot contained: the default was unreachable, and a `hash` /
`index` field then dereferenced the record the absent slot names and stopped the run.  `==  null`
asked `OpVectorIsNull`, which reads the handle sentinel and the slot word but called a record-less
DbRef present, so `vv[9] == null` answered `false` for an index plainly out of range.  `spatial`
and `trie` were in neither list: the coalesce's hand-written variants named `Vector`/`Sorted`/
`Hash`/`Index` only, so they fell to the generic convert, which hands back the bare handle —
`--interpret` read twelve pointer bytes as a boolean and `--native` would not compile the `if`.

Closed by giving the question ONE implementation: `vector::is_absent_collection` answers ABSENT for
a DbRef that reaches no slot (the missed-lookup encoding it used to call present), and the coalesce
asks `Parser::collection_is_null` — the lowering `== null` already used — through
`is_collection_type`, which names every kind including `Radix` and `Trie`.  The condition position
(`if c`) shares that lowering and was wrong in the same three ways.

⚠ **The oracle under the neighbouring `OPEN: 0`s could not see this.**  Five guards already covered
nullable collection fields (`909`, `917`, `920`, `922`, `936`) and every one of them writes `?? []`
— and empty is what the wrong answer looks like, so each cell agreed with itself.  A default whose
length differs from both the empty and the present arm is what separates them; that is what
`tests/scripts/1120-one-null-question-for-a-collection.loft` writes, over six collection kinds ×
{null, empty, filled} × {field, element field, parameter, handle, lookup} × {`??`, `== null`, `if`}.

## Carried by collections.md until 2026-09-04

The rules doc used to carry these beside its `OPEN` line — closure summaries, and notes on
the times the count read 0 over a live entry.  They are timeline, so they moved here
unchanged; [collections.md](collections.md) now states only what is open.

### `D-col-2` — OPENED AND CLOSED (2026-09-06, loft#1385): one element type, two layouts

A struct holding BOTH a dense `vector<E>` and a nullable `vector<E?>` beside a keyed member
split into TWO groups: a write through the dense vector reached only itself, one through the
keyed member reached the nullable vector and itself, and `len` of the member that never received
the record was a legal `0`.  `(Col-Group)` named that case in as many words — *"not about
whether the element is dense or nullable"* — so it read as a plain deviation.

**It was a conflict between two rules, not a gap in one.**  `(N-Dense)` says a `vector<E>`
stores `E` and its elements are non-null unless the author wrote `vector<E?>`.  One record set
that may hold absence cannot be read through a non-null element type, and the records are not
even the same shape: a nullable element is the tagged `__nullable<E>`, a dense one is `E`.

Both silent answers were measured.  The obvious fix — comparing the element through the
nullable peel (`Stores::key_owner`), so the rewritten keyed member and the dense vector compare
equal — DOES form the group, both ways, and the type dump shows every member linked.  Then the
dense member receives a record and misreads it: `a[0].n` answered `7` and `a[0].k` answered `2`,
the `Some` discriminant.  That is loft#1134's misread, a zero turned into garbage, and worse
than the zero.

**Status — CLOSED by a REFUSAL.**  The declaration has no coherent meaning and is declined
where the group would form (`Parser::refuse_mixed_nullability_group`, in the parser before
either membership derivation runs, so the `Stores::field` / `Parser::collection_groups`
agreement census is untouched).  The message names both cures.  `(Col-Group)` now states the
layout condition rather than leaving it to be re-derived, and the declined alternative — the
group adopting the tagged layout with tag-aware dense reads — is registered as C117 in
DESIGN_DECISIONS.md so it is not re-derived either.  Fires in all four declaration orders.
Guards: `1385-a-group-cannot-hold-one-element-two-ways.loft` and its controls twin
`1385b-a-group-agreeing-on-nullability-still-forms.loft`.  `Contract: strained` — a rule gained
a condition and a shipped declaration stopped compiling.

### `D-col-1` — CLOSED (2026-09-06, loft#1375): the keyed test was asked of the PAIR

`{ a: vector<E>, b: vector<E>, h: hash<E[k]> }` made the keyed member a HUB rather than the
group a set.  A write through `h` reached both vectors; a write through either vector reached
only `h`; each vector held its own entries plus whatever arrived through the hash.  Silent on
both backends — `len` of the short member is a legal `0`, the failure shape `(Col-Group)`'s own
paragraph warns about.

**No design call was open, though the issue was filed as one.**  `(Col-Group)` reads *"provided
at least one of THEM is keyed"*, where `them` is every collection over that element type in the
struct, and its second sentence settles the rest by being applied twice: if `a` and `h` are one
record set and `b` and `h` are one record set, then a record entering through `a` is in `h`, and
a record in `h` is in `b`.  The rule's last sentence needed qualifying rather than deciding —
two non-keyed members are independent exactly when the struct has NO keyed collection over their
element type.

**Two halves, and the second is what declaration order needs.**  `Stores::field` asks the keyed
question of the STRUCT now, not of the pair.  And because it runs once per field as the struct
is built, a keyed member arriving LAST has to join the members that were skipped while it was
absent: at the moment the second vector was added the struct held no key and the two were
correctly independent.  Without that half, `{h, a, b}` and `{a, h, b}` formed the group and
`{a, b, h}` did not — the declaration-order dependence loft#843 and loft#1158 had already
removed for the pairwise case, reappearing one level up.

Guard: `tests/scripts/1375-a-linked-group-is-a-set-not-a-hub.loft` — every declaration order,
every write route, three plain vectors on one keyed member, `sorted` and `index` as the keyed
kind, a nullable keyed member, and two controls: two plain vectors with NO keyed member stay
INDEPENDENT (the rule's last sentence, and the cell a fix that linked everything fails), and a
collection over ANOTHER element type is not a member.  Residual, measured and filed apart:
a dense vector beside a nullable one splits (D-col-2, loft#1385).  `Contract: settled` — the
rule already said the set; the test asked about a pair.

### the status line formal/README.md's area table carried until 2026-09-04

**SCOPE (2026-07-10)** — not yet rules: it inventories the shipped behaviour, names each rule with its anchor, and lists what must be both-backends-verified before it graduates to the normal form at 0 deviations. **`Slice-Open`/`Slice-Cap` now HOLD (2026-08-19, loft#1002)** — the open spatial slices answered the Z-order tail against a rule that already said *outward walk*, and open question 4 (`:n` exact-count) is answered: exactly n from any origin

## Deviations carried by collections.md until 2026-09-29

Closed entries moved here from the rules chapter's register (RELEASE.md § 5b), as written.

- **`D-col-20`** — opened and CLOSED 2026-10-07 (loft#1915): **`ix.len()` was refused on an
  `index`** ("Unknown field index<E[id]>.len") against `(Col-Len)`, while `len(ix)` answered:
  the length op takes the record-link offset, a constant only the parser knows, so no stdlib
  method carries it, and only the free spelling was routed.  Both now go through
  `Parser::index_len`.  Guard `1915-an-index-answers-len-in-both-spellings.loft`.
- **`D-col-19`** — opened and CLOSED 2026-10-07 (loft#1914): **`trie +=` of a held key kept both
  records**, so `len` counted the duplicate and a lookup answered whichever the tree's shape put
  first.  `(Col-Insert)` now states the repeated-key rule C68 decided — a key names one record at
  hash, sorted, index and trie; a spatial keeps every record at a point — and the trie insert
  displaces first, as a hash's does.  Guard `1914-a-trie-key-names-one-record.loft`.
- **`D-col-18`** — opened and CLOSED 2026-10-07 (loft#1913): **a `float`, `single` or `character`
  spatial axis was accepted**, against `(Col-Spatial)`'s integer-not-null coordinates; the
  encoding then missed the collection's own records (`(Col-Axis)`) and a float box query did not
  compile on `--native`.  The declaration refuses it.  Guard `1913-a-spatial-axis-is-an-integer.loft`.
- **`D-col-17`** — opened and CLOSED 2026-10-07 (loft#1912): **a keyed collection with an EMPTY key
  list compiled** (`hash<Q[]>`, `sorted<Q[]>`, `index<Q[]>`, `spatial<Q[]>`) and every insert
  replaced the last.  `parse_fields` refuses the empty list for every kind.  Guard
  `1912-a-keyed-collection-without-a-key-is-refused.loft`.
- **`D-col-16`** — opened and CLOSED 2026-10-07 (loft#1911): **an `index` element with a 2-byte
  field was refused** ("offset 17 is not a multiple of its alignment 2"): the packer placed the
  field after the 9-byte link triple without aligning it.  An appended field now starts on its
  boundary; every layout this changes was refused before.  Guard
  `1911-an-index-element-may-hold-a-two-byte-field.loft`.
- **`D-col-15`** — opened and CLOSED 2026-10-07 (loft#1910): **an `index` dropped records past tree
  depth 30**, silently, on both backends — the red-black descent bound answered a duplicate key,
  and sorted-order inserts lost every record from 98,303 on.  The bound is 64, the depth no tree
  of `u32`-addressed records reaches.  Guard `1910-an-index-holds-every-record-past-tree-depth-thirty.loft`.
- **`D-col-14`** — opened and CLOSED 2026-09-30 (loft#1760): **an absent keyed FIELD was followed as a
  record by nearly every reader**, against `(Col-Len)`, `(Col-Cons)` and `(Col-Copy)`.  `D-col-13`
  made the counts test the value-level null — what an absent LOCAL is — and its guard binds every
  collection to a local.  A null FIELD is not that null: its slot holds `DbRef::ABSENT_REC`
  (loft#917), which `Store::collection_rec` reads as "no records".  The vector side asked through
  it; `hash`, `radix_db`, `trie_db` and the `index` tree read the slot raw, and every arm of the
  owned walk (teardown, copy, `==`) tested the marker only to keep it from being REFUSED, then
  walked it.  So on a null `hash` / `sorted` / `index` / `spatial` / `trie` field, `len`, `= []`,
  `= [r]`, `= other`, `g = o.f`, `o.f[k] = null`, `= null` after a fill and a copy of the holding
  struct all panicked on `rec=4294967295`, both backends; `+=` did too on `trie` and `spatial`
  (loft#1213 fixed it for the other three).  The vector kind answered quietly wrong instead:
  `o.v = []` stayed `null`, and `g = o.v`, `o.v = src`, `o.v = f()` and `H { v: src }` of an
  absent source read back `[]`.  **Fix.**  The walk and the copy ask one question at their entry
  (`Stores::absent_collection_slot`) — an absent walk is an empty one, an absent copy is absent;
  the keyed modules read their slot through `collection_rec`; `vector::clear_vector` replaces the
  marker with the empty collection; `vector_replace` asks the source's SLOT; and a field declared
  `?` takes `OpReplaceVector` in every assignment arm and the constructor (loft#1319's rule for a
  local).  Guards `tests/scripts/an-absent-keyed-field-is-empty-to-every-reader.loft` and
  `tests/scripts/a-vector-field-keeps-its-absence.loft`; the second also moves the holder's field
  count, because a ONE-field holder's clear takes the root-reset path and was right.

- **`D-col-13`** — opened and CLOSED 2026-09-29: **`len` of a nullable KEYED collection was
  refused or crashed** where a `vector<…>?` warned and counted.  Three layers, one per kind
  family: `hash`, `sorted`, `spatial` and `trie` were warned by `(N-Store)`'s junction and then
  refused by `can_convert`'s keyed-to-generic arm, which matched the bare shape only ("expected
  hash, got hash<E[k]>?"); `index`'s own `len` route matched the bare shape and answered
  "Unknown function len"; and once admitted, an absent `hash`, `spatial`, `trie` or `index`
  panicked in a store accessor, because their counts indexed a store before testing the null.
  **Fix.**  The keyed arm reads through the `?`, the `index` route peels it and asks the store
  face itself, and the four counts test `is_null()` first, as `length_vector` did.  `(Col-Len)`
  now states the absent count.  Guard
  `tests/scripts/len-of-a-nullable-collection-warns-and-counts-an-absent-one-as-zero.loft`
  (six kinds, present and absent, both backends).

- **`D-col-12`** — opened and CLOSED 2026-09-29 (loft#1728): **a slice at a method's RECEIVER was
  not a vector.**  `(Slice-Value)` makes `v[a..b]` the fresh vector a bind would make at every
  vector-typed position, and a receiver is one — but it is resolved by its TYPE before any
  coercion sees it, so `len(v[a..b])` answered "Unknown function len" and `v[a..b].len()`
  "Unknown field iterator<integer>.len", on both backends.  **Fix.**  Where resolution would
  otherwise fail, the slice receiver goes through `iterator_as_vector`, the one home: the free
  spelling in `Parser::call_with_slice_receiver`, the method spelling when the member is a
  method a vector declares and no iterator does (`Parser::slice_receiver_method`).  A keyed
  range slice stays `(Slice-KeyedIter)`'s refusal.  Guard
  `tests/scripts/1728-a-slice-is-a-vector-as-a-methods-receiver.loft`.

- **`D-col-11`** — opened and CLOSED 2026-09-28: **a comprehension over a keyed collection walked
  the collection, not its snapshot**, against `(Col-Order)`.  A `for` statement walks a `hash`,
  `spatial` or `trie` through the ordered snapshot `parse_for` built; a comprehension over the same
  source built none and stepped the COLLECTION in the snapshot's mode — a non-empty hash panicked
  on a record number read out of its table header, a two-key trie answered `[null]`, a four-point
  spatial one element, the last two silently, on both backends.  **Closed**: `keyed_snapshot` is
  the one home both walks call, and the comprehension releases its snapshot when its loop ends.
  Guard `tests/scripts/a-comprehension-over-a-keyed-collection-walks-its-snapshot.loft`,
  falsified at `49350ce2b`.

- **`D-col-10`** — opened and CLOSED 2026-09-28: **a range slice's cap below zero answered every
  record**, against `(Slice-Cap)`'s "capped at n".  The lowering spelled "no `:n` written" as the
  limit `-1` and the runtime read every negative limit as that flag, so `xs[(x,y)..:k]` and
  `t[pre..:k]` with `k` gone negative (an overspent budget) or null answered the whole collection,
  on both backends — silently, since a cap has no answer that looks wrong.  **Closed** by moving the
  flag out of the program's reach: `SLICE_UNCAPPED` is `i64::MAX`, a cap that means "uncapped"
  whoever writes it, and `slice_cap` is the one reader, for the spatial and trie builders both.
  The paged loaders (`store_load_prefix`, `store_load_box`) keep "negative means no cap": theirs
  is a documented function parameter, not this rule.  Guard
  `tests/scripts/a-slice-cap-below-zero-answers-no-records.loft`.

- **`D-col-7`** — opened and CLOSED 2026-09-25 (loft#1670): **`insert` and `reverse` did not act
  on a LINKED vector's layout.**  A `vector<T>` is linked — each slot a 4-byte record id, each
  element a record of its own — as soon as any keyed collection over `T` exists, which the author
  never spells at the call site, so neither may change what these operations do.  Two defects:
  `Parser::element_store_size`, the `@FR-H-Stride` home, answered the STRUCT's size for a linked
  element, so both operations slid the wrong span (`reverse` of `1,2,3` read `null,null,3,`); and
  with the stride right, `insert` still wrote the element's FIELDS over the slot's record id
  (`1,4294967298,null,3,`), and a group insert reached no keyed sibling.  **Closed** at the
  stride home (it answers 4 for a linked element), and at one runtime body both backends' ops run,
  `Stores::insert_vector_element`: a linked slot is given a record claimed the way an append claims
  one, its id written into the slot, and the RECORD answered for the fields to be written into.
  `parse_insert` then hands the element to a group's keyed siblings (`OpLinkRecord`), since an
  insert, unlike an append, reaches no `OpFinishRecord`; the group site is the one
  `vector_group_site` derivation an element write through the group already reads.  Guards
  `tests/scripts/1670-a-vector-operation-walks-the-stride-its-layout-has.loft` (reverse, both
  layouts) and `tests/scripts/1670-an-insert-writes-the-element-its-layout-holds.loft` (10 cells:
  every index, negative and out-of-range, a text-owning element, a copied variable, an insert
  then a remove, and three through a group), falsified at `ea45d5fdf` on both backends.

- **`D-col-6`** — opened and CLOSED 2026-09-24 (loft#1664): **a write through a payload BINDING
  to a group member kept the field spelling after `(B-View)` had materialised the binding**, so
  the write went to the reassigned subject while the reads came from the copy
  (`binding_len` 1, the subject 3), which is loft#1662's split surviving exactly where the
  resolution must stay.  The `&` LINK spelling of the same question was settled the same day by
  making a collection link reach `(B-Ref-Reshape)` (`D-bind-60`), so it can never be downgraded
  to a copy and the field it named at the bind is the field it still names at every write; three
  sites that build an appended record (`build_vector_list`, the keyed `+= <elem>` fast path and
  the keyed `+= [ … ]` list path) now share `Parser::resolved_group_write`, and membership is
  asked both ways (`Stores::field_is_group_member`).
  **The binding half is closed where the materialise is decided.**  The field-spelled write is
  recognised by its ELEMENT, whose type records the binding it lives in, and `new_record` records
  on the `Function` the collection type the binding's own spelling passes
  (`group_write_views`).  The walk counts such a write as a USE of the binding; where the binding
  is condemned, the scope pass spells `OpNewRecord` / `OpFinishRecord` back to
  `(binding, its collection type, u16::MAX)`, so the write lands in the copy — which is what
  `(B-View)` says a materialised view's write does.  Undisturbed, the write keeps the field
  spelling and still reaches every member.  A KEYED member needed the materialise itself first
  ([binding.md](binding.md) `D-bind-61`).  Guard
  `tests/scripts/1664-a-group-member-payload-binding-materialises-like-any-view.loft`, 11 cells
  (vector and keyed members, both declaration orders, a back edge, the `is` spelling, a
  two-element append, and the undisturbed controls), falsified at `b938c9cb4` on both backends;
  `1160-…` and `1662-…` unchanged.
