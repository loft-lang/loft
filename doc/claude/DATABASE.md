
# Database and Storage Layer

## Overview

The runtime data layer is split across multiple source files that together implement a typed, heap-allocated, store-based memory model:

## Contents
- [Overview](#overview)
- [Store — Raw Heap Allocator (`src/store.rs`)](#store--raw-heap-allocator-srcstorers)
- Stores — Type Schema + Multi-Store Manager (`src/database/`) — [DATABASE_STORES.md](DATABASE_STORES.md)
- [DbRef, Key, Content — Universal Pointer and Key Types (`src/keys.rs`)](#dbref-key-content--universal-pointer-and-key-types-srckeysrs)
- [Vector Operations (`src/vector.rs`)](#vector-operations-srcvectorrs)
- Red-Black Tree, Open-Addressing Hash Table, Spatial Index, Text Trie — [DATABASE_INDEXES.md](DATABASE_INDEXES.md)
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

Every path that reads an EXISTING image through the program's types takes this gate
(`@FR-L-Sound`): `store_load`, `store_load_untrusted`, `store_persist_bind` on an
existing file, the whole-image URL loaders `store_load_url` / `store_load_url_trusted`
(the sidecar is `<url>.dschema`, over the same transport, and a missing one is not
checked), and the working-set loaders above. Each refusal names its own builtin. A
refused bind leaves the file, its `.dschema` and the collection as they were. That
matters because the bind WRITES the sidecar with the binding program's layout when it
succeeds — so a bind that accepted a mismatched file would also relabel it, and every
later load would then read the misread store as matching (loft#1562). The whole-image
paths share one verdict, `Stores::layout_verdict_ok`, and a new whole-image loader asks
it rather than growing its own; the working-set loaders refuse through `refuse_paged`,
so the program can read their refusal back.

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
opt-in for a reason ([STDLIB_RUNTIME.md § Memory diagnostics](STDLIB_RUNTIME.md)): on a churning
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
| `fast_order(keys, key) -> Option<FastOrder>` | how it ORDERS (`compare`), direction applied once (@FR-Col-Order-Sign) | `tree::find`, `tree::find_exact`, `vector::sorted_find`, `vector::ordered_find` |
| `fast_order_of(rec, stores, keys)` | the same, for a record's own key | `tree::add`, `sorted_finish`, `ordered_finish` |

All four answer `None` for a compound or partial key and for a width they do not list
(`u8`, `u16`, `single`, `float`), and the search then takes the general comparator
unchanged.  Every search calls ONE of `keys::order_key` / `keys::order_record`, which is
also where `LOFT_KEYED_VERIFY=1` checks the fast answer against the general one.  A text
key cannot be held across `tree::put` (it rebalances the store the text lives in), so
`FastOrder::detached` drops it there and a text-keyed `index` INSERT orders generally.
`LOFT_NO_FAST_ORDER=1` makes the order forms answer `None` everywhere.

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

**Red-black tree, hash table, spatial index, text trie** — see [DATABASE_INDEXES.md](DATABASE_INDEXES.md).

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
