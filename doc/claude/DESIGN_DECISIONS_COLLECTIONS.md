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

## C105 — a hash lookup keeps its TWO random reads (no hash-in-slot, no entries in the bucket table)

**Decision.** A hash lookup reads the bucket slot, then the entry; both stay random reads.  A hash
cached in the slot and entries in the bucket table are both declined; `reserve(h, n)` answers a fill
of known size.  **Why.** The cache saves at most ~12 % of a grown insert (none with `reserve`) for a
doubled table and a persisted-format break; in-table entries move on growth and make outstanding
`DbRef`s read wrong data ([COMPATIBILITY.md](COMPATIBILITY.md)).

**Revisit when.** A like-for-like measurement (same access order, same working set on both sides)
still shows a large gap; or reference stability into a keyed collection is given up; or the bucket
table's width changes for another reason.  Decided 2026-08-10 — [record](DESIGN_DECISIONS-history.md#c105--a-hash-lookup-keeps-its-two-random-reads-no-hash-in-slot-no-entries-in-the-bucket-table).
**Catalogue:** @F7 (`hash<T[keys]>` keyed collection).

## C113 — two `index` collections over one element type and key are refused, not made to work

**Decision.** A second `index` field with the same element type and key in one struct (or enum
variant) is a compile error at the declaration (`Parser::reject_duplicate_index`), naming the cures:
another kind (`hash` or `sorted`) over the same records, or another key; a different key, struct or
kind stays legal.  **Why.** An index keeps its tree links in the element record, allocated per index
TYPE, so two same-key fields are one tree through two roots and the first removal corrupts it.
Separate storage needs a schema id per field across four layers, and buys nothing: a second same-key
index answers what the first does.

**Revisit when.** A consumer needs two same-key indexes with genuinely different behaviour; a
filtered index would arrive as its own kind.  Decided 2026-08-14 — [record](DESIGN_DECISIONS-history.md#c113--two-index-collections-over-one-element-type-and-key-are-refused-not-made-to-work).
**Holds at:** `tests/scripts/902-duplicate-index-refused.loft` beside `902-duplicate-index-allowed.loft`.
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
