
# Database and Storage Layer

## Overview

The runtime data layer is split across multiple source files that together implement a typed, heap-allocated, store-based memory model:

## Contents
- [Overview](#overview)
- [Store — Raw Heap Allocator (`src/store.rs`)](#store--raw-heap-allocator-srcstorers)
- Stores — Type Schema + Multi-Store Manager (`src/database/`) — [DATABASE_STORES.md](DATABASE_STORES.md)
- [DbRef, Key, Content — Universal Pointer and Key Types (`src/keys.rs`)](#dbref-key-content--universal-pointer-and-key-types-srckeysrs)
- [Vector Operations (`src/vector.rs`)](#vector-operations-srcvectorrs)
- B+tree, Open-Addressing Hash Table, Spatial Index, Text Trie — [DATABASE_INDEXES.md](DATABASE_INDEXES.md)
- [How the Layers Fit Together](#how-the-layers-fit-together)
- Retired surfaces (bound and durable stores) — [DATABASE-history.md](DATABASE-history.md)

---

| File | Role |
|---|---|
| `src/store.rs` | Raw word-addressed heap allocator (`Store`) |
| `src/database/mod.rs` | `Stores` constructor, basic get/put, parse-key helpers |
| `src/database/types.rs` | Type-building: `structure`, `field`, `finish`, `sorted`, `hash`, sizes |
| `src/database/allocation.rs` | Store claim/free, `copy_claims*`, `clone_for_worker` |
| `src/database/search.rs` | Find/iterate: `find`, `find_vector`, `find_index`, `next`, `remove` |
| `src/database/structures.rs` | Record construction, field get/set, `vector_add`, struct parsing |
| `src/database/io.rs` | File I/O: `read_data`, `write_data`, `get_file`, `get_dir`, `get_png` |
| `src/database/format.rs` | Display/formatting: `show`, `dump`, `rec`, `path` |
| `src/keys.rs` | Universal store pointer (`DbRef`), key descriptors, compare/hash |
| `src/vector.rs` | Dynamic arrays: by-value (Vector), by-reference (Array/Ordered) |
| `src/btree.rs` | B+tree arena for `index<T>` |
| `src/hash.rs` | Open-addressing hash table for `hash<T>` / `index<T>` by hash |
| `src/radix_tree.rs` | Store-backed binary PATRICIA/radix tree over an abstract bit-key oracle (backs `spatial<T>`) |
| `src/radix_db.rs` | DB↔tree bridge: Morton/Z-order key interleaving + range/proximity primitives for `spatial<T>` |
| `src/spatial.rs` | Morton-coded near/within/nearest geometry algorithms used by `src/radix_db.rs` |

---

## Store — Raw Heap Allocator (`src/store.rs`)

See `src/store.rs` module docs for memory layout, signed size headers, and
free-block allocation.  See `src/keys.rs` module docs for `DbRef`, `Str`,
`Key`, and `Content` types.

**A store's memory is not filled** (@C154).  A buffer is allocated and grown without
zeroing, and a claim promises no value (@C137), so a word nothing wrote holds whatever the
allocator handed out.  The one exception is a DEFINED store (`Store::defined`), whose raw
bytes leave the process — an image being written, a placed library's arena, the REPL's
resume image, the stack census: it is zeroed at creation and growth, or copied from an image,
and `Store::raw_bytes` refuses any other.  The allocator never reads a claimed block's
payload: a delete asks the claims whether a claimed block ends at its left edge before it
reads a free footer there (`formal/allocator.md` `(H-FreeFooter)`).

### Working-set store loader + layout sidecar (`.dschema`) — @PLN97 arc G

`store_persist_copy(collection, path)` writes a keyed collection's image and a
`<path>.dschema` **layout-identity sidecar** beside it
(`src/schema_sidecar.rs`: `LayoutIdentity` = the `layout_algo_hash` + per-type
layout dump). The **working-set loaders** — `store_load_key` / `store_load_keys`
/ `store_load_key_text` / `store_load_keys_text` (hash and trie point lookups,
each with a batch form that opens ONE reader), `store_load_range` (sorted
range) and `store_load_prefix` (trie prefix) — materialise only the entries a
query touches, reading just the pages those touch from a **local file or an
`http(s)://` Range server**
(`src/paged_reader.rs`), then relocate each matched entry's heap graph into a
sound local store (`store_verify` proves the copy; every copyable field shape is
handled, `vector<text>`/`vector<vector>` safely refused).

Before any schema-derived read the loader checks the `.dschema`: a store whose
recorded layout differs from the loading program's collection type is REFUSED
(the **layout-identity gate**) rather than misread as foreign bytes; an absent
sidecar (a legacy store) falls back to the `store_verify` backstop. Set
`LOFT_LOADER_STATS=1` to observe `bytes_fetched` vs file size.

#### A changed definition: the store upgrade

The WHOLE-IMAGE loaders read the file through the program's types, and records are
fixed-stride, so a store whose structs changed — even by one field added at the end — must
never be read raw (`@FR-L-Sound`).  They CONVERT it instead: the
`.dschema` names the layout that wrote the file, the store upgrade map pairs it with the
program's, and the converter builds a new store in which every stored value arrives under its
own name and every added field takes its default (a default may read the record through `$`).
A change the map cannot carry — one that looks like a rename, a narrowed number, a changed
kind, a changed key with duplicates under it — is refused: the call returns `false` and names
the reason.  `loft layout upgrade <program> <store>…` converts a file on disk and prints what it
did; `loft layout check <program>` prints what a store written with the recorded baseline
(below) would go through.

**One map, one converter.**  The upgrade map (`UpgradeMap`, `src/database/upgrade_map.rs`) is
derived from the old layout description and the new type table and nothing else: per new
field where its value comes from, per old field whether it is dropped, per keyed collection how
it is rebuilt, and the refusals with their reasons.  The check prints it, the converter
(`Stores::upgrade_store`, `src/database/upgrade.rs`) executes it, and every caller below runs
that converter, so no caller re-decides a field's fate.  Old bytes are read only through the
old layout and written only through the new one; a type whose dump line (closure included) is
unchanged is copied raw.

| change | upgrade |
|---|---|
| field added | its default: a constant is written directly; an expression runs over the upgraded record after the carried fields, so it may read `$` |
| field removed | not copied, nor the heap it owned |
| number widened (narrow to wide integer, single to double float) | converted; a null stays null (`@FR-L-Null`) |
| number narrowed, a field's kind changed | **refused**, naming the field and both types |
| enum variant added / removed | nothing to do / a value of the removed variant becomes null |
| a field removed and one added in the same type | **refused** as a rename: do it in two steps (below) |
| key of a keyed collection changed | rebuilt by insertion under the new key |
| index added over records already stored (`@FR-Col-Group`) | built by insertion |
| an insert that would displace a record (a duplicate under a new key) | **refused**, with the count and the first key |
| index removed | not copied; the records it shared stay with the other members |
| `T` turns linked or unlinked (a keyed collection over it added or removed anywhere) | each element rewritten at the new stride, never a raw copy; the report names the collection that flipped it |
| a reference inside the store | re-pointed through the old-to-new record map |
| a reference into another store | **refused** |

**A rename takes two upgrades.**  A default is type-checked against the NEW struct, so
`name: text = $.label` compiles only while `label` is still declared: release N adds `name`
filled from `label`, a later release drops `label`.  No single upgrade both loses a name and
creates one, so a wrong guess about a drop and an add cannot destroy data.

```loft
struct Item {                          // v2 of { id, label, price }
  id: integer, label: text, price: integer,
  name: text = $.label,                // a rename, step 1
  cents: integer = $.price * 100,      // a unit change
  title: text = "{$.id}: {$.label}"    // a combination
}
```

| path | an older layout |
|---|---|
| `store_load`, `store_load_untrusted` | converted in memory; the file is left as it is |
| `store_load_url`, `store_load_url_trusted` | converted in memory, after the pin is checked; the sidecar is `<url>.dschema` over the same transport |
| `swap_world` on a live build swap | the running world converts into the new build's types |
| the working-set loaders above | refused (`refuse_paged`, so the program can read the refusal back) |
| `loft layout upgrade <program> <store>…` | the FILE is upgraded in place: written beside, swapped in, the original kept as `<path>.old` (a later upgrade replaces it) |

An image with no sidecar is refused too: nothing records the layout it was written with
(formal/structures.md `S-Gate`).  A refusal leaves the file, its `.dschema` and the
collection as they were.
The raw paths share one verdict, `Stores::layout_verdict_ok`, and the converting paths one
entry, `Stores::layout_changed` with `load_upgraded`; a new whole-image
loader uses both rather than growing its own.

**A store that skipped releases** (`@FR-L-Release`, `@FR-L-Upgrade-Routine`).  The `.dschema`
also records the release that wrote the store: the `version` of the writing program's
`loft.toml`.  A conversion sees only the store's layout and the program's, so a value an
intermediate release moved and a later one dropped would be lost — the drop and the add in one
record are refused as a rename.  The programmer writes the steps in between as a routine, one
per record type, in the program or in a separate upgrade module run with
`loft layout upgrade up.loft <store>`:

```loft
struct Item { id: integer, count: integer }       // release 3.0; 2.0 moved label into count
struct ItemBefore2 { label: text }                // the stored fields the routine reads
fn upgrade(from: Release, old: ItemBefore2, item: Item) {
  if from < "2.0" { item.count = len(old.label) }
}
```

It runs on every record of `Item` converted from a store older than the program, after the
fields carried by name and the added fields' defaults; `old` is filled by name from the stored
record, and a field the stored record lacks takes its default, as an omitted field does in a constructor.  `[package] data_compatible_with = "2.0"`
refuses stores written before 2.0 (retiring the routine's older branches), and
`data_bridge = "2.3"` names the release the refusal sends the user to: *this store was written
by 1.4, and this program reads stores from 2.0 on; open it once with 2.3, then with this one*.
A store that records no release passes the floor.

The compiler catches the move at the edit that loses it (`@FR-L-Upgrade-Source`):
`loft layout accept` — and `loft package`, the release step of a library and of an application
alike — records in the baseline
(`layout.lock` beside `loft.toml`, committed with the release) which stored fields each computed field reads
(`Item.count <- inner.label since 2.0`), and a later build that drops one of them is refused —
*Inner.label is dropped, and 2.0 computes Item.count from it* — until the build has an
`upgrade` routine for `Item` or `data_compatible_with` is `"2.0"` or later.  Every compile
of the project shows it as the `layout-upgrade-refused` warning at `struct Item` — the program
still runs — while `loft layout check` exits 1 on it and `loft package` will not release over
it (DIAGNOSTICS.md).

**Which types are checked** (`Data::surviving`): the types whose values outlive the program —
what a `store_*` call or `swap_world` hands over, every `pub` type of the project (a library's
user may store it), and every type one of them holds.  A store call on an untyped `reference`
makes every type count.  A stored type that only a side program declares is not recorded.

**A running world is checked the same way.**  A live rebuild compiles the next build against
the running world's layout (`LOFT_CHECK_WORLD`, `Stores::world_upgrade`), so a change the map
refuses fails the build and the swap is never offered; the world then crosses through
`swap_world` and the converter, in a native host and in the browser
(`Stores::load_world_image`).  In `loft --serve` a save compiles the file against the running
game's world — a refusal is the same `layout-upgrade-refused` warning, at the world's type —
and only **⇄ Apply** (Ctrl+Shift+Enter) activates the rebuilt program
([plans/16-debugger/IDE.md](plans/16-debugger/IDE.md)).

#### What an image's SIZE and BYTES mean

`store_persist_copy` writes the collection REBUILT into a fresh store, so an image holds what
the collection holds and nothing of how it was built: no interior free space, no growth slack,
no tables a hash outgrew, and every record in its collection's own order.  Two programs that
build the same content in different orders write the same records in the same places.

**`store_reclaim(collection)` gives a LIVE store's tail back** (@PLN123 arc A): it trims the
store's capacity to its high-water mark plus an eighth and answers the bytes it handed back,
with every surviving record bit-for-bit unchanged.  The eighth is the growth reserve: the store
stays live and growth multiplies by 7/3, so one trimmed to the byte pays a 2.33× resize on its
very next claim — and iterating a keyed collection claims its key-sorted snapshot inside the
store, so a READ is such a claim.  Safe without any reference tracking: everything above the
high-water mark is free by construction, and a `DbRef` is a POSITION (`store_nr, rec, pos`),
not a pointer — nothing can name a word above the mark, and no record moves.  What it will NOT
touch is the interior; read `store_memory()`'s `tail%` / `inner%` to see which of the two you
have.  It is opt-in ([STDLIB_RUNTIME.md § Memory diagnostics](STDLIB_RUNTIME.md)): on a
churning store, calling it per cycle buys density with 55× the store's size in resize traffic.

**The INTERIOR is taken when a store is LOADED** (@PLN123 arc B).  The space *between*
surviving records needs the collection rebuilt somewhere dense, which moves records — so it
happens only where an interior `DbRef` cannot be live: `store_load`, which replaces the slot's
bytes wholesale.  The one position that never moves is the collection root, because the
collection variable itself is a `DbRef` at it.  It is **gated**: a store whose interior free
space is under an eighth of its high-water mark is measured and left alone.  It declines, and
says why under `LOFT_LOADER_STATS`: a spatial (`Radix`) collection, a record holding a
`reference<T>` into another store, an untyped, read-only or borrowed store, and a store at or
below the image floor.  `LOFT_NO_COMPACT_ON_LOAD` turns it off.

**`LOFT_HASH_SEED=<n>` makes a build byte-reproducible.** A hash draws a random
seed (the P253 hash-DoS defense, `keys.rs::fresh_seed`) and stores it in its
bucket record, where it decides the bucket ORDER — so rebuilding identical data
gave a different file every run, and a per-block checksum could not separate "the
data changed" from "it was rebuilt". Setting this fixes the seed for every hash in
the process. It is opt-in because a program taking attacker-supplied keys still
wants the randomness; a publishing pipeline does not.

These loaders work **on every target, the browser included** (loft#678). Only the
byte source differs, and it is the sole thing that does: a native build issues
`Range` GETs over `ureq`, while `--html` issues them through the asyncify
`fetch()` host import that `store_load_url_trusted` already uses
(`net::fetch_range`, behind `PageProvider`). Everything above that seam — the
paged reader, the traversal, the relocating copy — is one code path, so a browser
load reads the same pages a native one does: `tests/paged_browser.rs` runs a real
`--html` bundle against a 3.8 MB store and pins the cost at a bounded handful of
64 KiB pages (~7% of the image), the same fraction the native path reports.
The build-time availability is the `paged_store` cfg (`build.rs`): the
`remote-store` feature, or the browser target, which cannot use `ureq` at all.

**Every paged refusal is reported on stderr** (`store loader: refusing <path> — …;
loaded NOTHING (a refusal, not an absent key)`). These loaders signal failure as
`false` / `0`, which is exactly what an ABSENT KEY looks like, so a silent refusal
reads as missing data and hides an unsupported shape. The refusal reasons are: the
layout gate above; an unopenable source; a store with no recorded type; an entry
with a field the working-set copy cannot relocate (`vector<text>` /
`vector<vector>` — see `store_load_vectext_refuse.loft`); and **a collection
declared as a struct FIELD**, whose store records the *wrapper struct* as its
type so no hash/sorted root is found ([#632](https://github.com/loft-lang/loft/issues/632)
— declare it as an annotated local `h: hash<T[k]> = []` for paged loads, or read it
whole with `store_load`, which carries the field form fine). Pinned by
`store_load_field_refusal.loft`.

Full design:
[`plans/97-layout-contract/REMOTE_STORE_LOADER.md`](plans/97-layout-contract/REMOTE_STORE_LOADER.md);
the layout contract itself: [`formal/layout.md`](formal/layout.md).

---

**Stores — type schema and multi-store manager (`src/database/`)** — see [DATABASE_STORES.md](DATABASE_STORES.md).

---

## DbRef, Key, Content — Universal Pointer and Key Types (`src/keys.rs`)

### DbRef

```rust
pub struct DbRef {
    pub store_nr: u16,   // which Store in Stores::allocations
    pub rec: u32,        // word offset of the record within the store
    pub pos: u32,        // byte offset within the record (field position)
}
```

`DbRef` is the universal runtime pointer. It encodes a complete address: which store, which record, and which field offset within that record.

**Absence has two spellings, and a site that knows only one is a bug waiting to happen.**

| spelling | produced by | test |
|---|---|---|
| `DbRef::NULL` — `store_nr == u16::MAX`, `rec == 0` | an absent heap value: unset struct reference, struct-enum, vector | `DbRef::is_null()` |
| a real store, `rec == 0` | indexing **past the end** of a live container (`vector::get_vector`), and `from == i64::MIN` | `rec == 0` |

`get_vector`'s own doc says the two "read as the same absent value", and every
store *accessor* honours that by testing `rec == 0` (`if db.rec == 0 { f64::NAN }`).
A site that tests only `store_nr == u16::MAX` therefore accepts an out-of-range
element as PRESENT. That is how loft#823 turned `v[oob] ?? default` into a live
empty record on `--native`: the materialise arm allocated first and asked the
narrow question second, so `??` saw a present value and answered with the fresh
record's uninitialised bytes. **Use `rec == 0` for "is this readable"; reserve
`is_null()` for "is this the absent-value sentinel" specifically.**

#### A null in flight is not a null in a slot

The parser has two helpers for "the null of type τ", and picking the wrong one is
silent on every scalar and corrupting on every collection:

| helper | for | a collection's answer |
|---|---|---|
| `Parser::null(tp)` | a VARIABLE's default-init — a declared slot the value lands in | `Value::Null` (nothing) — the slot is an allocated empty store already |
| `Parser::null_value(tp)` | a VALUE POSITION — the null travels on the eval stack | `OpNullRefSentinel()` — a 12-byte `DbRef` with `store_nr == u16::MAX` |

The split is real: a collection LOCAL must start as an ALLOCATED empty store,
because a later write (`w: vector<single> = f#read(16) as vector<single>`) fills
that store in place and the sentinel's `store_nr` indexes nothing. A value in
flight has no slot to fill, so there the sentinel is the only thing that can mean
null.

**Every branch-MERGE slot is a value position, exactly as a `return` is** — the
arms of an `if` or a `match` all push into one join, and an arm that pushes
NOTHING leaves the join reading an unwritten, value-sized slot: an uninitialised
`DbRef` the interpreter then treats as a live reference (loft#936), or a lost
value on `--native`. Arm ORDER is what made this look cosmetic rather than
systemic: the SECOND arm is parsed with its sibling's type already in hand and
converts correctly, so only a `null` written FIRST reached the back-patch that
asked the wrong helper.

`null_value` peels `Optional` and delegates to `null` for everything outside the
DbRef-backed family (the collections and a struct-enum, whose payload is a record
rather than the `255` discriminator a plain enum uses), so it is strictly the
safer default at any site that is not a declared slot.

### Key

```rust
pub struct Key {
    pub type_nr: i8,    // positive = ascending, negative = descending; magnitude = type code
    pub position: u16,  // byte offset of this field within the record
}
```

Type codes for `Key::type_nr`:

| Code | Type |
|---|---|
| 1 | `integer` (32-bit) |
| 2 | `long` (64-bit) |
| 3 | `single` (32-bit float) |
| 4 | `float` (64-bit float) |
| 6 | `text` (string reference) |
| other | byte-sized field |

Negative `type_nr` means descending order for that key field.

### Content

```rust
pub enum Content {
    Long(i64),
    Float(f64),
    Single(f32),
    Str(Str),
}
```

Used as the return type of `get_key` when extracting a key value from a record for comparison or hashing.

### Str

```rust
pub struct Str {
    pub ptr: *const u8,
    pub len: u32,
}
```

Zero-copy string reference into store memory. Lifetime is tied to the store; no heap allocation.

### Key Functions

| Function | Description |
|---|---|
| `compare(store, rec, other, keys) -> Ordering` | Compare two records by a list of `Key` fields |
| `key_compare(store, rec, key_vals, keys) -> Ordering` | Compare a record against extracted `Content` values |
| `hash(store, rec, keys) -> u64` | Hash a record by its key fields |
| `key_hash(key_vals, keys) -> u64` | Hash a list of `Content` values using the same algorithm |
| `get_key(store, rec, key) -> Content` | Extract one key field value from a record |
| `store(db_ref) -> &Store` | Resolve a `DbRef` to a `&Store` (shared borrow) |
| `mut_store(db_ref) -> &mut Store` | Resolve a `DbRef` to a `&mut Store` |

### The key resolved once: `FastKey` and `FastOrder`

`key_compare` and `compare` decide a key's KIND per call — a `(Content, type_nr)` match, a
bounds-checked store lookup, a validated read: 90–100 instructions for what is one load and
one compare.  A search asks about the same key every step, so the keyed searches resolve it
once and compare with a read:

| Form | Asks | Used by |
|---|---|---|
| `fast_key(keys, key) -> Option<FastKey>` | equal or not (`matches`) | the `hash::find` probe loop (@PLN135 arc B) |
| `fast_key_of(rec, stores, keys)` | the same, for a RECORD's own key | `hash::probe_for_insert` |
| `fast_order(keys, key) -> Option<FastOrder>` | how it ORDERS (`compare`), direction applied once (@FR-Col-Order-Sign) | `btree::find`, `btree::find_exact`, `vector::sorted_find`, `vector::ordered_find` |
| `fast_order_of(rec, stores, keys)` | the same, for a record's own key | `btree::add`, `sorted_finish`, `ordered_finish` |

All four answer `None` for a compound or partial key and for a width they do not list
(`u8`, `u16`, `single`, `float`), and the search then takes the general comparator
unchanged.  Every search calls ONE of `keys::order_key` / `keys::order_record`, which is
also where `LOFT_KEYED_VERIFY=1` checks the fast answer against the general one.  `LOFT_NO_FAST_ORDER=1` makes the order forms answer `None` everywhere.

---

## Vector Operations (`src/vector.rs`)

Three distinct collection layouts share the vector source file.

### By-Value Vector (`Vector` / `Parts::Vector`)

Elements are stored inline within the vector record:

```
word 0: claimed size (in words, same as Store header)
word 1: length (element count)
word 2+: element data (size bytes per element, packed)
```

Initial capacity claim: `(11 * element_size + 15) / 8` words — room for approximately 11 elements before the first resize.

| Function | Description |
|---|---|
| `vector_add(store, rec, size) -> u32` | Append one element slot; returns byte offset of new element |
| `vector_remove(store, rec, pos, size)` | Remove element at byte position `pos`; shifts remaining elements |
| `vector_next(store, rec, pos, size) -> u32` | Advance byte position by `size`; returns next byte offset |
| `vector_step(store, rec, index, size) -> u32` | Advance to next element index (forward) |
| `vector_step_rev(store, rec, index, size) -> u32` | Advance to previous element index (reverse) |
| `vector_length(store, rec) -> u32` | Return element count |

### Narrow vector elements

Vectors of narrow integer aliases (`vector<u8>` / `vector<u16>` /
`vector<i8>` / `vector<i16>` / `vector<i32>` / `vector<u32>`)
honour the alias's `forced_size` so that, e.g., `vector<i32>`
stores 4 bytes per element rather than 8.

The encoding for vector elements differs from struct fields:

- **Struct field** `Parts::Short` encodes `raw = val - min + 1`,
  reserving raw 0 as the null sentinel.
- **Vector element** `Parts::ShortRaw` (added 2026-04-22 alongside
  the rest of @PLAN02) encodes `raw = val - min` directly.

The divergence is required because `vector_add` raw-byte-copies
element bytes from source to destination — the +1 offset of
`Parts::Short` would cause read/write mismatch.  `Parts::Byte` is
direct-encoded and needs no separate "raw" variant, and the 8-byte
fallback (`Parts::Long`) is also direct.

The 4-byte width needs a second variant for a DIFFERENT reason, and
it is the one @FR-L-Narrow-Enc states: the shift is not the only
thing a width leaves undecided — the SIGN is too.  `Parts::Int`
sign-extends and spends `i32::MIN` on absence; `Parts::IntRaw`
zero-extends and spends `u32::MAX`.  `i32` and `u32` are the same
four bytes and different numbers by them, so a reader given only the
width has to guess, and the guess is silent.  Which one a slot uses
is `IntegerSpec::unsigned_wide()`, asked once — see the one-home
table below.

Public surface (in `src/data.rs`):

| API | Returns | Use |
|---|---|---|
| `IntegerSpec::vector_narrow_width()` | `Option<u8>` (1 / 2 / 4, or `None` for the 8-byte fallback) | "Should this vector element narrow?" |
| `Data::narrow_vector_content(content)` | content type with `forced_size` applied | Wrap a content type before calling `database.vector(...)` |
| `NarrowIntKind::of(width, nullable, narrow_vec, unsigned_wide)` | the storage KIND | The one home: which encoding this slot uses |
| `NarrowIntKind::part(db, min, nullable)` | `Option<u16>` | The schema `Parts` id for that kind — what the interpreter registers |
| `NarrowIntKind::part_ctor()` | `Option<&str>` | The `Stores` constructor generated `init()` emits for it |
| `NarrowIntKind::part_name(min, nullable)` | `Option<String>` | The schema key both the constructors and the native generator look it up by |
| `NarrowIntKind::get_op()` / `set_op()` | the op names | The read/write ops the codegen emits |

⚠ **Every one of those answers must come from the same `NarrowIntKind`.**  A site that
re-derives the encoding from the width — a `match n { 1 => …, 2 => …, 4 => … }`, or a
`format!("int<{min},{nullable}>")` rebuilding the schema key by hand — is a place the schema and
the ops can disagree, and the disagreement is a wrong NUMBER rather than an error.  There were
five such sites; loft#1437 is what they cost.

**Compiler-contributor gotcha**: `typedef.rs::fill_database`
walks ONLY struct definitions.  Local-variable / parameter /
return-type vector registration happens at every
`database.vector(c_tp)` call site in `src/parser/`.  Both paths
must call `narrow_vector_content()` on their content type before
registering, or narrowing only takes effect for struct fields.
See [INTERMEDIATE.md § Integer Storage Size](INTERMEDIATE.md#integer-storage-size)
for the per-variant table and the rule selection.

### Sorted By-Value Vector (`Parts::Sorted`)

Same record layout as Vector. Elements are kept in sorted order via binary search insertion.

| Function | Description |
|---|---|
| `sorted_find(store, rec, size, keys, vals) -> (u32, bool)` | Binary search; returns (byte_offset, found) |
| `sorted_add(store, rec, size, keys) -> u32` | Append then insertion-sort to correct position; returns offset |
| `sorted_finish(store, rec, size, keys)` | Insertion-sort the last added element into correct position |

### By-Reference Array (`Array` / `Parts::Array` / `Parts::Ordered`)

Stores 4-byte record references (offsets into a separate store) rather than inline data. Used for `sorted<T>` where `T` is a struct stored elsewhere.

| Function | Description |
|---|---|
| `ordered_find(store, rec, ref_store, keys, vals) -> (u32, bool)` | Binary search over references; dereferences into `ref_store` for comparison |
| `array_add(store, rec, ref_rec) -> u32` | Append a reference; returns slot offset |
| `array_remove(store, rec, pos)` | Remove reference at slot `pos`; shifts remaining |

---

**B+tree, hash table, spatial index, text trie** — see [DATABASE_INDEXES.md](DATABASE_INDEXES.md).

---

## How the Layers Fit Together

```
loft runtime value
    └── DbRef { store_nr, rec, pos }
            │
            ├── Stores::allocations[store_nr]   (Store — raw allocator)
            │       └── record at word offset rec
            │               └── field at byte offset pos
            │
            └── Stores::types[type_nr]          (Type — schema)
                    └── Parts::Sorted / Hash / Vector / Struct / ...
                            │
                            ├── Vector layout   → src/vector.rs
                            ├── Index           → src/btree.rs (Sorted/Ordered: src/vector.rs)
                            ├── Hash            → src/hash.rs
                            ├── Radix           → src/radix_tree.rs + src/radix_db.rs
                            ├── Trie            → src/radix_tree.rs + src/trie_db.rs
                            └── Key comparison  → src/keys.rs
```

- A `hash<MyStruct>` is a bucket-table record in the collection's `Store`.  Its entries sit in a chunked arena, IN the table (an element type no group or reference shares), or are the records a group member holds; each table records its layout and they live side by side ([DATABASE_INDEXES.md § Record Layout](DATABASE_INDEXES.md#record-layout), @C152).
- An `index<MyStruct>` is a B+tree whose nodes are one arena record in the collection's store, holding key parts and the numbers of the element records: O(log n) lookup by key, range queries and ordered iteration.
- A `vector<T>` is a single record with inline elements; a `sorted<T>` by value uses the same layout but maintains sort order via insertion sort on add.
- All cross-record pointers are `u32` rec offsets within the same `Store`; cross-store references use the full `DbRef`.

---

## See also
- [REMOTE_STORES.md](REMOTE_STORES.md) — reading a store image over HTTP range: serving a large
  immutable dataset as a static file and fetching only the pages a lookup touches
- [INTERMEDIATE.md](INTERMEDIATE.md) — Value/Type enums in detail; 233 bytecode operators; State layout
- [INTERNALS.md](INTERNALS.md) — calc.rs, stack.rs, create.rs, native.rs, ops.rs, parallel.rs, radix_tree.rs
- [DESIGN.md](DESIGN.md) — Algorithm catalog with complexity analysis for hash, index, sorted, store
