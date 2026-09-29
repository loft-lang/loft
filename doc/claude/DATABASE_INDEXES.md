# Database — tree, hash, spatial index and text trie

The keyed and indexed collection layers: red-black tree, open-addressing hash, spatial (radix) index, text trie.  Part of [DATABASE.md](DATABASE.md), which holds the overview, the raw `Store` heap, `DbRef` and vectors.

## Red-Black Tree (`src/tree.rs`)

Used for `sorted<T>` and `index<T>` collections that need O(log n) insert/delete/find with O(1) iteration via backward links.

### Node Layout

Each node is a record in a `Store`. The tree-management fields are stored at a fixed offset (`fields`) within the record, after any user data fields:

```
offset fields+0: LEFT  (i32) — positive = left child rec, negative = backward link to parent
offset fields+4: RIGHT (i32) — positive = right child rec, negative = backward link to parent
offset fields+8: FLAG  (i32) — 1 = red, 0 = black
```

User data fields occupy bytes 0 .. `fields-1`.

### Backward Links

Negative values in LEFT/RIGHT are backward links to the parent node (stored as the negated rec value). This enables O(1) `next` and `previous` without a stack or parent pointer field:

- From any node, follow backward links up until you come from a left child → that ancestor is `next`.
- `previous` is symmetric (came from a right child).
- This is the key structural invariant: the tree simultaneously encodes the parent relationship for traversal without extra memory.

### Limits

```rust
const RB_MAX_DEPTH: usize = 30;
```

Maximum tree depth of 30 is sufficient for up to ~2^15 nodes in a balanced red-black tree.

### Key Functions

| Function | Description |
|---|---|
| `find(data, before, fields, stores, keys, key) -> u32` | The BOUNDARY below / above `key`: never stops at an equal key, so it serves a range's ends and a partial key |
| `find_exact(data, fields, stores, keys, key) -> u32` | The point lookup for a FULL key: the descent stops at the equal node (a full key matches at most one record — an insert displaces its duplicate) |
| `add(data, rec, fields, stores, keys) -> u32` | Insert `rec` and rebalance; answers 0, or the record that already carries `rec`'s key with the tree UNCHANGED — so the caller displaces it and adds again, and no insert looks its key up first |
| `remove(store, root, rec, keys) -> u32` | Delete `rec`; rebalances; returns new root |
| `first(store, root) -> u32` | Leftmost node (minimum key) |
| `last(store, root) -> u32` | Rightmost node (maximum key) |
| `next(store, rec) -> u32` | In-order successor via backward links; 0 if none |
| `previous(store, rec) -> u32` | In-order predecessor via backward links; 0 if none |
| `validate(store, root, keys)` | Debug: verify RB invariants and backward-link consistency |

### Rebalancing

Standard left-leaning red-black tree rotations and color-flips. `add` performs a top-down split on the way down then a bottom-up fixup on the way back up. `remove` uses the standard delete-and-recolor approach, delegating to a helper for the six deletion cases.

---

## Open-Addressing Hash Table (`src/hash.rs`)

Used for `hash<T>` and the hash component of `index<T>`.

### Record Layout

The bucket table is a single record in a `Store`:

```
byte  0: room    (u32) — the record's size header, doubling as the word count
byte  4: LEN_FLD    (u32) — live-entry count
byte  8: SEED_FLD   (u64) — the per-hash seed, stored WITH the buckets so any
                            reader re-derives identical buckets
byte 16: DIR_FLD    (u32) — the entry arena's chunk directory (`src/arena.rs`)
byte 20: NEXT_FLD   (u32) — the arena's append cursor
byte 24: FREE_FLD   (u32) — head of the arena's free list
byte 28: STRIDE_FLD (u32) — bytes per entry slot, 0 when the table BORROWS
byte 32: BUCKET0    — slots, 4 bytes each (0 = empty)
```

`elms = (room - RESERVED_WORDS) * 2`, with `RESERVED_WORDS = 4`.

### An insert is one hash and one probe walk

`Stores::insert_record` asks `hash::probe_for_insert` two things at once: is the record's
key already here (@FR-Col-Insert — the latest insert displaces it), and if not, which
bucket takes it.  Both answers lie on the walk from the key's home bucket to the first
empty one, so the insert hashes once and files the entry with `hash::add_at`.  It used to
look the duplicate up first (`dedup_keyed`: the key copied into a `Vec<Content>`, hashed,
probed) and then run `hash::add`, which hashed the key again and walked again.  A found
duplicate, a table not yet created and a record that is already filed all take that
two-walk form still — it owns those answers — and `LOFT_NO_ONE_PROBE_INSERT=1` restores it
for every insert.  Measured on 5,000 integer keys: a fill −37 %
(`bench/portal/analysis/keyed.md`).

### What a walk over the buckets holds, and what it does not

* **`Entries`** — the loop-invariant half of decoding a bucket slot (the arena's directory,
  its capacity, the stride), read once per walk by `find`, `probe_for_insert`,
  `rehash_into` and `remove`.  Valid while nothing appends a chunk, which none of them does.
* **`probe`** — the walk for a pre-resolved key, compiled once per key KIND
  (`find_fast`, `find_long`): inside one arm the comparison is a read and a compare.
  `FastKey::matches` / `order` are `#[inline(always)]` because as hints they were declined,
  and an out-of-line comparator re-dispatched on the kind per bucket.
* **`home_bucket`** — the ONE place a digest becomes a bucket number, `digest % count`.
  It is a 64-bit division on every lookup's critical path, priced at 7–10 % of a
  cache-resident lookup, and it stays: a multiply-shift reduction would move every entry
  of every stored hash, and an exact reciprocal needs a per-table number the header has no
  word for.  Both are a format break (`crate::placement::HASH`).
* **A removal** recognises its entry by DECODING the slots on its chain (`Entries::names`)
  and answers the bucket value that named it, which `free_slot` releases — so it never
  maps a record back to an arena index (`arena::index_of`, a directory scan it used to
  make twice).  Its back-shift computes round-the-table distances with a compare and an
  add where it had three `%` per entry walked, and `Stores::remove` reads a `Copy` kind off
  the type row instead of cloning the row and the key descriptors (three allocations per
  removal, a quarter of its instructions).

### Entries live in a chunked arena, not one record each (@PLN135 arc H)

A bucket slot holds a **1-based arena index**, not a record number. Entries sit packed at
a fixed stride inside chunk records, so a `hash` costs 18.6 bytes an entry where a record
each cost 27.67, and 2000 entries claim 9 store records (table + directory + 6 chunks)
instead of 2000. Filling one is ~1.28x faster; **lookups are unchanged** — see
[@PLN135 § What H actually bought](plans/135-hash-performance/README.md), which records why
the locality win this was designed for does not exist.

The arena grows by APPENDING a chunk and never reallocates one that already holds slots,
so a `DbRef` a caller is holding stays valid for the entry's whole life — the stability a
per-entry record gave for free. Chunk sizes double only to `arena::CAP_CHUNK` and are
fixed after that, which bounds the tail waste at one partly-filled chunk instead of half
the collection.

**Two kinds of entry.** A hash allocates its entries from the arena. A SECONDARY index — a
sibling field's `other_indexes` — is a second route to records the primary owns, and may
neither move nor free them; its slots hold those record numbers, exactly as every slot did
before. `STRIDE_FLD == 0` marks the borrowed case, and `hash::owns_entries` is the one
place that asks. Freeing follows: an owned entry's storage returns to the arena
(`hash::free_entry`), a borrowed one is left to its owner.

A hash **in a linked group takes no arena at all** — not even as the group's primary
(loft#901). Every member of a group names its elements by a 4-byte record id: a hash slot
encodes `rec.rec`, an `array`/`ordered` slot stores it raw and reads it back at a
hard-coded payload start, and an `index` keeps its links in fields of the record. None can
express a position INSIDE a record, so a packed entry — several to a chunk, distinguished
only by offset — is unaddressable through the siblings: they saw two elements at one
record id and kept the first. `Type::linked` is the flag, `record_new` reads it, and
`Stores::finish` sets it for every element type of a group (a field with a non-empty
`other_indexes`). A collection that is not in a group is unaffected and keeps the arena.

The layout is pinned by `tests/layout_golden.rs::placement_contract_is_pinned`; changing
any of it without bumping `placement::HASH` would let an older store be misread instead of
refused ([`src/placement.rs`](../../src/placement.rs)).

### Clearing one member of a linked group (loft#898)

Two or more collections over one element type in one struct are auto-linked into
several routes to a SINGLE record set (`Field.other_indexes`, loft#843) — filling either
fills both. `trie` and `spatial` join on the same terms as the rest: they were missing
from the test that FORMS a group, which did not refuse the pairing but silently built a
second, independent collection (loft#927).

**A group needs at least one KEYED member, and nothing else about how it is written
matters** — including whether the fields sit in a `struct` or in a struct-enum VARIANT, which
holds fields on the same terms. Two plain vectors over one element type stay independent —
inserting into one must not propagate to the other — but a plain `vector<E>` beside any keyed
collection over `E` is a member like any other, in EITHER declaration order, and whether the
element is dense (`vector<E>`) or nullable (`vector<E?>`). Each of these was once a hole that
did not refuse the pairing but built a second, silent collection:

* **declaration order** — the pairing test asked only whether the field being ADDED was
  keyed, so `{ look: sorted<E[k]>, data: vector<E> }` formed no group while
  `{ data: vector<E>, look: sorted<E[k]> }` did. It now asks it of the PAIR.
* **a nullable element** — `vector<E?>` stores the synth `__nullable<E>` enum, so a view
  still declared over dense `E` no longer matched by content. The view's element is
  rewritten to the sibling's enum for every keyed kind (`link_shared_nullable_views`);
  only `hash` used to be.
* **a struct-enum VARIANT** — the nullable rewrite above ran from the struct parse only, so
  every keyed kind in a variant stayed dense beside its `vector<S?>` sibling. The DENSE half
  was always right, because group formation itself lives in `Stores::field`, which handles a
  variant like a struct.
* **a vector VALUE** — `data = rows()` and `data += rows()` move records in bulk through
  `vector_add` / `vector_replace`, which never reach `record_finish`, the per-record
  chokepoint that maintains the other members. Closed (loft#1152, loft#1159): the parser
  emits the re-index per view beside the write — § Filling one member with a whole VECTOR
  VALUE below. A member written through an `is` / `match` BINDING is the same shape and is
  recorded there too.
* **an element-level write through the vector member** — `v[i] = e`, `v[i] = null`,
  `v.remove(i)` reached no chokepoint at all, so the keyed views kept the record under its
  OLD key. Closed 2026-09-05 — § Replacing, nulling or removing one element through the
  vector member below.

#### Two collections over one element type that must stay APART

A group is formed from the element TYPE, so the way out is a different element type — and the
one spelling that looks like a different type and is not is a type ALIAS:

| written this way | result |
|---|---|
| `struct Lvl { by_key: hash<Tile[k]>, picked: vector<Chosen> }` — a second STRUCT, fields identical | **independent** |
| the two collections in **different structs** | **independent** |
| both as **locals**, not fields — a group is a FIELD rule | **independent** |
| `type Chosen = Tile;` then `vector<Chosen>` | ⚠ **one group** — an alias names the same type |

The newtype is the escape, and its cost is the conversion, which is a plain field copy:

```loft
struct Tile   { k: integer, n: text }
struct Chosen { k: integer, n: text }
fn to_chosen(t: Tile) -> Chosen { Chosen { k: t.k, n: t.n } }

struct Lvl { by_key: hash<Tile[k]>, picked: vector<Chosen> }
```

Two collections over one element type in one struct are a group with no way to decline it in
place — that is the deliberate trade behind auto-linking (loft#843): the pairing is what makes
the keyed view stay in step with its list without any code to keep them in step, and a
per-field opt-out would be a second way to spell the same declaration. If you want the two
apart, say so in the TYPES, where the reader can see it.

Every combination of kinds is a valid group
except **two `index` members with the same key**, which is refused where it is declared: an index keeps its tree links in a
field of the element record, so a second one has nowhere to put them (loft#902, and
[DESIGN_DECISIONS.md § C113](DESIGN_DECISIONS.md) for why it is refused rather than
given its own storage). **The records belong to exactly one member.** `types.rs` decides which when it
builds the group: the first-declared member is the PRIMARY, and every later one gets a
leading `u16::MAX` on its `other_indexes` marking it a VIEW. That marker is the only place
the ownership fact lives, and three readers now share it — the JSON default-init, the
struct teardown walk, and the clear.

Because the group is auto-formed, a struct literal that gives RECORDS to two members
reads as two collections and behaves as one. That is documented behaviour, so it is not
an error — but it is almost never what the author meant, and the `linked-group-double-fill`
advice names it at the literal ([DIAGNOSTICS.md](DIAGNOSTICS.md), `LOFT_NO_LINKED_GROUP`
opts out). It stays quiet on `field: []`, which is how every group is constructed, and on
a literal that fills one member — the two deliberate shapes.

Each member therefore releases only what it owns:

| Member | What it contributes to a clear |
|---|---|
| a VIEW | its own SPINE — the hash table record, the `Ordered` slot list, or (for `index`) nothing at all, since a b-tree's nodes ARE the element records and zeroing the root is the whole teardown. Never a record. |
| the PRIMARY | the records, once. |

**A clear spelled through ANY member empties the group**: every view's spine is reset and
the primary is cleared, so the members never disagree. That is not a choice the clear
makes — an operation spelled through a view already acts on the group, since `h.view +=
[e]` appends to every member (loft#843). Letting a view be emptied alone cannot be made
coherent for a NON-EMPTY literal: the elements still enter the group, so `h.view = [e]`
would leave the view holding `e` while the primary holds `e` plus everything it had, and
nothing repairs an index that silently does not index its records.

Both directions were broken and only one was filed: every member freed the shared
records, so whichever was cleared first took the other's elements down with it (a key
reading `4294967296`, a text reading `null`), and clearing the primary left the views
naming freed records. The plumbing is a `0x8000` bit on
`OpClearKeyed`'s `tp` operand — the same convention `OpSetKeyed`/`OpReplaceKeyed` already
use — set by the parser, which is the only layer that can ask the schema. Both backends
decode it in ONE place, `Stores::remove_claims_keyed`, so they cannot drift.
`Stores::keyed_group_members` is the single schema query behind all of it.

The clear is emitted by the KEYED assign and by the VECTOR assign, because the shape
DATABASE.md documents by name — `vector<T>` + `hash<T[k]>` — has the vector as its record
holder. Both route through `Parser::keyed_sibling_view_resets`.

### Filling one member with a whole VECTOR VALUE (loft#1152)

`Stores::record_finish` is the chokepoint that maintains a group — it walks the field's
`other_indexes` and inserts the record into every sibling — and every route that adds
records ONE AT A TIME reaches it. A whole-vector write does not: `OpAppendVector` reaches
`Stores::vector_add` → `vector_add_array`, which moves the records in bulk. So `s.v =
rows()` and `s.v += rows()` filled the vector and left every sibling view EMPTY, on both
backends, with `len` answering `0` and a lookup answering `null` — both legal values for a
group that happens to be empty, which is the state this page says has no repair.

The maintenance is emitted per VIEW member as `OpIndexGroup(primary, view, tp)`, beside the
resets the clear already emits, through `Parser::keyed_sibling_view_fills`. **A runtime fix
was not available**: `record_finish` can maintain a group because it is handed `(data, rec,
parent_tp, field)`, while `vector_add_array` has only the vector field's `DbRef` and the
element type, and `OpAppendVector` carries neither the parent type nor the field index —
recovering them from the `DbRef` is not a route, since `db.pos` is a byte offset into a
record whose type would be a guess. The call site is the right home anyway because the
unit of work differs on the two halves: the MEMBERS are known at emit time, so the parser
names them exactly as the clear does, while the per-RECORD loop lives inside the op.

The records are **not copied**. The view is handed the primary's own element records by id,
exactly as `record_finish` hands them over, which is what keeps a write through the vector
visible through the view; a null element stays in the vector and out of the index, which is
the same rule `record_finish` applies. The reset is emitted only where the statement does
not already carry one — a `=` reset its views via `clear_vector_field`, a `+=` had none —
because the re-index walks the whole primary and a view still holding the previous records
would be handed them twice.

An enum VARIANT's fields are a group on the same terms, and needed
`Parser::field_site` extending: a variant's fields live in the variant's own
`Parts::EnumValue`, so the enum's type id names no field and every group question about a
variant field answered *"no group"* — including the clear's. The variant is read back out
of the discriminant in the guard the field read is already wrapped in.

⚠ **Writing through an `is` / `match` BINDING is not this.** `if h is F { a, b } { a = rows()
}` leaves `h.a` empty: the binding COPIES (`@FR-B-Copy`), so the write never reaches the
record and the sibling is right to be empty. Reading `len(a)` after it shows `2` and looks
like a landed write; reading `h.a` back is what settles it.

Group FORMATION no longer depends on declaration order: the pairing test asks the question
of the PAIR (loft#1158), so `sorted` then `vector` links exactly as `vector` then `sorted`
does — `1158-a-group-forms-whichever-member-is-declared-first.loft` pins all five keyed kinds
in both orders.

### Replacing, nulling or removing one element through the vector member (2026-09-05)

The rule's second clause — *a record LEAVING through any member leaves every member* — had
three routes outside it, all through the VECTOR member and all silent on both backends:

| write | what the views held afterwards |
|---|---|
| `w.es[0] = E { k: 11 }` | the same record, under the hash of the OLD key: `by_k[11]` null, `by_k[7]` null, `len(by_k)` still 2 |
| `w.es[0] = null` on a `vector<E?>` | the nulled record: `len(by_k)` one too long |
| `w.es.remove(0)` | the removed record: its key still findable, and a re-add of that key counted twice |

An index write copies INTO the element record in place (`OpCopyRecord`), a null write clears
its payload, and `remove` unlinks the vector's slot — none of them a route through
`record_finish`, which only ever ADDS. `Parser::group_elem_write` (`src/parser/collections.rs`)
now wraps each of them: the element is bound ONCE to a temporary (its index evaluated once,
`hoist_index_arg`), every keyed sibling unlinks it (`Parser::group_sibling_unlinks` — the loop
`coll[key] = null` and `e#remove` already carried, now the one home), the write runs against
the temporary, and a replace ends with `OpLinkRecord`, which is `record_finish`'s sibling
half on its own (`Stores::link_record_siblings`): the primary already holds the record, and
`record_finish` would append it a second time. `v.remove(i)` now answers the boolean its op
always had. The temporary is typed as the element PLACE resolves, deps included — typed
without them, the native emitter reads `found = v[i]` as an owning bind and deep-copies the
record, and the unlinks then run on the copy. Nested shapes (`w.rooms[0].items[0] = …`, and
under a `vector<R?>`) resolve through `holder_type`, which reads the field type an
`OpGetField` carries as its third operand rather than re-walking the schema.

**Which vector holds, when a group has TWO plain vectors** (`{ a: vector<E>, b: vector<E>,
h: hash<E[k]> }`) is open — loft#1375: each vector links to the hash and never to the other,
so `h` holds the union and each vector only its own entries.

Guard: `a-group-element-written-through-the-vector-member-reaches-every-member.loft` (19
rows: replace by literal / by local / by a sibling element, null write, a record into a null
slot, `remove` at the first and last index and out of range, in a loop, through a parameter,
one nesting level down and under a `vector<R?>`, a variant holder, four keyed members
re-linked together, the same key replaced, an index expression evaluated once, an
out-of-range index, and the ungrouped and `e#remove` controls).

### Removing one entry of a linked group (loft#900)

Removal follows the clear's verdict: **a removal spelled through any member removes the
entry from the group**, and its record is freed exactly once. `h.by_k[1] = null` therefore
takes the entry out of the vector too. The alternative — dropping one index entry and
leaving the record in the primary — has no coherent successor state: `h.by_k[1] = null`
followed by `h.by_k[1] = E{k:1,…}` would remove one entry and then add to the whole group,
leaving the primary holding two records under one key with nothing able to repair it.

The ORDER is the mechanism, and it is what the plumbing is shaped around. Every unlink
reads the record's key out of the record, so the free must come LAST and the record must
stay reachable until then:

1. the key lookup runs ONCE, into a parser work-ref temporary marked `inline_ref` (the
   record belongs to the collection, not to the temporary, so nothing frees it twice);
2. one `OpHashRemove` per OTHER member, each carrying the `CLEAR_KEYED_VIEW` bit on its
   `tp` — the same `0x8000` convention `OpClearKeyed` and `OpSetKeyed` use, so the op's
   arity and both emitters are unchanged. The bit means UNLINK ONLY;
3. the ordinary removal on the member the source named, which unlinks and frees.

Resolving the lookup once is also what keeps the key expression evaluated once (@PLN102
F2). `Parser::keyed_group_remove` emits the sequence and `Parser::keyed_field_site` finds
the struct field by walking the `OpGetField` chain, so a group one level down resolves too.

Both directions were broken and only one was filed: through a VIEW the record was freed
while the primary still held it (the vector kept the entry and its key, the text read back
`null`), and through the PRIMARY the views were never told. Two supporting facts had to be
repaired with it — `Stores::remove`'s `Array` arm computed its slot by BY-VALUE arithmetic
and so unlinked slot 0 every time (the loft#719 defect, fixed then for `Ordered` only), and
`remove_owned` sent a grouped hash to `hash::free_entry`, which declines to free a record a
stride-0 table only borrows, so the record leaked. `Stores::hash_owns_entries` is the
table's own answer to which case that is.

### Removing one entry with `e#remove` (loft#903)

`#remove` reaches an element by POSITION rather than by key, and that half had no owner:
the cursor form kept its own arithmetic instead of `remove_owned`'s. It removed TWO
elements of an `array<T>` (`OpRemove` was handed the ELEMENT's width where a
record-backed container's slots are four bytes) and freed neither the record a slot
names nor what that record owned; inside a group it maintained no other member; a
`rev()` loop rewound the cursor the wrong way over a plain `vector` (which never put the
reverse bit in `on`) and one slot too far over an inline `sorted`; and over an `ordered`
the interpreter removed while `--native` removed nothing, having no arm for it.

The layout question now lives in ONE place. `Stores::remove_vector_at` reads the element
type's `linked` flag and answers both halves of it — a slot is four bytes and names a
record to free when the type is linked, and is an inline element otherwise — so the two
spellings that remove by index, `e#remove` and `v.remove(i)`, cannot disagree.
`OpRemoveVector`'s operand is the element TYPE for the same reason: a width cannot say
what an element owns. It is the by-INDEX twin of `remove_owned`, which stays the
by-RECORD form a key lookup reaches.

The group half is loft#900's sequence with one difference. A key lookup can be hoisted
into a temporary; a loop cursor cannot, and it does not have to be — the LOOP VARIABLE
already is the element's reference, resolved once per iteration and at the record's
payload start for every kind a group can hold (`index` yields `new_ref(.., 8)`, `ordered`
and a linked `array` yield the record a slot names). `Parser::loop_group_remove` emits
one `CLEAR_KEYED_VIEW` unlink per other member from it, then the spelled member's
`OpRemove` frees.

**Two `index` members are refused (loft#902).** An `index` keeps its red-black links in
FIELDS of the element record, and that `#left_N / #right_N / #color_N` triple is
allocated per index TYPE — so two fields whose declared type is identical name one set of
links: not two trees, but ONE tree reached through two roots. The fill therefore looked
right (both roots walked the same structure) and the first removal rebalanced through one
root, left the other stale, and panicked in `tree.rs` on the next walk. There is nothing
to make work — a second index with the SAME key answers exactly what the first answers,
in the same order — so `Parser::reject_duplicate_index` refuses it where the field is
declared, naming the workaround: give the second route a different KIND, or a different
key. A different key is a different type name and so its own link triple, and two
`index<E[k]>` fields in different structs hold different records; both stay legal.

### Constructing a group with a literal (loft#924)

**A group's members are all zeroed together, before any of them is filled.** A collection
field is a 4-byte header, and `parse_object` writes the group's headers as ONE block in
the literal's prelude — the treatment #437 already gives plain vector fields, and for the
same reason. `Parser::linked_group_offsets` names them; the two sites that otherwise
prime one field at a time (the field parse, and `object_init` for a field the literal
leaves out) skip whatever the prelude covered.

Per-field priming cannot work here, and the reason is the group's own rule. `OpFinishRecord`
through the member the author names indexes the record into every sibling, so a member
whose header is written AFTER that insert drops the spine it was just handed. The result
was decided by the ORDER the fields were written in: `S { data: […], lookup: [] }` left
`lookup` empty while `data` held the records, `S { lookup: [], data: […] }` did not, and
an OMITTED member was zeroed later still — by the default-init that runs once the body is
read, so it lost them too. Every keyed lookup then answered null for a record that was
there, which is indistinguishable from a key that was never inserted.

The corollary is worth stating because it surprises: **a literal that names TWO members
adds to the group twice.** `HS { by_k: [a], by_v: [b] }` puts two records in one set and
both members see both — the same thing `h.by_k += [a]; h.by_v += [b]` has always done.
Three in-tree fixtures had been reading the truncation as independence (`502`, `922` and
`85`), which is a fair signal of how easily two keyed fields over one element type are
written without meaning a group; there is no diagnostic for it yet (loft#926).

### Probing and Load Factor

Collision resolution is **linear probing**: on collision, advance slot index by 1 (wrapping). The load factor threshold is (`hash::is_full`):

```rust
length + RESERVED_WORDS >= room
```

which is `length >= elms / 2` — a table is rebuilt at HALF full.  It was three quarters
(`length * 2 / 3` in the same test) until 2026-09-21.  A probe here is a dependent read of
the entry, with no fingerprint to reject on, and a miss walks to the first empty bucket:
8.5 buckets at 0.75 against 2.5 at 0.5 — a table just under the old threshold measured
2.30 key compares a hit and 5.36 a miss.  The price is the bucket array, 4 bytes a slot:
~10.7 bytes an entry averaged over a doubling instead of ~7.1.  It is a WRITER's policy:
no reader assumes a load, so stores written under either rule read under both, and
`LOFT_NO_HALF_LOAD=1` restores the old one.  When it is met after an insertion, the table is rehashed
into a new record with doubled capacity, and the arena's four fields travel with it —
leaving them behind would strand every chunk and hand out index 1 again on top of a live
entry. `reserve(h, n)` sizes the table so this never fires while filling to `n`; it claims
one word PAST the trigger, because a table sized to exactly the trigger grows on the last
insert and the reservation buys nothing.

### Hash Function

`hash` from `src/keys.rs` is used to compute a 64-bit hash from the record's key fields. The slot index is `hash % slot_count`.

### Key Functions

| Function | Description |
|---|---|
| `add(store, hash_rec, ref_store, elem_rec, keys) -> u32` | Insert element; triggers rehash if over load factor; returns (possibly new) hash_rec |
| `find(store, hash_rec, ref_store, keys, vals) -> u32` | Lookup by key values; returns rec or 0 |
| `remove(store, hash_rec, ref_store, elem_rec, keys) -> u32` | Delete element; returns (possibly compacted) hash_rec |
| `validate(store, hash_rec, ref_store, keys)` | Debug: verify all slots are reachable from their hash position |

### Deletion

Deletion uses **backward shift**: after zeroing the removed slot, scan forward and shift back any element whose probe distance to the now-vacant slot is shorter than its probe distance to its current slot. This maintains the invariant that every element is reachable from its home slot by linear probing without encountering an empty slot.

The probe distance formula used:
```rust
d = (slot - ideal + elms) % elms
```
An element at `idx` with ideal slot `ideal` moves to `hole` when `d_hole < d_idx`. The slot containing the element to remove is found by scanning from `hash(rec) % elms` forward until a slot equals `rec.rec`.

**Null-rec guard**: `remove()` returns immediately if `rec.rec == 0` (element not found). Callers can safely call remove with a lookup result without checking first.

### `database::remove()` for Index

`database::remove()` routes to `tree::remove()` for `Parts::Index`. The `fields` argument passed to `tree::remove` must be the **byte offset** of the tree node pointers within the record (= `8 + struct_field[left_field_index].position`), not the raw field index. This is computed via `self.fields(db)` (same helper used by `tree::add`).

---

## Spatial Index (`src/radix_tree.rs`)

`spatial<T[x,y]>` / `spatial<T[x,y,z]>` (@PLN48) is a fully implemented keyed
collection on both backends (interpreter + `--native`). The `Radix(u16,
Vec<u16>)` variant of `Parts` is the schema-level marker — content type nr
plus the coordinate key field indices; the runtime `Type::Radix(content,
coord_fields, deps)` (`src/data.rs`) mirrors it. This was renamed from
`Spatial` to `Radix` (storage-honest — the language keyword stays `spatial`).

The backing structure is a **store-backed binary PATRICIA/radix tree**
(`src/radix_tree.rs`) over an abstract bit-key oracle. `src/radix_db.rs` is
the DB↔tree bridge: it interleaves the coordinate axes into a **Morton /
Z-order** key and implements `add`/`find`/`remove`/`count`/`records`/`range`.
`src/spatial.rs` holds the underlying near/within/nearest geometry algorithms
`radix_db.rs` builds on.

**Dimensionality: 1 to 3 coordinate axes** (`MAX_AXES = 3` in
`src/radix_db.rs`). The parser rejects a `spatial<T[a,b,c,d]>` with more than
3 axes with a diagnostic (*"spatial<T[…] > supports at most 3 coordinate
axes, got N"*); a bare `spatial<T>` with no key fields is also rejected
(*"needs coordinate key fields"*). See `tests/parse_errors.rs::spatial_needs_coordinate_keys`
and `::spatial_rejects_more_than_three_axes`.

Supported operations, all working on both backends:
- **Construct**: `xs: spatial<Mob[x, y]> = [];`, including as a struct field.
- **Append**: `xs += [Mob{x: 1, y: 2}];`.
- **Iterate**: `for m in xs { … }` — yields records in the tree's natural
  Morton/Z-order (no sort, unlike `hash`).
- **Length**: `xs.len()` — O(1), reads the tree's cached length word.
- **Range slices** — the language surface for proximity queries (no
  `.near`/`.within`/`.nearest` methods; spatial reuses ordinary slicing):
  `xs[(x,y)..]` (outward walk from the point, caller `break`s),
  `xs[(x,y)..:n]` (capped at `n`), and `xs[(x1,y1)..(x2,y2)]` (bounding-box).
  Slices carry up to 3 axes.
  The two OPEN forms walk OUTWARD from the query — two cursors seeded either side
  of it, each step yielding whichever is closer (`radix_db::near_range`, the n-axis
  form of `spatial::near`) — so `..:n` answers `n` records from any origin and a
  query past every record still answers its neighbours. They used to be the raw
  Morton TAIL, where a record one code behind the query never appeared however
  close it was and `..:n` silently under-delivered near the end of the curve
  (measured 3, 3, 3, 2, 1, 0 over five records as the query moved along; a query
  past every record answered nothing at all — loft#1002).
  The walk is APPROXIMATE, ordered by Morton distance: it tracks spatial distance
  closely but jumps at quadrant boundaries, so a truly-near point can arrive a
  little late. Every record is yielded eventually, each once. The BOX form
  is the geometric box exactly: it walks only the box, seeking over the runs
  Z-order threads outside it (@PLN136, `radix_db::box_walk`), where it used to
  read the whole code interval between the corners and filter afterwards
  (loft#800). A corner-swapped axis names the same box.

- **Point subscript** — `xs[x, y]` reads the record at exactly that point
  (`null` when empty), `xs[x, y] = mob` inserts-or-replaces, `xs[x, y] = null`
  removes. The coordinates are separate subscripts here, where the range forms
  above parenthesise them. All three were broken until loft#720; see the
  warning below for why that went unnoticed.

See [INTERNALS.md](INTERNALS.md) for the full radix-tree API and record
layout, and [plans/48-spacial-index/README.md](plans/48-spacial-index/README.md)
for the design history.

---

## Text Trie (`src/trie_db.rs`)

`trie<T[k]>` keys on ONE **text** field. It shares `spatial`'s PATRICIA tree
(`src/radix_tree.rs`) and nothing above it: `src/trie_db.rs` is the DB↔tree
bridge with a byte-key oracle where `radix_db.rs` has a Morton one, and the two
operation sets diverge from there — a bounding box means nothing for a word, and
a prefix means nothing for a coordinate.

**`spatial` is not called `radix` on purpose**, which is why this is a separate
`Parts` kind rather than `Radix` with a second oracle. Sharing the storage
structure is not sharing the kind; the rename to `Parts::Radix` was
storage-honesty about the tree. Design and its falsified first draft:
[plans/text-keyed-trie.md](plans/text-keyed-trie.md).

Supported operations, both backends:
- **Construct**: `t: trie<Word[w]> = [];`, including as a struct field.
- **Append**: `t += [Word{w: "kerk"}];`.
- **Iterate**: `for x in t { … }` — key order (byte order), no sort. The
  terminator sorts before any byte, so `kerk` precedes `kerkstraat` precedes
  `kerkweg`.
- **Exact lookup**: `t["kerk"]` — the record, or `null`. Never a neighbour.
- **Length**: `t.len()` — O(1), the tree's cached length word.
- **Prefix slice**: `t["kerk"..]` / `t["kerk"..:n]` — every key BEGINNING with
  the prefix, in key order, capped at `n`. This is the capability that earns the
  kind its place: a `sorted` range needs a successor string the caller must
  construct, and answers a key interval rather than a prefix. `t[a..b]` is
  refused and names `sorted` as the kind that answers an interval.

Exactly one key field, refused at the keyword: a trie orders one key's bytes, so
several keys have no order to share.

**Persistence is WHOLE-IMAGE.** `store_persist_bind` / `store_load` /
`store_load_url_trusted` carry a trie with its counts and key order intact. The
PAGED readers do not: `store_load_key(_text)` and a lazily-bound `.store` image
read a `hash`, and `store_lazy_range` reads a `sorted` / `index`. So a trie is
downloaded whole or not at all — for the `routing` name index that is 220 032
words, 23.4 MB raw and 5.9 MB gzipped, reloaded in 42 ms. That is a size cut, not
a per-query read, and the two compose rather than compete: keep the vocabulary
whole and page the postings behind it.

`store_bind_lazy` accepts a `hash`, a `trie` (@PLN134) and a `spatial` (@PLN136)
bound to an image, and REFUSES a `sorted` / `index`, answering `false` — that kind
cannot be paged, it is knowable with no I/O, and the alternative is `null` at
every lookup forever (loft#802).

The gate is `tests/scripts/801-trie-text-keyed.loft` — hand-computed values on
both backends with a `sorted` control alongside;
`tests/scripts/802-lazy-refusal-visible.loft` is the refusal's.

### The node array is laid out for paging when an image is written (@PLN134)

A PATRICIA descent is cheap in NODES — one root→leaf path, branching on bits of a
probe the caller already holds — and **that says nothing about what it costs over
a link**. A reader fetches 64 KB pages, and node ids are handed out in INSERTION
order, so a path visits nodes created at wildly different times. Measured over
978 842 real words (`trie_db::pages`, `#[ignore]`):

| node order | pages per prefix query, 64 KB | at 4 KB |
|---|---|---|
| as built (insertion) | 27.1 | 36.4 |
| breadth-first | 15.4 | 26.0 |
| key order (in-order) | 8.7 | 14.5 |
| depth-first pre-order | 4.2 | 7.2 |
| **van Emde Boas** | **2.8** | **3.8** |

To read ~330 bytes of nodes. The 4 KB column is what identifies the mechanism
rather than the number: vEB barely moves where every other order inflates by
half, which is the cache-oblivious property doing what it is for — and it matters
beyond elegance, because the page size is not ours to pick (a local file, an HTTP
range read and a browser cache disagree, and one layout is near-optimal for all).

So `store_persist_bind` runs `Stores::relayout_trees` before it writes the image
— `radix_tree::rtree_relayout` renumbers each tree van Emde Boas and compacts the
free list. **Node ids are internal**, so nothing observable moves: same records,
same key order, same answer to every lookup, which is what `r11` holds it to. It
is idempotent (the layout is a function of the tree, not of the current ids) and
it REFUSES a tree whose walk does not account for `n-1` nodes over `n` records,
leaving it exactly as it was. Stores whose SCHEMA holds no trie skip the data
walk entirely (`type_has_tree`), so the cost falls only on the kinds that have one.

The other half is where the RECORDS land, and it is the larger one: a query also
reads what it returns, and 20 records claimed in insertion order sit on ~20
distinct pages — one fetch per row. Written in trie key order they occupy **1**.
A deep copy already claims them in key order (`copy_claims_trie_body` walks the
tree), so a rebuilt store has this; a store persisted as built does not.

**`store_persist_copy(r, path)` is where a rebuilt image comes from**, and it is
a separate call rather than a fix to `store_persist_bind` because of a contract.
Binding documents *"Caller's existing DbRefs into that slot remain valid"*, and a
record number IS its word offset — so the guarantee and the placement are the
same fact, and reordering is exactly what it forbids (@PLN123 B2 records the same
constraint at the compaction call site: the fresh branch is a WRITE, where a
program's interior references are live). So the copy is rebuilt into a scratch
store nobody holds a reference into, `relayout_trees` runs on THAT, and the live
collection keeps every number it handed out. Measured on 74,692 real words, one
20-record prefix query, bytes off the wire:

| | requests | fetched |
|---|---|---|
| bound image, as built | 19.9 | 1.28 MB |
| `store_persist_copy` image | **4.9** | **0.32 MB** |
| whole-image download | 1 | 5.17 MB |

The file is not bound, so writes after it do not reach it — it is the artefact
you ship, written when the data is final. `store_persist_bind` remains the call
for a store you go on writing to.

Together: ~2.8 + 1.0 = **3.8 pages, 250 KB** per cold query, against 27 + 20 = 47
as built and a 5.9 MB gzipped whole image — and a second keystroke costs ONE page
with the reader's 64-page cache warm.

What the layout unblocked: **a trie is paged** — the work the numbers above made
worth building at 3.8 pages a query where it was not at 47.

#### A paged trie — `store_load_key_text` and `store_load_prefix`

`paged_reader::trie_find_rec` answers one text key by a root→leaf descent, and
`trie_prefix_recs` answers a prefix by a seek plus a bounded in-order walk. They
sit beside `find_hash_entry` and `sorted_range_positions`, and reach the surface
as `store_load_key_text` (extended to a trie root) and `store_load_prefix`
(`local, path, pre, limit`; `limit < 0` = no cap). `store_bind_lazy` accepts a
trie image, so a bound trie faults into its source like a bound hash.

**One walk, two storages.** The paged reader does not carry its own copy of the
descent. `radix_tree` exposes the geometry over a `TreeNodes` / `TreeKeys` source
(`descend_gen`, `split_point_gen`, `descend_extreme_gen`, `RadixIter::step_gen`,
`seek_gen`) plus the two subtlest derived facts — `composed_bit` (the
`user bits ‖ 0x00 ‖ id` string) and `first_diff_words`. The resident tree passes
`StoreNodes`/`StoreKeys`; the reader passes a source that answers a node by
FETCHING, which is why the trait methods take `&mut self`. What remains in
`paged_reader` is the node accessor, the key read (a string record through its
pointer) and the two query wrappers. `trie_db::paged::r12` pins the two answering
identically — every key, every prefix, every cap, on both node layouts.

**Fuel, because an image is a file.** Reads are already total (the reader
zero-pads past EOF), but a cyclic child pointer in a truncated or foreign image
would spin a descent. The paged source refills a hop budget at `walk_begin` — per
WALK, not per query, since a seek runs four of them and a guessed multiple
under-provisions on a small tree and over-provisions on a large one. Exhausting
it answers `Empty`, so a corrupt image reports ABSENT instead of hanging.

**The cap bounds the walk.** `t["kerk"..:8]` stops stepping at the eighth record,
so the ninth's pages are never fetched. A walk that materialised the run and then
truncated would read all 459 records for `kerk` to return 8 — the one operation
where paging could quietly become a whole-image read.

Two things the layout pass deliberately does NOT do, so neither reads as a defect:

- **It runs on the FRESH bind only.** Re-binding an existing file leaves its
  layout alone — the image is already laid out if this loft wrote it, and
  rewriting someone's file to improve a read cost is not a bind's business.
- **A bound store drifts.** Inserts after the bind mint node ids at the tail in
  insertion order again, so a long-lived writable image slowly loses the layout.
  For the shape this is for — build a vocabulary, persist it, serve it read-only
  — that never happens; for a store written to over months it would, and the
  answer is to persist afresh rather than to relayout on every insert.

### A bounding box is paged too, and it is a different walk (@PLN136)

@PLN134's motivation named `spatial` as the next consumer of the same geometry.
That was half true, and the false half is the whole of this section: **a bounding
box is a different WALK.** A prefix is a seek to one point followed by an in-order
run that stops at the first key not bearing it — one contiguous interval. A box
over a Morton code is not one interval: the curve leaves the box and comes back,
so the box's records are several disjoint runs and the query has to know where the
next one starts. So the paged GEOMETRY is reusable and the query is not.

Measured before anything was built, over 3.19 M real OpenStreetMap points across
the Benelux (`radix_db::pages`, `#[ignore]`; a 158 MB image, 2532 pages of 64 KB).
Records READ to answer one box, uncapped:

| box | in the box | the walk reads | the code interval |
|---|---|---|---|
| ±220 m (a street) | 104 | 126 | 1 155 |
| ±2.2 km (a viewport) | 4 875 | 4 985 | 47 327 |
| ±22 km (a city) | 93 762 | 94 146 | 492 480 |
| 222 km × 440 m (a wide strip) | 3 965 | 5 064 | **1 463 785** |
| 440 m × 222 km (a tall strip) | 3 297 | 4 046 | **994 691** |

The degenerate rows are the answer. Seek-to-one-corner-and-walk-to-the-other reads
289× what the box holds on the wide, shallow viewport a map actually issues — so
"read the pages the walk touches" is not a sentence about a spatial index until
the walk stops reading the gaps.

**`radix_db::box_walk` is what stops.** On a record outside the box it computes
BIGMIN (Tropf & Herzog, 1981) — the smallest Morton code ≥ the current one that is
back inside the box — and SEEKS there through `radix_tree`'s own `seek_gen`, so
the gap's records are never read and neither are their pages. The bounds come off
a RECORD rather than off the path, and that is not an implementation detail: a
PATRICIA path skips exactly the high-order bits every record below it shares, so
bounds built from the tested bits alone stay the whole plane, reject nothing, and
the walk degrades into a correct full traversal. (It did, in the first draft here.)

With the walk pruning, the layout question is the trie's again. One capped
200-marker viewport query:

| | as built | BFS | key order | DFS | **vEB** |
|---|---|---|---|---|---|
| node pages, 64 KB | 222.4 | 20.1 | 15.3 | 8.5 | **3.6** |
| record pages, 64 KB | 203.4 | — | — | — | **1.7** (Morton order) |

And panning that map, against a 64-page reader cache:

| image | step 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 |
|---|---|---|---|---|---|---|---|---|
| as built | 426.1 | 427.7 | 419.0 | 399.0 | 375.1 | 363.3 | 352.5 | 347.8 |
| **vEB + Morton order** | **5.3** | **1.6** | **2.1** | **1.6** | **1.5** | **1.2** | **1.1** | **1.2** |

Against 2532 pages for the whole image, downloaded once. So the same four changes
the trie needed, made in ONE change because the last two are two halves of one
fact:

- **`type_is_compactable`'s `Radix` arm accepts**, so `store_persist_copy` writes a
  spatial image with its records in Morton order (`copy_claims_radix_body` already
  re-inserted in walk order; only the gate refused).
- **`relayout_trees`** (named `relayout_tries` while a trie was the only kind it
  found) finds `Parts::Radix` roots as well as `Parts::Trie` ones. Both kinds ARE the
  tree, and one numbering decides what either costs.
- **`store_load_box(local, path, from, till, limit)`** — the paged query.
  `paged_reader::spatial_box_recs` drives `box_walk` over a `PagedSpatial` source;
  the corners are `vector<integer>` so one call serves 1, 2 or 3 axes.
- **`unservable_kind` and `collection_type_of_store` accept `Radix`** in the same
  change, because a kind that becomes servable and is not removed from the refusal
  list keeps refusing a binding that would now work — and `fetch_from_file` routes a
  lazy fault by the COLLECTION's kind rather than the key's shape, since a spatial
  lookup arrives as one `Content::Long` per axis and a one-axis one is
  indistinguishable from a hash's integer key. A point faults as the degenerate box,
  loading every record AT that coordinate (a spatial keeps duplicates, and taking the
  first would leave the rest permanently unreachable). Narrowing the refusal without
  that routing is the loft#802 shape exactly: binds `true`, answers `null` forever,
  `store_lazy_error` empty. `802-lazy-refusal-visible.loft` carries the cell.

**The cap bounds the WALK**, the lesson @PLN134 pinned: `limit` 200 means the 201st
marker is never stepped to. That is a page COUNT to test, not an answer to test —
a walk that materialised the box and truncated returns the same records.

**Two readers of one coordinate.** The tree walk is shared, but reading a record's
AXES is not: `radix_db::axis_i64` goes through `Store`'s checked accessors and
`PagedSpatial::axis_value` decodes the same bytes out of an image, across six
integer widths each with its own null sentinel. A disagreement on one width does
not present as an error — it presents as a box query missing a point inside it —
so `radix_db::paged` drives every width through both readers AND checks each
against the value that was written.

The gates: `radix_db::tests::d5`/`d6` (the walk is exactly the box, and skips the
gaps), `radix_db::paged` (both storages agree, the query reads a small fraction of
the image, the cap bounds the walk), and
`store_persist_loft.rs::a_spatial_survives_the_rebuild_and_comes_out_in_morton_order`
end to end on both backends.

### Adding or changing a collection kind — the per-kind lists

A `Parts` collection variant is not implemented in one place. It has to be
named in each per-kind dispatch below, and **an omission does not read as a
missing feature** — the surrounding kinds keep working, so the gap surfaces
later as a crash or as silent corruption. loft#720 was three such omissions of
`Radix` at once, each failing differently:

| Site | Omitting the kind gives you |
|---|---|
| `Stores::get_keys` (`database/search.rs`) | **Stack desync.** The answer decides how many values `read_key` pops, so an empty list pops NOTHING and the next `get_stack::<DbRef>()` reads a leftover key value as the collection — `sp[3, 3]` looked itself up in store #3. |
| `Stores::find` / `remove` / `remove_owned` | Lookup or unlink silently does nothing, or reads the element at the wrong frame. |
| `Stores::set_keyed` | `coll[k] = v` falls through to the update-only `OpCopyRecord`, which no-ops on an insert-miss — and copying into a null lookup **clobbers the collection root**. |
| `towards_set_hash_remove` / the `OpSetKeyed` route (`parser/collections.rs`) | The removal or the insert never lowers to the runtime that handles it: the interpreter corrupts the store, `--native` fails to compile a void argument. |
| The `is_radix` scratch selector (`parser/collections.rs`) | `for x in coll` takes the HASH builder — a bucket walk over a tree. `trie` hit this: the site names every keyed kind, so the sweep had counted it as mechanical and handled. |
| `emit_field` (`generation/mod.rs`) | A keyed STRUCT FIELD's type id is never registered on `--native`, and its record reads as a struct with no fields (`field_type` indexes an empty list). Local-only vars still work, so it looks kind-specific rather than field-specific. |
| `Iterated` (`database/descriptor.rs`) and its readers | The layout descriptor, `type_of(…).collection` and the lazy-store SQL deriver all match `Iterated` exhaustively, so these are compile errors — EXCEPT `ffi_deliver::collect_keyed`, which is `#[cfg(target_arch = "wasm32")]` and therefore dead on the host that compiles the audit, and `rewrite_iterated`, which closes with `_ => continue`. Check the wasm target explicitly. |
| `Stores::borrowed_spine` (`database/allocation.rs`) | **A use-after-free, or a leak.** It answers what a SECONDARY VIEW of a linked group owns (loft#898). A kind missing from it falls through to the OWNING walk and frees the records its primary holds; a kind wrongly added with no spine leaks the block it should release. It rides the same per-`Parts` match as `for_each_owned_child` for exactly this reason — the spine a view drops is the `container_rec`/`extra_recs` that walk already names. |
| `Stores::unservable_kind` (`database/allocation.rs`) and `collection_type_of_store`'s `is_keyed` | **A binding that reports itself healthy and answers nothing.** The paged loader serves a `hash`, a `trie` and a `spatial`, so every other kind must be refused at `store_bind_lazy`; a kind missing from the check binds, answers `null` at every lookup, and leaves `store_lazy_error` empty — whose documented meaning is "reachable, genuinely no such key" (loft#802). The refusal is a STATIC property of the pair, so it costs no I/O to give and there is no reason to defer it to a lookup. The list runs BOTH ways: a kind that becomes servable and is not removed keeps refusing a binding that would now work, which is why @PLN134 moved the trie out of it in the same change that made it pageable, and @PLN136 the spatial. |

Two habits that make the class visible instead of latent:

- **Spell the non-collection variants out; never close one of these matches
  with `_`.** `get_keys` had a catch-all, so adding `Radix` to `Parts` compiled
  cleanly with the kind missing. `Stores::remove` lists them, and would not
  have. The verbosity is the point — it turns "someone must remember" into a
  compile error.
- **Check the interpreter, not just `--native`.** The two derive key lists
  separately: native builds its `&[Content]` inline in generated code and never
  calls `read_key`, so a `get_keys` gap passes every native test while the
  interpreter faults on the same line.
