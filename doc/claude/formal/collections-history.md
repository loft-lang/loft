# formal/collections-history.md — the deviation register for [collections.md](collections.md)

> **The rules are next door.**  [collections.md](collections.md) states what must always be true of the
> language; this file is its TIMELINE — every place the code was measured not to do it, when,
> what it cost, and what closed it.  The two are apart because a contract a reader has to skim
> past its own history stops being a contract they can skim.  The rules doc carries the CURRENT
> state (how many are open, and which); everything below is the record behind it.

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
> so nothing is unfreed at exit.  Found in the `@FR-Col-Remove` walk (QUALITY.md B8f).
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

OPEN: **1** — `D-col-lookup` OPENED 2026-09-07 (loft#1450, below): `(Col-Lookup)`'s `τ?` is
recorded by a LINT FLAG and never reaches a type.  `D-col-null` was opened and CLOSED the same
day (2026-08-28, below).

### `D-col-lookup` — OPEN (2026-09-07, loft#1450): the rule's cited anchor is a lint switch, not a type

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

