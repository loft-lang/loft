<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/collections.md — collection kinds, indexing & slicing (strict) · **SCOPE**

> **STATUS: SCOPE (2026-07-10).** This is the *scoping* pass, not the finished rules. It
> inventories the shipped behavior of loft's collection kinds — `vector`, `hash`, `sorted`,
> `index`, `spatial` (`Radix`), `trie` — and the shared index/slice surface, names each rule with its
> intent + anchor, and lists what must be **both-backends-verified** before the rules graduate to
> the normal `## Rules` / `## Deviations` form at **0 deviations**. This is **describe-SHIPPED-
> behavior** mode (the usual `formal/` discipline — the opposite of the spec-first @PLN35 work).
>
> **Why this doc exists.** `spatial` shipped (PR #550) into an area the formal spec never covered:
> the collection **type formers** aren't in [types.md](types.md), and **indexing/slicing** is
> nowhere. Today the only keyed-collection rule is [concurrency.md](concurrency.md)'s `C-Order`
> (hash bucket-walk under `par`) plus passing mentions in [calls.md](calls.md) / [capabilities.md](capabilities.md).
> This doc closes that gap. It also firms up the ground [matching.md § PEG patterns](matching.md)
> (@PLN35) stands on — its pattern cursor reuses the same slice/`⟨i, src⟩` surface.

---

## 0. The shared slice mechanism (the load-bearing structural fact)

Slicing is **one type-directed mechanism**, not per-structure code. The index/slice parser
(`src/parser/fields.rs`, the `[...]` dispatch at ~`:660-717`) branches on the receiver type into
**two families** with different result kinds:

| family | receiver | form | result | anchor |
|---|---|---|---|---|
| **Value slice** | `vector<T>`, `text` | `v[a..b]` `[a..=b]` `[a..]` `[..b]` | a **fresh sub-collection VALUE** (`vector<T>` / `text`); bounds clamp | LOFT.md:1203-1206, :790-813; fields.rs:670-679 |
| **Keyed range-slice iterator** | `sorted`, `index`, `spatial`(`Radix`), `trie` | `c[lo..hi]`; spatial `c[(x,y)..(x,y)]` / `[(x,y)..]` / `[(x,y)..:n]`; trie `c[pre..]` | a **`for`-only iterator** (`Value::Iter`) over the raw KEY interval — **not a value** | fields.rs:688-717, :1228 (`D-key-1`), :779 (trie); STDLIB.md:277-281 |

`hash` has **point lookup only** (`h[key]`) — no order ⟹ no range slice. `spatial` is the
**multi-axis (tuple-key, Morton) instance** of the keyed-range-slice family; nothing about slicing
is spatial-specific except the tuple key and the Morton interval. `trie` is the **single-`text`-key
instance**, whose range is a PREFIX rather than a scalar interval (`parse_trie_slice`; a trie never
reaches the generic scalar-range branch). This split is the doc's spine.

---

## 1. Rule inventory (labels + intent; each grounded in shipped behavior)

### 1.1 Collection kinds & type formers — `Col-Type` (some land in [types.md](types.md))

```
  (Col-Vec)     vector<τ>                     ordered, 0-based integer index; the only value-sliceable collection.
  (Col-Hash)    hash<T[k…]>                   O(1) point lookup by key; UNSORTED (bucket) order.
  (Col-Sorted)  sorted<T[k…]>                 red-black tree; KEY-ORDERED iteration + range slices.
  (Col-Index)   index<T[k…]>                  BOTH — a red-black tree AND a hash table over the same records
                                              (O(1) lookup AND ordered/range).  (DATABASE.md:704)
  (Col-Spatial) spatial<T[a]> / [a,b] / [a,b,c]   1–3 coordinate axes (MAX_AXES=3), Morton/Z-order radix tree;
                                              the runtime Parts variant is `Radix`.  integer-not-null coord keys;
                                              negative coords via offset-binary (signed axes order like sorted).
  (Col-Trie)    trie<T[k]>                    a radix tree over ONE `text` key field: exact lookup, KEY-ORDERED
                                              iteration, and a PREFIX slice — the operation the kind exists for.
                                              Shares `radix_tree` with `Radix` and nothing above it: `Radix` is
                                              GEOMETRIC (Morton interleave, boxes, nearest) and none of that
                                              means anything for a word, which is why `spatial` is not spelled
                                              `radix` at the surface.
```
*Anchors:* `Type::{Vector,Hash,Sorted,Index,Radix,Trie}` (src/data.rs); DATABASE.md:693,:704; spatial
surface tests/scripts/48-spatial-construct-free.loft; trie `Parts::Trie` (database/mod.rs:194) + @PLN134. **To decide when writing:** which formers
live here vs in types.md's former list (recommend: types.md gains the one-line formers; collections.md
owns their *operations + order*).

### 1.2 Construction, insert, length — `Col-Cons` / `Col-Insert` / `Col-Len`

```
  (Col-Cons)    c: <kind><…> = []             empty-literal construction (all kinds).
  (Col-Insert)  c += [ rec, … ]               append/insert a record; keyed kinds place it by key.
  (Col-Len)     c.len()                        element count; O(1) (verified O(1) for spatial).
```
*Anchor:* tests/scripts/48-spatial-construct-free.loft (construct/append/len).

**What `Col-Insert`'s source may BE, and what it may not.** The rule is written over one
spelling, `c += [ rec, … ]`, and three source shapes satisfy it: the collection ITSELF (for a
vector, concatenation), ONE element, and a `vector` OF elements (which a keyed kind places one
by one — loft#1159).  Anything else is not an insert at all and is refused, by
[types.md](types.md) `C-Only`: `⤳` is the only implicit coercion, so a value the collection
cannot hold has no reading here.  A source that IS holdable but is spelled another way is
still an insert — [types.md](types.md) `C-Var` makes a VARIANT one element of a vector over
its enum, and a nullable record reaches a keyed kind as `Reference(d)` and a vector as the
synthetic `Enum(d, true, …)`, which are two spellings of the same element and not two
questions (loft#1215, loft#1221).

Two source shapes the rule deliberately does NOT license, each with its own answer:

  * **a whole KEYED collection into another of the same type.**  `Col-Insert` is about records
    joining a collection and says nothing about merging two, so this is refused rather than
    implemented.  Not a permanent decision — the rule could grow a merge — but silence was
    never one of the options, and it is what shipped until loft#1221.

    What it did instead of merging is the reason refusing is rule-CONSISTENT rather than merely
    safe: at a keyed FIELD it emitted no write at all, and between two keyed LOCALS it REBOUND —
    the IR is a plain `b = a`, so the destination is repointed at the source's store and takes a
    dep on it.  From an empty destination that is indistinguishable from a successful merge;
    from a populated one the destination's own records are gone, and mutating the source
    afterwards moves the destination.  No rule in this document admits an aliasing rebind, and
    `(T-Cons)`'s independence requirement for a keyed tuple element says the opposite — so the
    behaviour was not an unwritten merge waiting to be documented.  A real merge would have to
    walk the source and insert each record with the dedup `Col-Insert` already performs; there
    is no such copy anywhere in the language today, which is what `tuples-history.md`'s
    `D-tup-4` keyed half has been waiting on (loft#1230).
  * **a bare element into a VECTOR.**  Legal by this rule and refused by @PLAN52's ambiguity
    rule instead: `vector<vector<T>> += vector<T>` is both "push one element" and "concatenate",
    so the brackets are required for every element type, not only the ambiguous ones — and for
    every SPELLING of an element type.  A `τ?` source is one, because [types.md](types.md)
    `N-Opt` gives it τ's storage plus one reserved null: nullability does not decide whether a
    bare element is ambiguous with a concat.  Asked unpeeled, `d.c += n` on an `integer?` was
    accepted where the dense `d.c += 9` is refused, which made the `?` spelling of a statement
    more permissive than the plain one (loft#1223).

### 1.2b Removal — `Col-Remove` (a vector RENUMBERS; a keyed kind does not)

```
  (Col-Remove)      v.remove(i)  ·  v#remove  ·  c[key] = null  ·  e#remove
                    delete one element.  The two kinds differ in what happens to the OTHERS:
  (Col-RemoveDense) a VECTOR stays DENSE.  Removing index i shifts every later element down
                    one, so len decreases by 1 and every position after i is RENUMBERED.  There
                    are no holes and no tombstones: index j > i now names what was at j+1.
  (Col-RemoveKeyed) a KEYED kind (hash / index / sorted / spatial / trie) removes BY KEY, and every
                    other key stays reachable and unchanged — keys are not positions, so nothing
                    is renumbered.
```
*Anchor:* tests/scripts/200-vector-stays-dense.loft (density); measured for the keyed kinds —
`s[30] = null` on a `sorted<Elm[key]>` leaves `s[10]` and `s[50]` intact.

**Why this rule is load-bearing beyond collections.** `Col-RemoveDense` is exactly what makes a
held element reference go stale: a reference names a POSITION, and a removal renumbers positions,
so a reference taken before the removal names a different element after it. That is the fact
[binding.md](binding.md)'s `B-Disturb` / `B-Ref-Reshape` and [heap.md](heap.md)'s `H-Materialise`
rest on, and it is why density is a *contract* rather than an implementation detail: a
hole-punching vector would keep references valid and was decided against (@PLN130 F3) because
every read would then pay for the check.

### 1.3 Point lookup is nullable — `Col-Lookup` (reuses [types.md](types.md) `τ?`)

```
  (Col-Lookup)  Γ ⊢ c[key] ⇒ τ?              a keyed point lookup is NULLABLE — an absent key yields the
                                              null record (P285), discharged by `?? d` / `match` like any τ?.
```
*Anchor:* fields.rs:700-706 (P285, the `expr_not_null` clear); mirrors types.md `(N-Index)` for `v[i]`.

### 1.4 Iteration order per kind — `Col-Order` (EXTENDS [concurrency.md](concurrency.md) `C-Order`)

```
  (Col-Order)   for x in c { … } visits in a per-kind ORDER, identical on both backends:
                  vector  → index order 0,1,2,… (iteration.md I-For)
                  hash    → KEY order, via the ordered snapshot the walk builds for it
                            (its `par` walk is the UNSORTED one — concurrency.md C-Order)
                  sorted  → key order
                  index   → key order (its tree side)
                  spatial → Morton / Z-order
                  trie    → key order (lexicographic over the text key)

  (Col-Order-Sign)  a `-` on a key field is applied by the COMPARATOR and by nothing else, so
                    exactly once.  The stored form (a red-black tree, a sorted vector) is
                    therefore already in the declared order, and every reader walks it FORWARD:
                    no consumer of an ordered collection re-reads `keys[i].type_nr` to decide a
                    direction, and the iterator's reverse bit carries one fact only — did the
                    caller write `rev(...)`.  It follows that a range names its bounds in the
                    COLLECTION's key order (`ix[from..till]` starts at `from` and walks toward
                    `till` whichever way the keys are declared), and that `sorted` and `index`
                    answer identically for the same declaration.
```
*Anchor:* concurrency.md `C-Order` (hash); STDLIB.md/DATABASE.md (spatial Morton). **This is the
divergence-prone rule** (interp store-walk vs native emitted loop) — the whole reason the area needs
pinning. `C-Order` already states the hash edge; `Col-Order` generalises it to every kind.

**The hash line read the opposite of the rule it cites, and of what ships, until 2026-09-06.**
It said *"UNSORTED bucket walk (no key order) — the C-Order decided edge"*, and the edge
`C-Order` actually decides is the other one: the SEQUENTIAL walk is key-ordered and only the
`par` walk gives that up, "because the parallel queue has no use for key order". Measured on
both backends: a `hash<E[id]>` filled 49 down to 0 iterates 0,1,2,…,49, a `hash<E[k]>` on text
iterates alphabetically, and the same collection under `par(…, 4)` comes out scrambled. The
parser builds the ordered snapshot that makes it so (`parse_for`'s `hash_scratch`, an O(n log n)
key sort for a hash and nothing for a radix, which is already ordered), and LOFT.md and
STDLIB.md both describe it — *"hash iterates via its internal ordered index"*. So this was a
transcription inverted in one place, not a rule the code had drifted from: the code, `C-Order`
and the user-facing docs already agreed, and only this line dissented. Found in the
`@FR-Col-Order` walk (QUALITY.md B8g).

`Col-Order-Sign` is the half that was violated rather than merely unpinned. `index` applied the
sign a second time in two places — the iterator bit (`fill_iter`) and the range-cursor bound swap
(`tree::range_cursors`) — and reversing a total order twice is the identity, so every query on a
descending `index` answered the exact reverse of its declaration. One key hid it (`[-nr]` reversed
reads as `[nr]`); two keys did not, because `[-nr, key]` reversed is `[nr, -key]`. `sorted` never
carried either site, which is why it stayed correct and is the oracle a guard pairs against
(loft#1267).

### 1.5 Value slices (vector / text) — `Slice-Value`

```
  (Slice-Value)  v[a..b] / v[a..=b] / v[a..] / v[..b]  yields a FRESH sub-collection value:
                   vector<τ> → a fresh vector<τ> (H-Alloc); text → a text substring.
                 Bounds CLAMP: a partial-OOB slice returns the in-range part; a fully-OOB slice ⟹ [].
                 `..` is end-EXCLUSIVE, `..=` end-INCLUSIVE.  (Index vs slice asymmetry for text:
                 v[i] ⇒ character, v[i..j] ⇒ text.)
```
*Anchors:* LOFT.md:1203-1206, :790-813; clamp behavior plans/25-nullable-sequences/README.md:234.
**To verify when writing:** the exact clamp values on both backends; freshness (a value slice is
independent of the source — cross-link heap.md H-Alloc / iteration.md I-Comp).

### 1.6 Keyed range-slice iterators — `Slice-KeyedIter` (`D-key-1`, the shipped decided edge)

```
  (Slice-KeyedIter)  a keyed range slice c[lo..hi] is a `for`-ONLY ITERATOR (Value::Iter) over the raw
                     KEY interval, in the collection's key order.  It is NOT a value: `x = idx[lo..hi]`
                     in value position is a STATIC ERROR ("a keyed range slice is a for-loop iterator,
                     not a value — iterate it").  (Applies to sorted / index / spatial / trie.)
```
*Anchors:* fields.rs:1228,:1237 (`D-key-1`); RELEASE.md (the D-key-1 crash-fix, value-position reject);
STDLIB.md:281. sorted-slice design: [../plans/38-sorted-slice/](../plans/38-sorted-slice/).

### 1.7 Spatial slices — `Slice-Spatial` (the Morton specialization of `Slice-KeyedIter`)

```
  (Slice-Box)    xs[(x1,y1)..(x2,y2)]   iterate records whose MORTON code is in [code(x1,y1), code(x2,y2)],
                                        in Morton order.  This is a SUPERSET of the geometric box — Z-order
                                        threads codes outside the box IN, so the caller filters/`break`s for
                                        an exact shape.  (INV-Superset — a deliberate contract, not a bug.)
  (Slice-Open)   xs[(x,y)..]            open outward walk from a point; the caller `break`s to stop.
  (Slice-Cap)    xs[(x,y)..:n]          same, capped at n records (k nearest-in-Morton).  EXACTLY
                                        n when the collection holds n — the cap does not vary with
                                        where the query sits (answers open question 4 below).
                 1–3 axes; lowers to n_spatial_range(...); the same scratch path as iteration.
```

> **`Slice-Open`/`Slice-Cap` HELD only from 2026-08-19** (loft#1002). Until then both lowered to
> `radix_db::range` — the one-directional walk — so they answered the Z-order **tail**: records
> at or after the query only. A record one code behind was unreachable however close it was
> (from `(12,11)`, `C` at distance 2.2 was never returned while `E` at ~12 was), and the cap
> under-delivered by however close the query sat to the end of the curve — measured 3, 3, 3, 2,
> 1, 0 over five records as the query moved along, with a query past every record answering
> nothing at all. The rule above is what settled it: the issue proposed *"keep the tail and
> rename it"* as an equal option, and it was not one — the code changes to match the rules.
> Now lowers to `radix_db::near_range`, the n-axis form of `spatial::near` (two cursors seeded
> either side of the query, each step yielding whichever is closer), which existed, was correct,
> was unit-tested, and no loft program could reach.
>
> **The walk is APPROXIMATE and the rule says so** — `k nearest-in-Morton`, not nearest-in-space.
> Morton distance tracks euclidean distance closely but jumps at quadrant boundaries, so a
> truly-near point can arrive a place late; `Slice-Box` is the exact form. Every record is
> yielded eventually, each once, which is what makes `break` the intended way to stop.
*Anchors:* fields.rs:688-696,:1558 (parse_spatial_slice); default/01_code.loft:1176
(`spatial_range`); STDLIB.md:272-281; DATABASE.md:668-674; radix_db.rs:238 (superset comment);
tests/scripts/48b-spatial-slice.loft (the asserted box/open/cap slices). CAVEATS.md:593 (spatial op set).

### 1.8 Storage & whole-value copy — `Col-Store` / `Col-Copy` (cross-link [heap.md](heap.md))

```
  (Col-Store)   a collection is store-backed (Parts::{Vector,Hash,Sorted,Radix,Trie}); index = tree+hash over
                one record set; addressed by DbRef.  (Layout/format ⟶ layout.md; steps ⟶ heap.md.)
  (Col-Copy)    a keyed whole-value bind COPIES (g = h; g += … leaves len(h)) — heap.md H-Copy for keyed.
```
*Anchors:* DATABASE.md:693,:704; VERIFICATION.md heap.md "H-Copy (keyed)" (oracle `16`).

### 1.9 The linked GROUP — `Col-Group` (cross-link [DATABASE.md § Clearing one member](../DATABASE.md))

```
  (Col-Group)   two or more collections over ONE element type in ONE struct are several ROUTES
                to a single record set, provided at least one of them is keyed.  A record
                entering through any member is in every member, and a record LEAVING through
                any member leaves every member, by any write route — the element-level writes
                through the vector member included (v[i] = e, v[i] = null, v.remove(i),
                e#remove), where a replaced record keeps its identity and is indexed again
                under the key it now carries.  Membership is a fact about the PAIR — not about
                declaration order, not about which member is written first, and not about
                whether a MEMBER itself is nullable (hash<E[k]>? is a collection over E in
                that struct, so it is a member).
                The members must share one element LAYOUT, though: a nullable element is the
                tagged __nullable<E> (a discriminant plus the payload) and a dense one is E
                itself, so a dense vector<E> and a vector<E?> cannot both be routes to one
                record set, and a struct declaring both beside a keyed member is REFUSED
                (loft#1385).  Every member dense, or every member nullable, is one set.
                Membership is the whole SET, not a pair: *at least one of THEM* is a question
                about every collection over that element type in the struct, and the second
                sentence settles the rest by being applied twice — if a and h are one record
                set and b and h are one record set, a record entering through a is in h, and a
                record in h is in b.  Two members neither of which is keyed are INDEPENDENT
                exactly when the struct has NO keyed collection over their element type.
  (Col-Group-Dup) a key already held by a keyed member, entering AGAIN through ANY member,
                displaces the OLDER record from every keyed member and leaves it in the
                vector, which has no key to refuse on: es += [E{k:7}]; es += [E{k:7,n:"dup"}]
                reads len(es) = 2, len(by_k) = 1, by_k[7].n = "dup", and the same through
                by_k — the group's dedup is unlink-only, never a free (loft#1226).
```

Seven fixes are all instances of this one rule, which is why it is written here rather than left
to the issues: `trie`/`spatial` were absent from the pairing test (loft#927); the `others` link
ran one way, so which member maintained the rest depended on declaration order (loft#843); the
test asked only whether the field being ADDED was keyed, so a plain `vector<E>` declared second
formed no group (loft#1158); only `hash` had its element rewritten to a nullable sibling's
`__nullable<E>`, so the other four kinds no longer matched by content; a whole vector VALUE
(`data = rows()`) reached only the member it was assigned to, because the bulk write never
passed the per-record chokepoint that maintains the group (loft#1152, and loft#1159 for the
same route into a KEYED member); the same nullable-element rewrite asked both of its halves
with a bare variant test, so a member spelled `hash<S[k]>?` — or a vector spelled
`vector<S?>?` — fell out of the set entirely (loft#1204); and the keyed test was asked of the
PAIR rather than of the STRUCT, so two plain vectors beside a keyed member skipped each other
and the keyed member became a HUB — a write through it reached both vectors, a write through
either vector reached only it (loft#1375).

Every one of them **failed silently** — the pairing was never refused, a second independent
collection was built instead, and `len` of the empty view is a legal value.  That is the shape
to test for: a group's failure mode is not an error, it is a zero.

**The demonstration, on real data, in one line of arithmetic.**  `tools/indexer/src/scan.loft`
declared its distinct-tag set and its distinct-link-target set over ONE element type, so they
were one set.  `make index` over this repo reported

```
before   1781 distinct tags   1781 link targets     ← identical, both are |tags ∪ links|
after    1002 distinct tags    779 link targets     ← 1002 + 779 = 1781
```

Two counts reading the same number is the suspicion; the two halves summing to it is the proof,
and it takes one line to state.  Reach for that shape whenever a group is suspected — a merged
set and a coincidence are hard to tell apart by eye and trivial to tell apart by addition.  The
same run shows why the zero is only half the failure mode: this group's members were both
NON-empty and both wrong, because each walk saw the union (every link target was emitted with a
tag bucket, and every tag with a link bucket).  A group that fails by over-filling reads as a
plausible number rather than as a zero.

Confirmed independently from a second checkout: 779 of that build's index entries are
path-shaped link targets, matching the after-count exactly from a tree built before the fix.

⚠ **Not settled by this rule: which member HOLDS the records.**  The first-declared member is
the holder and the rest are views.  loft#1158 predicted that a keyed-first group would need the
vector made holder regardless of order; measured, it does not — all four write routes
(element-wise `+=`, whole vector value, keyed literal, keyed `+=`) read back complete through
both members in both orders, on both backends, under `LOFT_STRICT_STORES=1`, with no holder
machinery touched.  The holder choice is not observable through the rule, so the rule does not
name one.

*"By any write route"* is the clause the binding spellings broke and now hold to: a write
through a variant's `match` / `is` payload binding is resolved back to the field it projects,
so it reaches the group exactly as the direct spelling does (loft#1160, and loft#1161 for the
`is` capture, whose write did not even reach the subject).  A capture spanning ALTERNATIVES is
the one route still outside it, and not by omission — it picks its origin from the runtime tag,
so there is no one field to resolve it to.

**The element-level writes through the vector member were the routes that reached no
chokepoint** (2026-09-05, the `@FR-Col-Group` walk).  Every route that ADDS a record reaches
`record_finish`; a keyed removal and `e#remove` emit the unlinks (loft#900, loft#903).  But
`w.es[0] = E{k:11}` copied INTO the record in place, so the views kept it under the hash of
the OLD key — `by_k[11]` null, `by_k[7]` null, `len(by_k)` still 2; `w.es[0] = null` on a
`vector<E?>` left the view one entry long; `w.es.remove(0)` left the removed key findable and
a re-add of it counted twice.  Silent, both backends, one nesting level down too.  Now one
parser home, `Parser::group_elem_write`, binds the element once, emits
`Parser::group_sibling_unlinks` (the loop the two removal spellings already carried, now
shared), the write, and — for a replace — `OpLinkRecord`, which is `Stores::record_finish`'s
sibling half on its own (`link_record_siblings`).  The temporary is typed as the element PLACE
resolves, deps included: without them the native emitter reads the bind as owning and copies.

The walk that finds the holder is a walk over the READ, so it inherits whatever spelling the
read has.  A group one level inside a `vector<R?>` element (`rooms[0].items.remove(0)`) arrives
as `if <present> { payload } else { nullref }` — `(L-Null)`'s non-slot spelling — which is not a
vector read, so the holder resolved to nothing and the sibling unlinks were never emitted: the
removed record stayed findable under its key, silently, on both backends.  `keyed_field_site`,
`holder_type` and `vector_element_type` peel that read through `use_analysis::through_null_arm`,
which is the one home for *"what does a null-arm read answer?"*.  The DENSE twin was never
broken, which is what says the tag is the axis.

*Anchors:* `Stores::field` (`src/database/types.rs`, the pairing test + `other_indexes`);
`Parser::collection_groups` (`src/parser/objects.rs`, the parser's derivation of the same
question — measured agreeing with `Stores::field` on nine shapes: a forward-declared element,
an alias, a variant, a nullable member, a nullable element, three members, two groups in one
struct, a nullable vector member, two plain vectors);
`Parser::link_shared_nullable_views` (`src/parser/definitions.rs`, the nullable-element
rewrite); `Stores::record_finish` (`src/database/structures.rs`, the per-record sibling
insert) and `Stores::link_record_siblings` (its sibling half alone, `OpLinkRecord`);
`Parser::group_sibling_unlinks` / `Parser::group_elem_write` (`src/parser/collections.rs`, a
record leaving, and the element-level writes through the vector member);
`Stores::insert_keyed_copy` (`src/database/search.rs`, the one keyed insert both the
point write and the bulk fill take); DATABASE.md § Clearing one member of a linked group;
tests/scripts/a-group-element-written-through-the-vector-member-reaches-every-member.loft;
tests/scripts/a-keyed-view-joins-a-nullable-element-vector.loft;
tests/scripts/a-collection-group-does-not-depend-on-declaration-order.loft;
tests/scripts/1158-a-group-forms-whichever-member-is-declared-first.loft;
tests/scripts/1152-a-vector-value-into-a-group-reaches-every-member.loft;
tests/scripts/1159-a-keyed-collection-filled-from-a-vector-value.loft;
tests/scripts/a-nullable-keyed-member-joins-its-group.loft;
tests/scripts/1160-a-variant-binding-write-means-the-field-write.loft;
tests/scripts/927-trie-spatial-linked-group.loft;
tests/scripts/901-linked-group-fill.loft.

---

## 2. Invariants (the both-backends contracts this doc pins)

- **INV-Order** — per-kind iteration order (`Col-Order`) is IDENTICAL on `--interpret` and `--native`.
  The load-bearing one: interp walks a store index, native emits a Rust loop, so a reordering in
  either is a definitional error (the `C-Order` precedent, generalised to every kind incl. spatial Morton).
- **INV-KeyedSlice** — a keyed range slice is a `for`-only iterator, never a value (`D-key-1`); a
  value-position use is rejected identically across `--dump`/`--interpret`/`--native` (driver-agreement).
- **INV-Superset** — a spatial box slice yields a SUPERSET of the geometric box (caller filters). The
  honest contract; both backends return the same superset (same Morton interval), so a divergence in
  membership or order is the error.
- **INV-LookupNull** — a keyed point lookup is `τ?` (absent ⟹ null); enforced by `(N-Store)` like any
  other nullable, both backends.
- **INV-SliceFresh** — a `vector`/`text` value slice is a FRESH, independent value (H-Alloc); mutating
  it never touches the source.

## 3. Deviations / decided edges

**OPEN: 0.**  The record of the closed ones is in
the companion [collections-history.md](collections-history.md).

## 4. Conformance / oracle plan (how each rule gets pinned — [VERIFICATION.md](VERIFICATION.md))

Existing coverage: oracle `16` (keyed copy / hash behaviour). To add, as a `collections.md` block in
VERIFICATION.md (one ☐ row per rule, both-backends + leak + driver-agreement):
- `Col-Order` per kind (esp. spatial Morton order + hash unsorted vs sorted key-order).
- `Slice-Value` clamp + freshness (vector + text).
- `Slice-KeyedIter` value-position REJECT (driver-agreement) + iterate-in-key-order.
- `Slice-Box/Open/Cap` — the superset membership + `:n` cap + open-walk `break` (extend
  tests/scripts/48b-spatial-slice.loft → an oracle program).
- `Col-Lookup` nullable (absent key ⟹ null, discharge required) — pinned by
  `tests/scripts/1120-one-null-question-for-a-collection.loft`, which scores `??`, `== null` and the
  condition position against each other so no two of them can drift apart again.  Its defaults are
  never `[]`: see `D-col-null` for why that is the whole difficulty.

## 5. Open questions / to-verify when writing the rules

1. **Former placement** — collection type formers in types.md (one-line each) vs here (recommend split:
   types.md = the former, collections.md = operations + order).
2. **Exact vector-slice clamp** — hand-verify the partial-OOB and fully-OOB values on both backends
   (plans/25 says partial → in-range part, fully-OOB → `[]`); pin the boundary.
3. **sorted vs index slice** — do they differ observably (index has both tree+hash)? Confirm both expose
   the same `Slice-KeyedIter` iterator; is a `hash` range slice a clean reject (no order)?
4. ~~**`:n` cap semantics** — exact count guarantee for `[(x,y)..:n]` (≤ n? exactly n if available?)~~
   **ANSWERED 2026-08-19 (loft#1002): exactly n when the collection holds n, from any origin.**
   The cap bounds the WALK, and the walk is outward from the query rather than onward from it, so
   the count no longer depends on where the query lands. Pinned in
   `tests/scripts/48b-spatial-slice.loft`, which sweeps the origin along the curve (the axis the
   count-only cells above cannot see) and asserts WHICH records each origin answers, with the
   euclidean distances each expectation follows from. The superset interaction is `Slice-Box`'s
   only — the open forms do not filter, they order.
5. **Both-backends spatial order** — is Morton order proven identical interp-vs-native? (48b runs both +
   leak; confirm it also pins ORDER, not just set membership.)
6. **Scope boundary** — does this doc also state `for x in c` (iteration.md already owns `I-For`; here just
   the per-kind ORDER as `Col-Order`, cross-linking rather than restating)?

## 6. See also

- [types.md](types.md) — the type formers + `τ?` (lookup nullability, value-slice element type).
- [iteration.md](iteration.md) — `I-For` cursor; `Col-Order` fixes the per-kind order it iterates.
- [concurrency.md](concurrency.md) — `C-Order`, the hash edge this generalises.
- [heap.md](heap.md) — store steps (`H-Alloc` for fresh slices, `H-Copy` keyed), [layout.md](layout.md) — byte layout.
- [matching.md § PEG patterns](matching.md) — @PLN35 reuses the slice / `⟨i, src⟩` cursor surface.
- Code: `src/parser/fields.rs` (the shared index/slice dispatch, `parse_spatial_slice`, `D-key-1`);
  DATABASE.md / STDLIB.md (the user-facing surface); [../plans/38-sorted-slice/](../plans/38-sorted-slice/).
