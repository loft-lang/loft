
# Database and Storage Layer

## Overview

The runtime data layer is split across multiple source files that together implement a typed, heap-allocated, store-based memory model:

## Contents
- [Overview](#overview)
- [Store — Raw Heap Allocator (`src/store.rs`)](#store--raw-heap-allocator-srcstorers)
- [Stores — Type Schema + Multi-Store Manager (`src/database/`)](#stores--type-schema--multi-store-manager-srcdatabase)
- [DbRef, Key, Content — Universal Pointer and Key Types (`src/keys.rs`)](#dbref-key-content--universal-pointer-and-key-types-srckeysrs)
- [Vector Operations (`src/vector.rs`)](#vector-operations-srcvectorrs)
- [Red-Black Tree (`src/tree.rs`)](#red-black-tree-srctreers)
- [Open-Addressing Hash Table (`src/hash.rs`)](#open-addressing-hash-table-srchashrs)
- [Spatial Index (`src/radix_tree.rs`)](#spatial-index-srcradix_treers)
- [How the Layers Fit Together](#how-the-layers-fit-together)

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
| `src/tree.rs` | Left-leaning red-black tree for `sorted<T>` / `index<T>` |
| `src/hash.rs` | Open-addressing hash table for `hash<T>` / `index<T>` by hash |
| `src/radix_tree.rs` | Store-backed binary PATRICIA/radix tree over an abstract bit-key oracle (backs `spatial<T>`) |
| `src/radix_db.rs` | DB↔tree bridge: Morton/Z-order key interleaving + range/proximity primitives for `spatial<T>` |
| `src/spatial.rs` | Morton-coded near/within/nearest geometry algorithms used by `src/radix_db.rs` |

---

## Store — Raw Heap Allocator (`src/store.rs`)

See `src/store.rs` module docs for memory layout, signed size headers, and
free-block allocation.  See `src/keys.rs` module docs for `DbRef`, `Str`,
`Key`, and `Content` types.

### Durable stores (`Store::open_durable`) — @PLN43

A durable store is a normal mmap-backed `Store` plus a 40-byte `.dmeta`
sidecar file alongside the main store file.  The sidecar holds a signature
(`"DStoreV1"`), tier id, CRC32 over the main file, and a `last_clean_ns`
timestamp.  On clean drop the sidecar is rewritten atomically (`write tmp
→ fsync → rename`); on `kill -9` the sidecar stays stale, and the next
open detects corruption and invokes the consumer's rebuild callback.

The main store file is bit-for-bit identical to a non-durable store —
durability is a metadata layer, not a payload-layout change.  Existing
record/claim/resize code paths are untouched.

Three tiers are planned; phase 01 (the first PR slice on the
`store-durable-phase1` branch) ships **Tier 1 — `IntegrityOnly`** only:

| Tier | Mode | Hot-path cost | Loss bound | Consumer |
|---|---|---|---|---|
| 1 | `IntegrityOnly` | None (only msync on clean drop) | Everything since last clean drop | `personal/training` port (initial), `@PLN42` indexer (when phase 08 lands) |
| 2 | `SnapshotEvery(interval)` (planned, phase 02) | One msync per interval | One interval | TTT v5 multiplayer (`plans/future/32-…`) |
| 3 | `WAL` (planned, phase 03) | fsync per record, amortised by group-commit window | Zero for committed writes | @PLN6 audience demo |

API surface:

```rust
use loft::store::{Store, DurabilityMode};

let store = Store::open_durable(
    path,
    DurabilityMode::IntegrityOnly {
        on_corruption: Box::new(|p| rebuild_from_source(p)),
    },
)?;
```

**Fresh-file semantics.**  When the main file doesn't exist yet,
`on_corruption` fires with `TailMarkerMissing` and is expected to
"create empty + populate from authoritative sources" — not "repair
existing file."  After the callback returns successfully, `open_durable`
captures a fresh sidecar and retries once.

**Reading a bound collection invalidates its seal.** Iterating a keyed collection
materialises a key-sorted snapshot, and for a collection bound with
`store_persist_bind` that snapshot is claimed INSIDE the store — so the file's
bytes change and the sidecar's CRC no longer matches, with the file LENGTH
unchanged. The snapshot is released at loop exit, but a claim-then-free still
leaves different bytes than it found. Measured: a bare re-bind keeps
`store_durable_check` true; one traversal makes it false. Seal AFTER the reads
you intend to do, not before. (`store_reclaim` and compaction both refuse
outright on a store with a live sidecar, so those cannot surprise you the same
way.)

**What it does NOT do any more is grow the file.** The snapshot used to be left
behind — the loop epilogue released it only when it had a store of its own, and
a writable collection's is co-located, on the reasoning that its records go back
when the store dies. A collection outlives the loops that read it, and a bound
collection's store is a file that outlives the process, so every read leaked 4
bytes per element into it permanently: sixteen runs of a program that only READ
a 4,000-record hash took its file from 566,472 to 1,321,768 bytes, with no
writes anywhere (loft#727). Reading is free now, in memory and on disk — a
40-pass traversal loop leaves a store's census byte-identical. The old note
"stat the file the instant `store_persist_bind` returns, before anything walks
the collection" no longer applies.

**Drop-on-panic is by design.**  A panic between open and clean drop
skips the sidecar write → next open detects corruption → callback fires.
This is what makes Tier 1 cheap.  Do not use Tier 1 for data that cannot
be re-derived from authoritative sources; use Tier 2 or Tier 3 instead.

Full design + implementation history:
[`doc/claude/plans/43-loft-store-durable/`](plans/43-loft-store-durable/README.md).

### Working-set store loader + layout sidecar (`.dschema`) — @PLN97 arc G

`store_persist_bind(collection, path)` binds a keyed collection to a durable
store and writes a `<path>.dschema` **layout-identity sidecar** beside it
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

The WHOLE-IMAGE loader `store_load` takes the same gate (loft#700). It keeps the
target slot's type and reinterprets the file's bytes through it, and records are
fixed-stride — so **changing a stored struct at all, including adding a field at the
end, changes the layout and makes older stores unreadable**. That is the rule to plan
around: a store is readable only by a program whose structs lay out identically to the
one that wrote it. Before the gate the mismatch was silent, and `len()` on an added
collection returned wild values (`510277628`) that a consumer then iterated. Now
`store_load` returns `false` and names what differs. Rebuild the store with the new
program, or read it with the version that wrote it.

#### What the file's SIZE and BYTES mean (loft#710)

A persisted store used to be the arena's whole **capacity**, so its size said how
the store was BUILT, not what it holds: filling each record's vector whole before
inserting gave 1.84× what growing them interleaved gave for byte-identical data,
and 160,000 coordinates persisted to the same byte count as 290,000. The image is
now sized from the **high-water mark** — the end of the last live record — plus an
eighth. The eighth is not slack for its own sake: a bound store stays live and the
arena grows by 7/3, so persisting with no room left costs a 2.33× file resize on
the very next claim, which is worse than the tail it removed.

**Interior free space is still there.** Reclaiming that means relocating records
and rewriting every `DbRef` — compaction, which @PLN123 arc B remains open for. So
the size now follows the content, but two construction orders can still differ by
the fragmentation each genuinely leaves.

**The bind-first path reaches the same answer at RELEASE (loft#752).** That #710
fix is on the IMAGE WRITE, so it covers `store_persist_bind` LAST and nothing
else. Bind a store FIRST and its file IS the live arena: while the program runs,
the size is the arena's **capacity**, which grows by 7/3 and never shrinks on its
own. So the file used to be quantized to a ladder and could sit up to **57%**
(`1 − 3/7`) above its content. Measured on one generator shape, varying only the
feature count:

| features | file (bytes), before | after |
|---|---|---|
| 150 000 | 39,179,744 | content-sized |
| 200 000 – 400 000 | **91,419,400** (unchanged across a 2× data increase) | one size per count |
| 500 000 – 700 000 | 213,311,928 | content-sized |

Freeing the collection's store now hands the tail back before the slot is marked
free — the same `reclaim_tail` `store_reclaim` calls, at the one moment the
runtime can tell a permanent drop from a lull, because there is no next claim to
pay 7/3 for. `store_reclaim` at the end of a build is therefore no longer needed
and finds nothing left: measured over 40 000 and 60 000 features, with and
without it, the files are byte-identical per count and differ between counts.

⚠ **MID-RUN, a bound store's file size still compares nothing.** Between the bind
and the release it is capacity: two points a rung apart differ by 133% with
byte-identical content. Call `store_reclaim(collection)` before reading a size in
the middle of a run, or do not read it.

This is not a footnote: a consumer measured two insertion orders, saw 2.3×, and
concluded that feeding keys in order — the thing that bounds a generator's working
set — was the worse strategy on every axis. Both numbers were rungs (loft#747).
Right-sized, the same shape leaves a 1.30× spread, which IS the fragmentation the
two orders genuinely differ by.

### Binding FIRST is the low-memory choice, and its pages are reclaimable

**`store_persist_bind` FIRST uses far LESS memory than binding last**, which is the
opposite of what a reader expects and of what loft#747 was filed claiming. Measured
across two generator shapes and two boxes:

| features / tiles | RSS, bind FIRST | RSS, bind LAST |
|---|---|---|
| 400 000 / 40 000 | **12 MB** | 29 MB |
| 1 600 000 / 160 000 | **31 MB** | 125 MB |
| 4 000 000 / 400 000 | **65 MB** | 289 MB |

4.4× lower here, 2.8–5.3× lower on a heavier consumer shape. Bind LAST builds in
anonymous heap and then writes an image; bind FIRST makes the file the arena, and
**file-backed pages are reclaimable while anonymous heap is not**. So the dataset
size does not set a hard memory requirement — it sets a working-set/throughput
tradeoff. Under a hard cgroup cap with swap disabled:

| | result |
|---|---|
| 4 M features, `MemoryMax=32M` | **completes**, 34 MB peak, 271 s, correct read-back |
| 1.6 M, cap 96 MB, bind FIRST | **completes**, 88 MB peak, 27.6 s (12.7 s uncapped) |
| 1.6 M, cap 96 MB, bind LAST | **OOM-killed** |

⚠ **`MemoryMax` alone proves nothing on a box with swap.** A first attempt at the
table above had BOTH orders passing under 96 MB, because the unbound heap simply
paged out to 8 GB of swap. `MemorySwapMax=0` is what makes a cap a cap; without it
the measurement is vacuous in the direction that looks like success.

The cost of capping is that the kernel's LRU only learns the working set by evicting
the wrong pages first — ~2× wall at a modest cap, 271 s at an aggressive one.
`store_release(collection)` lets the program say so instead, and turns that cliff
into a curve.

### `store_release` — the working set follows the program, not the eviction (@PLN126)

`store_release(collection)` starts writing everything below the arena's high-water
mark out to the bound file and drops it from the resident set, answering the bytes
dropped. **Content is untouched and every reference stays valid**: nothing moves and
nothing is freed, the mapping is `MAP_SHARED`, and reading a released record simply
re-reads it from the file one page fault later. It is a HINT — calling it too often,
or on a collection that is not bound, costs a little speed and can never cost an
answer. Not `store_reclaim`, which changes the FILE's length; this never does. Not a
durability barrier either — it asks for writeback to START, which is
`store_durable_seal`'s job to promise.

Measured on a 20 000-record generator, one call per record: **peak RSS 44.3 MB → 2.2
MB (20×) at 1.0× wall clock.**

**It pays for a build in KEY ORDER and does nothing for an interleaved one** (1.01× on
the same data), and the reason is the allocator rather than the layout: a generator
that keeps many records open at once relocates its growing vectors and leaves the
arena scattered with free blocks — 3 691 against 10 for the same data written in
order. The LLRB free-space tree's nodes live *inside* those freed blocks, so a
scattered arena gives the allocator its own scattered working set, which is exactly
the region the release just dropped. Stream in cell order and this is close to free.

Three implementation facts, each of which cost 200× before it was found, and each
recorded because the next residency feature meets all three again:

* **Deriving the frontier costs the whole point.** `Store::usage` walks the block
  chain, one header per block, touching every page of the arena — so asking it what to
  drop faulted the entire store back in first (peak RSS became the whole file, 80.9 MB
  against 44.3 MB for making no call at all). `Store::claimed_end` carries the mark
  forward at `claim_block` instead. It is a monotone UPPER bound on `live_end_words`,
  which is the safe direction here (flushing a few free pages costs a fault) and the
  wrong one for `shrink_to` / `reclaim_tail` / `bind_path`, which still read the chain
  because truncating to an upper bound cuts live data.
* **Each call must be bounded to what is new.** Flushing from zero every time re-syncs
  a region that grows with the run: 208× wall. `Store::released_bytes` is the
  watermark.
* **`MS_ASYNC`, never `MS_SYNC`.** Both reach the same resident set; waiting costs
  ~1.5 ms a call, which is the difference between a call a generator can make per
  record and one it cannot make at all.

**Why there is no per-RECORD release, and it is not a matter of taste.** `MADV_DONTNEED`
works on pages, and `database::spans` measured what one record of a real generator's
shape (`hash<TTile[tkey]>` with two grown-by-append vectors) actually occupies:
**0.0% of the pages a record touches hold only that record**, at every window and every
scale tried. The span between a record's lowest and highest word is 356–1348× the bytes
it owns, because the hash keeps its entries in a chunked arena claimed early while the
record's vectors are claimed at the frontier much later — so a record's own bytes sit
either side of the whole store. A frontier release needs none of that: it needs the
region below the mark not to be written again, and 87–99% of its pages are not.
Full workings: [plans/126-record-frontier.md](plans/126-record-frontier.md).

**A BOUND store's file only ever grew, until `store_reclaim`.** The sizing above
happens when the image is WRITTEN; after that `resize_store` returns early on any
request at or below the current size, so a bound store that grew ten-fold and
dropped back to its original live set kept the ten-fold file for the rest of the
run (12.7× measured). `store_reclaim(collection)` (@PLN123 arc A) truncates the
file to the store's high-water mark **plus an eighth** and answers with the bytes
it gave back — half the file on that shape, with every surviving record
bit-for-bit unchanged.

The eighth is not a rounding: it is the same slack the image format gives a
freshly-bound store, and for the same reason. The store stays live, growth
multiplies by 7/3, so one trimmed to the byte pays a 2.33× resize on its very
next claim — on the FILE. Trimming to the bare mark made `store_reclaim` hand
back 40% of a file and take 133% back on the next read (loft#727). A store that
came straight from `bind_path` is therefore already the right size and answers
`0`; a tail only appears once the store has GROWN while bound.

Safe without any reference tracking, and the reason is worth knowing: everything
above the high-water mark is free by construction, and a `DbRef` is a POSITION
(`store_nr, rec, pos`), not a pointer — so nothing can name a word above the
mark, and no record moves. What it will NOT do is touch the interior; read
`store_memory()`'s `tail%` / `inner%` to see which of the two you have. It is
opt-in for a reason ([STDLIB.md § Memory diagnostics](STDLIB.md)): on a churning
store, calling it per cycle buys density with 55× the store's size in resize
traffic. And it refuses outright on a store carrying a `store_durable_seal`
sidecar — that sidecar records the file's byte length and CRC, so truncating
behind its back would report a healthy store as corrupt.

**The INTERIOR is taken automatically, when a store is LOADED** (@PLN123 arc B).
The space *between* surviving records needs the collection rebuilt somewhere
dense, which moves records — so it happens only where an interior `DbRef` cannot
be live: `store_load`, and `store_persist_bind` on an **existing** file. Both are
loads, and both already replace the slot's bytes wholesale, so a reference held
across them was already meaningless. Binding to a NEW file is a *write* and is
deliberately untouched: a program keeps element references across it, and the
byte-for-byte image is what makes that work.

A bound store that peaked at 2,000 records and settled at 200 came back at
180,104 bytes every run; it now loads at 26,992 — the same records, the same
digest, and still bound. The one position that never moves is the collection
root, because the collection variable itself is a `DbRef` at it.

It is **gated**, which is what lets it be a default: a store whose interior free
space is under an eighth of its high-water mark is measured and left alone. An
eighth is the slack the image format already carries on purpose (the mark plus
an eighth, above), and the estimate is a *lower bound* on what a rebuild returns
— a rebuild also right-sizes live structures the metric counts as data, such as
a hash's bucket array still sized for its peak.

It declines, and says why under `LOFT_LOADER_STATS`: a spatial (`Radix`)
collection, a record holding a `reference<T>` into another store, an untyped,
read-only or borrowed store, a store at or below the image floor, and one
carrying a durable sidecar. `LOFT_NO_COMPACT_ON_LOAD` turns the whole thing off.

**The eighth of slack survives `store_reclaim`** — and for a while it did not.
The image size used to be clamped to the arena's current capacity ("never larger
than we would have written before"), which was safe while capacity sat well above
the mark. `store_reclaim` trims capacity TO the mark, so the clamp collapsed the
eighth to zero for exactly the stores someone had just tidied, and the next claim
paid the 7/3 ladder the eighth exists to prevent. The claim that tripped it was
the most ordinary one there is: READING the collection, because iterating a keyed
collection claims its key-sorted snapshot inside the store. A 2,000-record hash
wrote 187,784 bytes and one read took it to 438,160 — **2.07× larger than never
reclaiming at all**. The clamp is gone; both paths now land on 211,256 and stay
there. Guarded by `persisted_image_keeps_its_slack_after_store_reclaim`.

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
declared as a struct FIELD**, whose bound store records the *wrapper struct* as its
type so no hash/sorted root is found ([#632](https://github.com/loft-lang/loft/issues/632)
— declare it as an annotated local `h: hash<T[k]> = []` for paged loads, or read it
whole with `store_load`, which carries the field form fine). Pinned by
`store_load_field_refusal.loft`.

Full design:
[`plans/97-layout-contract/REMOTE_STORE_LOADER.md`](plans/97-layout-contract/REMOTE_STORE_LOADER.md);
the layout contract itself: [`formal/layout.md`](formal/layout.md).

---

## Stores — Type Schema + Multi-Store Manager (`src/database/`)

### Stores struct

```rust
pub struct Stores {
    pub types: Vec<Type>,           // all registered types
    names: HashMap<String, u16>,    // type name → index
    allocations: Vec<Store>,        // one Store per allocation context
    pub max: u16,                   // number of registered types
}
```

`Stores` owns the complete type schema and all live stores. The `types` vector is append-only at runtime; type indices (`u16`) are stable.

### Fixed Base Type IDs

The following type indices are permanently fixed:

| ID | Type |
|---|---|
| 0 | `integer` (32-bit signed) |
| 1 | `long` (64-bit signed) |
| 2 | `single` (32-bit float) |
| 3 | `float` (64-bit float) |
| 4 | `boolean` |
| 5 | `text` (string) |
| 6 | `character` |

Types 0–6 are registered at construction time and never relocated.

### Type struct

```rust
pub struct Type {
    pub name: String,
    pub parts: Parts,
    pub keys: Vec<Key>,      // key fields for sorted/hash/index
    pub size: u32,           // byte size of one record
    pub align: u32,          // alignment requirement
    pub linked: bool,        // has back-reference (tree backward links)
    pub complex: bool,       // contains non-trivial types (strings, refs)
}
```

### Parts enum

`Parts` describes the runtime layout and category of a type:

| Variant | Description |
|---|---|
| `Base` | Primitive (integer, long, float, boolean, text, character) |
| `Struct(Vec<Field>)` | Named fields with offsets |
| `Enum(Vec<(u16, String)>)` | Discriminated union; entries are (discriminant, name) |
| `EnumValue(u8, Vec<Field>)` | One variant of an enum (discriminant + fields) |
| `Byte(i32, bool)` | Byte-sized integer; `bool` = signed |
| `Short(i32, bool)` | 16-bit integer; `bool` = signed |
| `Vector(u16)` | Dynamic by-value array of element type `u16` |
| `Array(u16)` | Dynamic by-reference array of element type `u16` |
| `Sorted(u16, Vec<(u16,bool)>)` | Red-black tree ordered by key fields; `bool` = ascending |
| `Ordered(u16, Vec<(u16,bool)>)` | Ordered array (binary search) by key fields |
| `Hash(u16, Vec<u16>)` | Open-addressing hash table; field indices as hash keys |
| `Index(u16, Vec<(u16,bool)>, u16)` | Combo: sorted tree + hash table for a single collection |
| `Radix(u16, Vec<u16>)` | Spatial index for `spatial<T[x,y]>` / `spatial<T[x,y,z]>` — Morton/Z-order radix tree, 1–3 coordinate axes (renamed from `Spatial`) |
| `Trie(u16, u16)` | Text index for `trie<T[k]>` — the SAME radix tree over ONE text key; content type nr + the key field index |

### Field struct

```rust
pub struct Field {
    pub name: String,
    pub type_nr: u16,    // index into Stores::types
    pub offset: u32,     // byte offset within the record
}
```

### Stores API

| Method | Description |
|---|---|
| `new() -> Stores` | Create empty stores; registers base types 0–6 |
| `structure(name) -> u16` | Register a new struct type; returns its index |
| `field(type_nr, name, field_type, offset)` | Add a field to an existing struct type |
| `enumerate(name) -> u16` | Register a new enum type |
| `value(enum_nr, discriminant, name)` | Add a variant to an enum type |
| `finish()` | Seal schema registration (calculates sizes, alignment) |
| `allocate() -> u16` | Create a new `Store`; returns its index |
| `store(nr) -> &Store` | Borrow store by index |
| `mut_store(nr) -> &mut Store` | Mutably borrow store by index |
| `byte(min: i32, nullable: bool) -> u16` | Register or get a byte integer type; name = `"byte"` for (0,false) or `"byte<min,nullable>"` |
| `short(min: i32, nullable: bool) -> u16` | Register or get a 16-bit integer type; name = `"short<min,nullable>"` |
| `database(size: u32) -> DbRef` | Allocate a new top-level store slot; `size=u32::MAX` means no record claim |
| `free(db: &DbRef)` | Release a top-level store slot (LIFO order required) |
| `null() -> DbRef` | Allocate an empty store slot (calls `database(u32::MAX)`) |
| `read_data(r, tp, little_endian, data)` | Serialize a stored value to raw bytes (for writing to binary file) |
| `write_data(r, tp, little_endian, data)` | Deserialize raw bytes into a stored value (from reading a binary file) |
| `lock_store(r: &DbRef)` | Lock the store that owns `r` (no-op for null refs) |
| `unlock_store(r: &DbRef)` | Unlock the store that owns `r` |
| `is_store_locked(r: &DbRef) -> bool` | Return whether the store that owns `r` is locked |
| `adopt_store(store) -> u16` | Install an externally-built `Store`; clears the slot's free bit |
| `take_store(slot) -> Store` | Move a `Store` out, leaving a freed sentinel — **does NOT release the slot** |
| `release_slot(slot)` | Give a slot borrowed by `adopt_store` back to the pool |

**`adopt_store` / `take_store` are not symmetric about the slot, on purpose.**
`take_store` is written for a store handed out to OUTLIVE the table — the REPL's
session store, adopted for a run and taken back afterwards — where the slot
should stay reserved. So it leaves the free bit CLEAR, and `find_free_slot` only
ever returns a slot whose bit is SET. A caller borrowing a slot as **scratch**
must therefore call `release_slot`, or the slot number is burned for the life of
the process.

That leak is invisible from two places you would look: `store_memory()` counts
only LIVE stores and a freed sentinel is not one, and `LOFT_STORES=log` does not
trace this allocation path. It was found by reading the pair rather than by any
probe (@PLN123 B2, where compaction borrows a scratch slot on every load), and
`slot_recycling_tests` in `src/database/mod.rs` pins both halves so the
asymmetry stays recorded.

### The value stack lives in ONE record, and that record has to grow with it

Store index `0` is the interpreter's value stack. It is a single claimed
record — `PRIMARY`, record 1 — and every frame slot is addressed as
`(0, 1, pos)`. Frame writes go straight through `addr_mut`; they never call
`claim`, so the normal growth path never runs and `State::ensure_stack` is what
extends the buffer when a program nests deeply enough.

**Growing the buffer is only half of it.** `Store::grow_words` extends the
allocation; record 1's header still claims the size it was born with (1000 words
= 8000 bytes), so every stack byte above that mark sits outside the record that
owns it. `ensure_stack` therefore calls `Store::extend_primary_to_store_end`
after every growth — the store's only record spans the whole store, which is the
invariant `State::new` established and nothing since maintained (loft#935).

`Store::resize` is the wrong tool here and must stay unused on this store: its
fallback is claim-copy-delete, which RELOCATES the record. Record 1 IS the
running stack, so moving it moves every live frame out from under the
interpreter.

The failure this caused is worth remembering for its shape rather than its
cause: a consumer added a `vector<Struct>` local to a ~700-line dispatcher and
got `realloc(): invalid next size` — a glibc heap abort — in a *different* test
file, one that never called the edited code. The enclosing function's size was
never the defect; it was what made the frame big enough to cross the initial
claim. That is also why the shape resisted every attempt to shrink it: the axis
it needed was stack DEPTH, and a matrix that varies the expression while holding
the function fixed cannot reach it. `tests/scripts/935-stack-store-growth.loft`
is the depth axis in fifteen lines.

### Constant store (`CONST_STORE`)

Store index `1` is reserved for compile-time constant data:

```rust
// src/database/mod.rs
pub const CONST_STORE: u16 = 1;
```

Allocated by `State::new()` immediately after the stack store (index 0)
and before any runtime store.  Populated during `byte_code()` and
**locked** before `execute()` runs.

| Index | Purpose | Allocated in |
|---|---|---|
| 0 | Stack store (evaluation stack, record in store 1000 historical alias) | `State::new()` |
| 1 | **Constant store** (read-only data) | `State::new()` |
| 2+ | Runtime stores (structs, vectors) | `OpDatabase` at runtime |

**What lives in `CONST_STORE`** (@PLN82 Phase A, 2026):

- **Vector constants** — file-scope `QUAD = [1, 2, 3];` is built as
  a vector record in `CONST_STORE` during `byte_code()`.  Each
  constant's `DbRef` is recorded in `Definition.const_ref` and
  cached in `State.const_refs[d_nr]`.  Closes P127 (Var-collision
  on inlined vector-literal IR).
- **Long string constants** (>= 256 bytes) — `Store::set_str()`
  copies bytes into `CONST_STORE`; `OpConstStoreText` reads the
  `Str` pointer at runtime.  Replaced the ad-hoc `text_code:
  Arc<Vec<u8>>` buffer that previously lived on `State`.
- Short strings (< 256 bytes) stay embedded inline in the bytecode
  via `OpConstText` — record-header overhead exceeds the inline
  format's 1-byte-prefix cost at small sizes.

**Reference-site codegen** for vector constants:

```text
__cv = null
OpDatabase(__cv, vec_tp)             # allocate fresh runtime store
OpConstRef(d_nr)                     # push the constant's DbRef
OpCopyRecord(const, __cv, tp)        # deep-copy into __cv's store
return __cv                          # caller owns __cv (mutable)
```

Each reference site allocates a fresh runtime store and deep-copies
the constant record in.  Mutations to the copy never affect the
original; the copy participates in normal `OpFreeRef` lifetime.

**Lifetime + safety**:

- `CONST_STORE` is **never freed** — persists for the program's lifetime.
- **Locked** after construction (`store.locked = true`) — writes panic
  in debug, are no-ops in release.
- No `OpFreeRef` for `CONST_STORE` — it has no runtime refcount.
- Parallel workers may read the locked store directly without cloning
  (read-only = thread-safe).
- Excluded from the debug-mode "Database N not correctly freed" exit
  check — expected to remain allocated.

See [INTERMEDIATE.md § Bytecode State](INTERMEDIATE.md#bytecode-state--srcstate)
for `State.const_refs`'s role in `OpConstRef` dispatch.

For deferred follow-ups (mmap-backed cache file; WASM
pre-compiled stdlib including `CONST_STORE` as static bytes via
`include_bytes!`) see
[`plans/82-const-store/`](plans/82-const-store) §
Memory-mapped + WASM fast startup.

### Store Locking via `Stores`

`Stores` exposes three methods that wrap the per-`Store` lock flag:

```rust
pub fn lock_store(&mut self, r: &DbRef)       // enable write-protection
pub fn unlock_store(&mut self, r: &DbRef)     // remove write-protection
pub fn is_store_locked(&self, r: &DbRef) -> bool
```

All three methods silently ignore null refs (`r.rec == 0`) and out-of-range store indices so they are safe to call unconditionally from generated code.

These methods are surfaced to loft code via two native functions registered in `src/native.rs`:

| Native function | Loft declaration (`default/01_code.loft`) |
|---|---|
| `n_get_store_lock` | `fn get_store_lock(r: reference) -> boolean` |
| `n_set_store_lock` | `fn set_store_lock(r: reference, locked: boolean)` |

The `reference` parameter type accepts any concrete `Reference` type at call sites thanks to the type-compatibility check in the parser.

### `d#lock` Syntax

Loft code interacts with store locks through the `#lock` pseudo-field syntax:

```loft
c#lock        // read: boolean — true if the store is locked
c#lock = true // write: lock the store
```

**Parser routing** (`src/parser/collections.rs` and `src/parser/expressions.rs`):
- `iter_op` detects the `lock` keyword and emits `n_get_store_lock(c)` for reads.
- `towards_set` converts a `n_get_store_lock` call into `n_set_store_lock` for the left-hand side of an assignment.
- `parse_assign` validates the assignment: only a literal `true` or `false` is accepted (not an expression); assigning `false` to a `const` variable or argument is a compile-time error.

**Constraints enforced by the compiler**:
1. `d#lock` is only valid on `Reference` or `Vector` typed variables; any other type is a diagnostic error.
2. The right-hand side must be a constant boolean (`true` or `false`).
3. `d#lock = false` on a `const` variable is a compile-time error.

### `const` Variables and Arguments

The `const` keyword can be applied to local variable declarations and function arguments:

```loft
const d = Counter { value: 42 }   // local const variable
fn read_value(self: const Counter) // const argument
```

**Semantics**:
- The compiler marks the variable with `const_param`, preventing reassignment via `OpSet` in generated bytecode.
- In **debug builds only** (`#[cfg(debug_assertions)]`): the store is automatically locked immediately after initialisation (local `const`) or at the start of the function body (const arguments). This turns any accidental write into a runtime panic.
- In **release builds**: the lock is _not_ set automatically; only explicit `d#lock = true` in loft code locks the store.
- Reading `c#lock` on a const variable emits a runtime `n_get_store_lock` call. In a debug build this always returns `true` because the store was auto-locked; in release it returns whatever the current flag is.

**Implementation locations**:
- Auto-lock for local `const`: `expression()` in `src/parser/expressions.rs` — after the initialising assignment is compiled, inserts a `n_set_store_lock` call under `#[cfg(debug_assertions)]`.
- Auto-lock for const arguments: `parse_code()` in `src/parser/expressions.rs` — inserts lock calls at the start of the function body for every argument that is both an argument and const.

### Binary File I/O: `read_data` and `write_data`

`read_data` reads from a `DbRef` into a `Vec<u8>` (for writing to a binary file). `write_data` reads from a `&[u8]` into a `DbRef` (for reading from a binary file).

**Critical design constraint**: temp variables used for file I/O (created by `write_to_file` / `read_from_file` in `parser.rs`) are **always stored as full i32 on the stack** (`Context::Variable` always allocates 4 bytes for all integer types). This means `read_data`/`write_data` for `Parts::Byte` and `Parts::Short` must use `get_int`/`set_int`, NOT `get_byte`/`get_short`.

The reason: `get_short(rec, pos, min)` reads the null-sentinel-encoded storage (`stored_u16 = value − min + 1`) and returns the actual value. But a temp var's slot holds a raw i32 (no encoding offset). Using `get_short` on an i32 temp var returns `raw_u16 − 1`, which is off by one.

| Part type | `read_data` (store → bytes) | `write_data` (bytes → store) |
|---|---|---|
| `Base(0)` / `Base(6)` (integer/char) | `get_int` → 4 bytes | `set_int` from 4 bytes |
| `Base(1)` (long) | `get_long` → 8 bytes | `set_long` from 8 bytes |
| `Base(2)` (single) | `get_single` → 4 bytes | `set_single` from 4 bytes |
| `Base(3)` (float) | `get_float` → 8 bytes | `set_float` from 8 bytes |
| `Base(4)` (boolean) | `get_byte(_, _, 0) as u8` → 1 byte | `set_byte(_, _, 0, data[0])` |
| `Base(5)` (text) | `get_str` → UTF-8 bytes | `set_str` from UTF-8 bytes |
| `Parts::Byte(_, _)` | `get_int` → truncate to u8 → 1 byte | `set_int(i32::from(data[0]))` |
| `Parts::Short(_, _)` | `get_int` → truncate to i16 → 2 bytes | `set_int(i32::from(i16::from_le/be_bytes))` |
| `Parts::Struct(fields)` | recurse for each field | recurse for each field |
| `Parts::Enum(_)` | `get_byte` → 1 byte | `set_int(i32::from(data[0]))` |
| `Parts::Vector(elem_tp)` | iterate elements, recurse per element | `vector_append` + `write_data` per element + `vector_finish` |

**Note**: `Parts::Byte`/`Parts::Short` in `read_data`/`write_data` are designed for temp variable contexts (i32 layout). Using these with actual 1/2-byte struct fields would produce incorrect results. Struct serialization via `Parts::Struct` recursion is not yet fully tested.

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

### Sorted By-Value Vector (`Parts::Ordered`)

Same record layout as Vector. Elements are kept in sorted order via binary search insertion.

| Function | Description |
|---|---|
| `sorted_find(store, rec, size, keys, vals) -> (u32, bool)` | Binary search; returns (byte_offset, found) |
| `sorted_add(store, rec, size, keys) -> u32` | Append then insertion-sort to correct position; returns offset |
| `sorted_finish(store, rec, size, keys)` | Insertion-sort the last added element into correct position |

### By-Reference Array (`Array` / `Parts::Array` / `Parts::Sorted`)

Stores 4-byte record references (offsets into a separate store) rather than inline data. Used for `sorted<T>` where `T` is a struct stored elsewhere.

| Function | Description |
|---|---|
| `ordered_find(store, rec, ref_store, keys, vals) -> (u32, bool)` | Binary search over references; dereferences into `ref_store` for comparison |
| `array_add(store, rec, ref_rec) -> u32` | Append a reference; returns slot offset |
| `array_remove(store, rec, pos)` | Remove reference at slot `pos`; shifts remaining |

---

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
| `find(store, root, keys, vals) -> (u32, bool)` | Search; returns (rec, found) |
| `add(store, root, rec, keys) -> u32` | Insert `rec`; rebalances; returns new root |
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

Collision resolution is **linear probing**: on collision, advance slot index by 1 (wrapping). The load factor threshold is:

```rust
(length * 2 / 3) + RESERVED_WORDS >= room
```

which is `length >= 0.75 * elms`. When it is met after an insertion, the table is rehashed
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
                            ├── Sorted/Index    → src/tree.rs  (+ src/vector.rs for Ordered)
                            ├── Hash            → src/hash.rs
                            ├── Radix           → src/radix_tree.rs + src/radix_db.rs
                            ├── Trie            → src/radix_tree.rs + src/trie_db.rs
                            └── Key comparison  → src/keys.rs
```

- A `sorted<MyStruct>` is a red-black tree in one `Store`; the node records also contain the user data fields (the tree fields are appended after the user fields at offset `fields`).
- A `hash<MyStruct>` is a hash-table record in one `Store` pointing to element records in another (or the same) `Store`.
- An `index<MyStruct>` combines both: the same element records are simultaneously in a red-black tree (for range queries and ordered iteration) and a hash table (for O(1) lookup by key).
- A `vector<T>` is a single record with inline elements; a `sorted<T>` by value uses the same layout but maintains sort order via insertion sort on add.
- All cross-record pointers are `u32` rec offsets within the same `Store`; cross-store references use the full `DbRef`.

---

## See also
- [REMOTE_STORES.md](REMOTE_STORES.md) — reading a store image over HTTP range: serving a large
  immutable dataset as a static file and fetching only the pages a lookup touches
- [INTERMEDIATE.md](INTERMEDIATE.md) — Value/Type enums in detail; 233 bytecode operators; State layout
- [INTERNALS.md](INTERNALS.md) — calc.rs, stack.rs, create.rs, native.rs, ops.rs, parallel.rs, radix_tree.rs
- [DESIGN.md](DESIGN.md) — Algorithm catalog with complexity analysis for hash, index, sorted, store
