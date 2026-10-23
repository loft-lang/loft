<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Design decisions — collections

The entries of the [decision register](DESIGN_DECISIONS.md) about what a keyed collection is, how it is addressed, and what may be an element.  Each states
the decision, its reason and what would reopen it; its **record** link opens the question, the
trade-offs and the evidence in [DESIGN_DECISIONS-history.md](DESIGN_DECISIONS-history.md).

---

## C68 — Keyed collections dedup on insert (`+=` AND `coll[key]=value`)

**Decision.** On a `hash`, `sorted`, `index` or `trie`, both `coll += [entry]` and
`coll[key] = value` replace an existing entry with the same key — the latest insert wins, and
`len` counts keys.  A `spatial` is the exception: its key is a POINT, two records may share one,
and a box query answers both (formal/collections.md `(Col-Insert)`).
**Why.** A keyed collection promises key uniqueness; the world-chunk index needs an insert at an
occupied coordinate to replace, not stack a shadowed duplicate.

**Revisit when.** Bulk inserts of known-unique keys are measured too slow for the dedup check.
Decided 2026-05-21 (the same day's append/upsert split was reversed) — [record](DESIGN_DECISIONS-history.md#c68--keyed-collections-dedup-on-insert--and-collkeyvalue).
**Catalogue:** @F7 (hash), @F8 (sorted), @F9 (index).

## C99 — A keyed collection's subscript is uniformly KEY-addressed (lookup / range / removal), never positional

**Decision.** On every keyed collection (`sorted`, `index`, `hash`, `spatial`) `coll[k]` is a
key lookup, `coll[k] = null` a key removal, and on the ordered ones `coll[lo..hi]` a key-range or
proximity query; none is positional.  **Why.** The single subscript is already key-addressed,
so rejecting only the range form would make the surface less consistent and break `spatial`; the
likeness to a vector slice is inherent to keyed collections and is documented as a gotcha.

**Revisit when.** The owner prefers rejecting the range slice for a `.range()` call — which would
also have to re-home `s[key]` and exempt `spatial`.  Decided 2026-07-15 — [record](DESIGN_DECISIONS-history.md#c99--a-keyed-collections-subscript-is-uniformly-key-addressed-lookup--range--removal-never-positional).
**Catalogue:** @F8 (sorted) / @PLN102 arc-E lib-audit **H8** (INC#2). The freeze-time resolution of "the sorted key-range slice shares vector's positional-slice syntax."
**Holds at:** `tests/expressions.rs` `sorted_range_iteration`,
`sorted_subscript_is_key_addressed_not_positional`; [INCONSISTENCIES.md](INCONSISTENCIES.md) #2.

## C104 — a slice on a lazily-bound collection never fetches; a range is an explicit call

**Decision.** A slice on a lazily-bound collection reads the resident set like any read; a range
fetch is the explicit `store_lazy_range(c, lo, hi)`, an ad-hoc query `store_lazy_query`.  **Why.** A
lazy collection answers "what have I got", never "what exists" ([LAZY_STORES.md](LAZY_STORES.md)); a
fetching slice would make `xs[0..10]`, `for x in xs` and `len(xs)` disagree.

**Revisit when.** A consumer shows the explicit call is a real burden AND the honesty question has
an answer (e.g. a streaming collection whose `len` and iteration mean something else by
declaration).  Decided 2026-08-06 — [record](DESIGN_DECISIONS-history.md#c104--a-slice-on-a-lazily-bound-collection-never-fetches-a-range-is-an-explicit-call).
**Holds at:** `tests/lazy_sql_source.rs::a_slice_reads_what_is_resident_and_fetches_nothing`.
**Catalogue:** @F108 (lazy store binding)

## C105 — a hash that owns its entries keeps them in its bucket table; a shared one keeps tagged slots

**Decision.** A bucket slot carries a TAG, the low half of the key's digest, beside the entry it
names (`formal/collections.md` `(Col-Tag)`), so a probe reads only the entry it answers.  A hash
whose element type owns its entries keeps them IN the bucket table, one per bucket
(`(Col-Inline)`), so a hit is one random read.  A hash whose records a group member or a stored
`reference<T>` shares keeps them where they are and names them from tagged slots: those entries
never move.  `reserve(h, n)` answers a fill of known size.  **Why.** Measured like for like, the
two-read table left the keyed class a large gap that only the layout closed (the ledger:
`bench/portal/analysis/keyed.md`).  The objection to entries in the table — they move on growth,
so an outstanding `DbRef` reads the wrong data — holds only for a type something can name, and
such a type is never inline; a growth is a keyed add, which ends every view already
(`binding.md` B-Disturb).  The persisted-format cost is carried by @C152: the layouts live side
by side, so no store is converted.

**Revisit when.** Reference stability into an owned hash is promised; or a measured shape loses
with its entries in the table (a wide entry whose moves on growth outweigh the read saved).
Decided 2026-08-10, revised by @C152 2026-10-23 — [record](DESIGN_DECISIONS-history.md#c105--a-hash-lookup-keeps-its-two-random-reads-no-hash-in-slot-no-entries-in-the-bucket-table).
**Holds at:** `@C105` — `hash::SLOT_BYTES`, `TAGGED`, `INLINE` (`src/hash.rs`), guards
`tests/layout_golden.rs` `layout_golden` and `placement_contract_is_pinned`.
**Catalogue:** @F7 (`hash<T[keys]>` keyed collection).

## C113 — two `index` collections over one element type and key are refused, not made to work

**Decision.** A second `index` field with the same element type and key in one struct (or enum
variant) is a compile error at the declaration (`Parser::reject_duplicate_index`), naming the cures:
another kind (`hash` or `sorted`) over the same records, or another key; a different key, struct or
kind stays legal.  **Why.** Two keyed fields over one element type in one struct are two routes to ONE
record set (`(Col-Group)`), so a second `index` on the same key answers exactly what the first does
and costs a second tree to keep in step.  (Each index now keeps its tree in its own arena, @C147, so
the refusal no longer guards storage; it stands on this ground alone.)

**Revisit when.** A consumer needs two same-key indexes with genuinely different behaviour; a
filtered index would arrive as its own kind.  Decided 2026-08-14 — [record](DESIGN_DECISIONS-history.md#c113--two-index-collections-over-one-element-type-and-key-are-refused-not-made-to-work).
**Holds at:** `loft-suite/attic/scripts/902-duplicate-index-refused.loft` beside `902-duplicate-index-allowed.loft`.
**Catalogue:** @F9 (`index<T[keys]>`) · loft#902 · loft#843 (linked collection groups)

## C114 — a keyed collection is refused as a vector ELEMENT, not given an element form

**Decision.** `vector<hash<…>>`, and the same with `sorted`, `index`, `spatial` and `trie`, is a
compile error where it is written, naming the kind; the cure is a struct holding the keyed
collection, in a `vector` of that struct.  **Why.** No program could ever fill such a vector, and
giving a keyed type an element form is a design (what the vector owns, what a copy does to the
spines), not a missing emitter arm — the cheap arm would shift every runtime schema id after it.

**Revisit when.** A consumer shows the struct wrapper's extra record is measurably in the way; that
arrives with ownership rules, as an inline element region.  Decided 2026-08-15 — [record](DESIGN_DECISIONS-history.md#c114--a-keyed-collection-is-refused-as-a-vector-element-not-given-an-element-form).
**Holds at:** `Parser::refuse_keyed_vector_element` (`src/parser/vectors.rs`);
`tests/parse_errors.rs::every_keyed_kind_is_refused_as_a_vector_element`.

## C117 — a linked group's members must share one element LAYOUT; the tag-aware dense read is declined

**Decision.** A struct declaring both a dense `vector<E>` and a `vector<E?>` beside a keyed member
is refused; the message names the cures (`vector<E?>` for both, or drop the keyed member). **Why.**
One record set that may hold absence cannot be read through a non-null element type: left alone the
dense member silently leaves its group, and made to join it misreads the tag as data.  A tag-aware
dense read would give `vector<E>` a layout its author did not write (`(N-Dense)`).

**Revisit when.** Not stated; declined, not deferred.  Decided 2026-09-06 (loft#1385) — [record](DESIGN_DECISIONS-history.md#c117--a-linked-groups-members-must-share-one-element-layout-the-tag-aware-dense-read-is-declined).
Holds at `(Col-Group)`, [formal/collections.md](formal/collections.md).

## C118 — an append to an ABSENT collection instantiates it empty and fills it; refusing or warning is declined

**Decision.** `c += [x]` on a nullable collection holding null creates it empty and inserts, with
no diagnostic; `for` over and `remove` from an absent collection stay refused.  **Why.** Its main
producer is decoding a document with a missing key, and the append binds no element — its only
question is which store the record joins, and an absent collection has one sensible answer.

**Revisit when.** A consumer shows the silent instantiation hid a defect — evidence for an
`advice` note, not an error.  Decided 2026-09-07 (loft#1434) — [record](DESIGN_DECISIONS-history.md#c118--an-append-to-an-absent-collection-instantiates-it-empty-and-fills-it-refusing-or-warning-is-declined).
Holds at `(Col-Insert-Absent)`, [formal/collections.md](formal/collections.md).
**Catalogue:** @F1 (null model), @F38.

## C146 — A hash table's bucket count is a power of two

**Decision.** Every hash table loft sizes has a power-of-two bucket count: it starts at 16,
grows by doubling, and a `reserve` rounds up to the next power of two past the load trigger.
The home bucket stays `digest mod count`, and for a power of two it is computed as a mask.
**Why.** The count is chosen by loft, so it can be chosen cheap to divide — the division was
7–10 % of a lookup, on its critical path.  The placement formula does not change, so a store
written before keeps loading: its non-power-of-two tables take the division, are re-sized to a
power of two at their next growth, and no `placement::HASH` bump refuses them.  A
multiply-shift reduction and a per-table reciprocal were the alternatives; both move every
stored entry.  The price is memory: a reserved table rounds up as a grown one does, so its
bucket array (4 bytes a slot) can be up to twice what an exact reserve took.
**Revisit when.** A placement change is made for another reason (`placement::HASH` bumped):
then the division's fallback for older tables can go with it.  Decided 2026-10-11 (owner) —
[record](DESIGN_DECISIONS-history.md#c146--a-hash-tables-bucket-count-is-a-power-of-two).
**Holds at:** `@C146` — `hash::home_bucket` and `hash::room_for_buckets` (`src/hash.rs`), the
paged reader's two placements (`src/paged_reader.rs`), `formal/collections.md` `(Col-Bucket)`,
guard `tests/scripts/a-hash-table-has-a-power-of-two-bucket-count.loft`.
**Catalogue:** BOTH_BACKEND_SWITCHES.md `LOFT_NO_POW2_BUCKETS`.

## C147 — An `index` is a B+tree of key parts and record numbers, kept apart from the records

**Decision.** `index<T[k…]>` is a B+tree in an arena record of its own (`src/btree.rs`).  A node
holds 16 slots, each an 8-byte KEY PART (the first key field as an order-preserving integer: the
integer, a float's total-order bits, a text's leading bytes) beside a record number; a part tie
reads the record.  A text index's node whose parts commonly tie keeps its keys' shared prefix (up
to 24 bytes) and takes its parts past it.  The element record carries nothing for the index.
**Why.** The owner's point: the records are already there with their keys, so the tree needs only
enough of each key to steer — and a node of parts is a few cache lines a descent scans, where a
tree threaded through the records reads a different record per level.  It made fill and lookup
2–3.6× faster than the red-black tree it replaced (integer keys within 1.1–1.8× of Rust's
`BTreeMap`; text keys sharing long prefixes at or below it), `len` O(1), and every indexed record
9 bytes smaller.  A copy of the whole key in the node was priced and gave no gain over the part;
a walk that holds a slot position instead of a record was declined (a loop body may remove other
records by key mid-walk, which moves slots): it holds the record and takes a verified per-thread
hint, at the red-black tree's walk speed.
**Revisit when.** A walk costs a routine more than its lookups (the position cursor, guarded by a
structural version, is the alternative), or text keys share more than 24 bytes in bulk.  Decided
2026-10-12 (owner) — [record](DESIGN_DECISIONS-history.md#c147--an-index-is-a-btree-of-key-parts-and-record-numbers-kept-apart-from-the-records).
**Holds at:** `@C147` — `src/btree.rs` (`record_part`, `fit_text`, `step_record`),
`placement::INDEX` = 2 (`src/placement.rs`), `formal/collections.md` `(Col-Index)`, guard
`tests/btree.rs` (the tree against a sorted model; a walk under removals).
**Catalogue:** @F9 (`index<T[keys]>`) · @PLN190.

## C151 — A hash walks in key order; a walk that needs no order says `unordered(h)`

**Decision.** `for e in h` over a `hash` walks in KEY order, as (Col-Order) states, and no
rewrite changes that order behind the program's back — not for a body that only sums, and not
by speculating and checking.  A program that has no use for the order writes
`for e in unordered(h)`: the walk skips the key sort and visits the entries in slot order, an
order loft does not specify but keeps FIXED — it follows the inserts and removes that made the
table, never the table's seed, so a run repeats.  Insertion order is not promised: a removed
entry's slot is reused by a later insert.  On a collection that already walks without a sort
(vector, `sorted`, `index`, `spatial`, `trie`) `unordered` changes nothing.  The comparison
twins walk a hash in key order too (`by_key` in `bench/17_consumer_moros_dryopea/bench.rs`), so
a ratio compares like with like.
**Why.** The owner did not want a random walk on the hash: an order the program can observe
must be the one it was promised.  A closure-based reduction (`sum(h, |e| …)`) was declined as
harder to reason about; making storage order the DEFAULT was declined for now, because it
changes what existing programs print (COMPATIBILITY.md's breaking class) and storage order is
not insertion order once an entry is removed.
**Revisit when.** A compatibility break is made for another reason: storage order as the default
walk, with key order as the opt-in, can be weighed then against how often programs wrote
`unordered`.  Decided 2026-10-16 (owner) — [record](DESIGN_DECISIONS-history.md#c151--a-hash-walks-in-key-order-a-walk-that-needs-no-order-says-unorderedh).
**Holds at:** `@C151` — `hash::records_by_slot` (`src/hash.rs`), `Parser::parse_unordered_source`
(`src/parser/objects.rs`), `Parser::keyed_snapshot` (`src/parser/collections.rs`),
`formal/collections.md` `(Col-Unordered)`, guard
`tests/scripts/a-hash-walked-unordered-repeats-its-order.loft`.
**Catalogue:** @F7 (`hash<T[keys]>` keyed collection).

## C152 — A data structure may have several layouts side by side, and every build reads and writes them all

**Decision.** One collection kind may be implemented by more than one layout, provided the
layouts live BESIDE each other: each instance records its own layout in its header, one store
holds instances of every layout, and every build — the interpreter and `--native` — reads and
writes every layout.  Which layout a new instance takes is the writing build's policy, and a
value never depends on it.  A build that cannot read a layout REFUSES the store rather than
misreading it.  The hash is the first such kind: untagged, tagged and inline tables, owning
or borrowing (`formal/collections.md` `(Col-Layouts)`).
**Why.** A layout is chosen per shape for speed (@C105), and a store already written in an
older layout stays readable without a conversion: the format grows instead of breaking.  The
interpreter is held to the same layouts so a store moves between backends, and the
interpreter remains the reference that checks the native build's values.
**Owed.** The refusal: `placement::HASH` is still `"2"`, so a build older than the tagged and
inline tables reads one as untagged and misreads it.  The token changes to `"3"`, with a rule
that a build accepts an older token it can read ("3 reads 2"), before these layouts are
pushed.
**Revisit when.** A layout cannot record itself in its instance (it needs an outside schema to
be read); or keeping every layout readable costs a measured read on the common one.
Decided 2026-10-23 (owner) — [record](DESIGN_DECISIONS-history.md#c152--a-data-structure-may-have-several-layouts-side-by-side-and-every-build-reads-and-writes-them-all).
**Holds at:** `@C152` — `hash::hash_inline` and the `STRIDE_FLD` flags (`src/hash.rs`),
`formal/collections.md` `(Col-Layouts)`, guard `tests/store_persist_loft.rs`
`every_hash_layout_lives_beside_the_others_in_one_store` with
`tests/scripts/a-store-holds-every-hash-layout-side-by-side.loft`.
**Catalogue:** @F7 (`hash<T[keys]>` keyed collection).
