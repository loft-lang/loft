<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Serving static data over HTTP range

How a loft program reads **part** of a large file that lives on a web server,
fetching only the bytes a lookup touches instead of downloading the whole thing.

The shape this is for is ordinary and not map-specific: **one big immutable
dataset, many small reads, no server-side code.** A game's world chunks, meshes
and sounds; a product catalogue; a gazetteer; a dictionary; a parts list; a
sensor archive; a map block — anything you can build once, upload as a static
file, and then query. The dataset sits on whatever serves static files (a CDN,
S3, GitHub Pages, nginx), and the client pulls pages of it on demand.

**No database server and no API.** The file is the API; HTTP range is the
protocol; the server is a dumb file host. That is the whole trade this buys.

## Games: asset streaming without an asset server

This is the shape a game already has, and loft is games-first, so read the rest
of this document with that case in mind. **World chunks, meshes, textures,
sounds, animations, dialogue, level data** — built once by a tool, packed into a
store, uploaded as a static file, and pulled in as the player moves.

```loft
// The world is one 2 GB pack on a CDN. Load the chunks around the player.
chunks: hash<Chunk[cell]> = [];
_ = store_load_keys(chunks, "https://cdn.example/world.store", ring_around(here));
```

What it replaces is an asset server, a streaming service, and the deployment that
goes with them. A browser game built with `--html` gets the same call: the world
streams from static hosting, and the program is written as though the pack were
on local disk.

Three things follow from the cost model below, and they are the ones that decide
whether a game feels good:

- **Pack by locality, because a page is shared.** Neighbouring chunks in the same
  64 KiB page arrive for the price of one; an animation's frames, a mesh and its
  LODs, a sound and its variants all want to be adjacent in the file. This is a
  property of how the pack is BUILT, and it is worth more than any runtime knob.
- **Prefetch ahead of the player, never on demand at the point of use.** A round
  trip is ~40 ms and you cannot take one inside a frame. Load the ring around the
  player, not the cell under them.
- **Batch the ring.** `store_load_keys` opens the reader once and reuses its page
  cache, so a ring is one traversal rather than N — `store_load_keys_text` is the
  same call for a pack keyed by NAME, which is what an asset pack usually is
  (`"page/mobs"`, `"hit.ogg"`). Looping the single-key form instead re-fetches the
  bucket-table page once per key: measured on a 447 KB pack, 25 keys cost 5.37 MB
  in 82 requests looped against 256 KB in 4 batched — the loop downloads the whole
  file twelve times over (loft#1064). ⚠ **Quote the measurement, not the ratio.**
  That corpus gives 20×, but the factor is entirely a function of how the keys fall
  across pages: a second measurement on a 5.1 MB pack of 80 blobs of exactly 64 KiB
  — close to worst case for bucket sharing — gave 3.4× (20 keys: 81 requests /
  5 308 416 bytes looped against 24 / 1 572 864 batched). The sentence that travels
  between corpora is not a number but the shape: *the loop fetches more than the
  whole pack to deliver a fraction of it, and the batch fetches roughly the
  payload.* `store_load_range` does the same for a
  contiguous span of ordered keys, and `store_load_box` fetches a viewport's worth
  of a `spatial` index in one traversal.

Split the pack the way the reads split: assets needed on **every** frame from the
first (the UI atlas, the player mesh, core sounds) are small and universal, so
load them **whole** at boot with `store_load_url_trusted`. The world is large and
read sparsely, so page it. Mixing those two in one file makes the boot load pay
for the world.

### An asset pack is a store, not a bundle (@PLN146 arc F)

The obvious first pass for the browser is a `--html` flag that bundles every
referenced file into the page — the Flash `[Embed]` shape. **Do not build that.**
The route above already exists and is better: the pack IS a loft store, hosted on
any dumb file server and read by range, so the struct definition is the file
layout — no codec, no parse step, no serialize seam. The `assets` package carries
the schema, `pack_write` / `pack_read`, `keys_near` / `prefetch`, the layout
fingerprint and the alpha-derived collision proxy.

Two constraints carry over from the `routing` project, which shipped this for map
tiles first:

- **Plan → fetch → read, never fetch-on-miss inside a frame.** Synchronous wasm
  cannot await, and a frame blocking on a range read stutters visibly. Ask for
  assets at load or level boundaries, or as a ring around the player.
- **Verify the layout fingerprint across native and wasm before anything reads a
  pack.** A silent divergence turns every asset into garbage at a byte offset,
  which reads as a corrupt file rather than as a layout bug. The fingerprint is
  pinned in `assets`, which is what makes a format change say so.

Embedding stays for the bytes a page needs before its first fetch — a boot font, a
loading sprite. The exception, not the pipeline.

⚠ The two halves accept **different URL spellings**: the metadata half reads a
`file://` URL and the paged half refuses one. One base would therefore have loaded
a game's scenes and silently none of its art.

## The two ways to read a remote store

| | fetches | use when |
|---|---|---|
| `store_load_url_trusted(r, url)` | the **whole** image, once | it is small, or you need all of it anyway |
| `store_load_url(r, url, sha256)` | the whole image, **pinned** to a hash | the origin is not trusted |
| `store_load_key(local, url, key)` | only the pages that ONE lookup touches | the dataset is large and reads are sparse |
| `store_load_key_text(local, url, key)` | same, for a text-keyed hash or trie | |
| `store_load_keys(local, url, keys)` | several keys, reader opened once | a batch — cheaper than N calls |
| `store_load_keys_text(local, url, keys)` | same, for a text-keyed hash or trie | an asset pack keyed by name |
| `store_load_range(local, url, lo, hi)` | a contiguous key range | ordered collections |
| `store_load_prefix(local, url, pre, n)` | a text prefix, capped at `n` | a search box over a `trie` |
| `store_load_box(local, url, from, till, n)` | a bounding box, capped at `n` | a map viewport over a `spatial` |

`store_load_prefix` is the one a phone typing into a search box wants: a laid-out
vocabulary trie answers `"kerk"` in about **3.8 pages** — a root→leaf descent plus
the records it returns — where the alternative is downloading the whole 5.9 MB
image (@PLN134). The cap bounds the WALK, not just the answer, so asking for 8 of
459 matches fetches 8 records' worth of pages; a negative cap means no limit and
on a common prefix reads the whole run.

`store_load_box` is its counterpart for a map: the corners are `vector<integer>`,
so one call serves 1, 2 or 3 axes. Measured on a 3.2 M-point index (a 158 MB
image), a 200-marker viewport costs **5.3 pages** cold and about **1.5** for each
pan after it (@PLN136). Two things bound it, and a map needs both — the cap bounds
the walk as above, and so does the BOX: a spatial index is ordered by Morton code,
the code interval between two corners is a superset the curve threads in and out
of, and on a wide, shallow viewport that superset is 1.46 M records for 4 k of
answer. The walk seeks over each excursion rather than reading it.

A THIRD thing decides the cost and this call cannot supply it: the order the file
was written in. The Morton walk is symmetric; a file laid out along one axis is
not, so a box crossing that axis pays for every stride it crosses. On a
62 500-point grid returning the same 500 records, a 250x2 box read **107 pages**
against **7** for its 2x250 mirror — and the two swapped when the identical data
was written in the other axis order, which is what proves the file rather than
the query is the cause. At 107 pages the read pulls 81 % of the image to return
0.8 % of the records, worse than fetching the whole file once. Write a dataset
meant for viewport reads **tiled**: it has no bad orientation, and for the
square-ish boxes a viewport actually uses it beats either linear order.

The six paged forms take **a local path or an `http(s)://` URL, interchangeably**,
on every target including the browser (`--html`). Nothing in the program changes
when the data moves from disk to a CDN — which is what makes "develop against a
local file, ship against a URL" work.

There is a third way, for when the program should not have to say which entries it
wants: `store_bind_lazy` binds a collection to one of these sources once, and every
lookup that MISSES fetches its own entry ([STDLIB_FILES.md § Lazy store
binding](STDLIB_FILES.md), `@F108`). Same reader, same pages — driven by the lookups
instead of by a call.

That binding also takes a source these functions do not: **`sqlite:<path>`**, where
the entries are rows in a relational table rather than pages of a loft image, and
the `SELECT` is derived from the collection's own type
([LAZY_STORES.md](LAZY_STORES.md)). Worth keeping the two apart while reading this
document: everything below is about an image, whose bytes are loft's own, so `len`,
ordering and identity are correct by construction. A foreign table owes loft none of
that, which is why the database source is read-only and refuses anything it cannot
derive rather than approximating it.

```loft
// The whole dataset is 800 MB on a CDN. This reads a few pages of it.
parts: hash<Part[id]> = [];
if !store_load_key(parts, "https://cdn.example/catalogue.store", 91_337) {
  println("no such part");
  return
}
```

`local` starts EMPTY and ends holding only what was asked for. The entry's own
fields are **relocated** into the local store, so `text`, nested structs and flat
vectors all come across. A `vector<text>` or a `vector<vector>` is refused — its
element pointers would dangle — and the refusal says so on stderr rather than
looking like an absent key.

## What the server has to do

Almost nothing, which is the point. Two requirements:

1. **Answer `Range: bytes=<off>-<last>` with `206 Partial Content`.** Every
   static file server does this by default; it is the same mechanism video
   seeking uses.
2. **Report the file's total size**, in either `Content-Range: bytes 0-0/TOTAL`
   or `Content-Length`. loft discovers the size with a `bytes=0-0` probe before
   its first real read.

A server that **ignores** `Range` and returns `200` with the whole body still
produces **correct answers** — loft reads and discards the prefix to reach the
offset. It is just slow, and slow in a way that grows with the offset. That
fallback exists so a misconfigured host is a performance bug, not a wrong result.

There is also an optional `<url>.dschema` sidecar beside the store (the
self-describing layout descriptor). A **404 is a normal answer** meaning "no
sidecar", not an error.

## Paging, and the knob that matters

Reads are served through a page cache, not issued one-per-lookup:

| | default | env override |
|---|---|---|
| page size | **64 KiB** | `LOFT_PAGE_BYTES` |
| cache size | **the load's working set** | `LOFT_PAGE_CACHE_BYTES` |

**The cache holds what the load reads, and is not bounded by default — on every target.**
An eviction costs a RE-FETCH, and that is true whatever the transport, so never evicting is what
removes a 3.6× read amplification (loft#785).

A wasm-only cap was tried and measured worse (loft#787). The reasoning was that a browser
eviction costs only a copy out of a buffer the host already holds, while linear memory is the
scarce resource — every step true, and the conclusion still wrong, because it priced the eviction
and not the re-fetch after it: 4 MiB against a 5.6 MB working set took range reads from 71 to 115
and bytes from 5.7 to 8.5 MB, to save 1 MB of a 17 MB heap.

**What DOES split per transport is prefetching, not retention.** `warm` exists to turn N round
trips into one, so a transport whose `fetch_many` is sequential (wasm, no threads) declines it
rather than paying the computation for a saving it cannot collect.

A page dropped before the walk reaches it costs a second fetch of the same bytes, so residency is
what stops a load paying twice — and a keyed load's working set is the data you asked for, a local
copy of which is being materialised anyway. Peak memory is about the size of the result, not a
constant.

`LOFT_PAGE_CACHE_BYTES` turns it into a **hard cap** for a memory-bounded host. Expect
re-fetching once the cap is below the working set: the bytes have to come back somehow, and
that is the price of the bound. It was the default until loft#785, where 4 MiB against a
20 MB working set fetched **3.6× the bytes** — the prefetch evicting the very pages it was
about to read.

### What a single read costs

Two guarantees, because a paged traversal performs them by the hundred thousand — one
viewport in a real consumer issues ~800 000 four-byte index reads (loft#783):

| a resident read of 1–8 bytes | |
|---|---|
| allocations | **0** |
| page-map lookups | **1** |

Neither was true before loft#787. `resolve` returned an owning `Vec`, so a four-byte index
word cost a malloc and a free; and the read hashed its page key **twice**, once to ask
whether the page was resident and once to index it. Both are per-READ costs, both are
invisible in a profile that attributes by function, and both hid completely on native builds
because glibc's per-thread cache serves a 4-byte request in ~15 ns. In a browser — dlmalloc
on the linear heap, no thread-local fast path — the same code was 1.14× slower on a keyed
paged load while issuing FEWER requests and fewer bytes, which is the signature of a cost
that scales with reads rather than with round trips.

Measured end-to-end by the consumer that reported it, one viewport, medians, arms
interleaved:

```
pre-arc baseline    169 ms / 763 ms (CPU 1x / 4x)   411 reads   26.9 MB
as filed            254 ms / 1018 ms                369 reads   26.7 MB
fixed               173 ms /  694 ms                369 reads   26.7 MB
```

Faster than before the work started, with 42 fewer reads. `tests/paged_read_alloc.rs` pins
both counts, asserting them as SCALING properties (the same range read 200 and 300 000 times
must cost the same) so a legitimate per-page or per-record allocation can never be mistaken
for a per-read one. The span read that relocates a record body still owns one buffer — per
RECORD, which is the right unit.

Every fetch is one page-aligned range. **A page is the unit of waste and the unit
of amortisation**: a 12-byte record costs a 64 KiB fetch, and so do the next
thousand records beside it. That is why *what sits next to what* in the file is
the thing to tune — not the page size.

Read this the other way round when you are designing the dataset: records that
are read together should be **written** together. A store whose records are
ordered by the key you query is a store where one page answers many lookups; the
same records in insertion order can cost a fetch each.

## The cost model — round trips, not bytes

The number to think about is **round trips**, and it is easy to get this backwards.
Measured against a live CDN:

```
1 B      41 ms      ← a 64 KiB range costs the SAME as one byte
64 kB    41 ms
512 kB   85 ms
2 MB    221 ms
```

A range request is dominated by its round trip until it gets big. So:

- **Fewer, larger requests win.** Two adjacent pages fetched separately cost
  roughly twice one fetch that covers both.
- **Byte-shaving does not help** while you are latency-bound. Fetching *more*
  bytes in fewer requests is routinely faster.
- **Serial depth is the enemy.** N dependent lookups cost N round trips no matter
  how small each one is. `store_load_keys` exists for exactly this — it opens the
  reader once and reuses its cache, so a batch is not N independent reads.

If a cold read feels slow, count the requests before you count the bytes.

**In the browser the ranges still go one at a time.** Native builds issue a batch's
independent ranges concurrently, so a batch costs about one round trip. A browser has no
threads, and the bytes arrive through a synchronous host bridge, so `--html` spends one
round trip per range — correct, and as fast as it was before batching existed, but the
concurrency win is not there. Budget browser paging by round-trip *count*, and lean harder
on fetching fewer, larger ranges.

**If you run a local Range server for testing, raise its listen backlog.** Concurrent
ranges arrive together, and a small backlog drops the excess SYNs — each dropped one costs
a ~1 s TCP retransmit, so a batch that should take 30 ms takes over a second and reads as a
loft problem. Python's `socketserver` defaults to a backlog of **5**; set
`request_queue_size = 128`. Measured on a real harness: 1066/1285/1075 ms at the default,
27–40 ms at 128.

## Choosing whole vs paged

Paging is not automatically better, and a small file is the case where it loses:

- A dataset of a few MB that is **always needed in full** is one request whole,
  and several requests paged. Use `store_load_url_trusted`.
- A dataset that is large, or needed **sparsely**, is what paging is for.
- The choice is a property of **how you read**, not of how the file was written.
  The same `.store` serves both, so switching is a code change and not a
  re-upload.

That last point is worth leaning on. Because the file never changes, anything
*derived* from it — a prefetch list, a key→page map, an ordering experiment — is
cheap to publish, cheap to revise, and cannot corrupt the dataset. Derived data
over an immutable artifact is the cheap thing to iterate on; the artifact is not.

## Failure modes worth knowing

- **A short read at EOF is not an error.** A range that overhangs the end returns
  what exists; the paged reader tolerates a short tail by contract.
- **An absent key returns `false`**, not a fault. So does an unreadable file and a
  collection that is not the right kind — check the boolean.
- **A truncated or malformed image is REJECTED, never adopted.** Both URL loaders
  validate structure; `store_load_url` additionally pins the SHA-256.
- **`store_verify(local)` proves the load produced a sound heap**, which is worth
  asserting after a paged read from an untrusted origin.
- **Native builds need the bytes-fetching feature** — `remote-store` (the paged
  loaders' own) or `registry`. Without either, a URL is a clear "unavailable"
  error rather than a silent wrong answer.

## Targets

The loft-visible call is identical everywhere; only the byte source differs
(`src/net.rs` is where that difference is confined).

| target | how the bytes arrive |
|---|---|
| native | `ureq`, plus the `file://` offline path |
| browser (`--html`) | the `loft_host_http_range` host import |

The browser case is the interesting one: JS `fetch()` is async and a synchronous
call would deadlock a single-threaded event loop, so the import is on the
wasm-opt `--asyncify` allowlist. Calling it unwinds the wasm stack back to the
event loop, the shell awaits the fetch, then rewinds with the bytes. **The loft
call returns synchronously and the page never freezes** — the program is written
as if the data were local.

## See also

- [STDLIB.md](STDLIB.md) — the `store_load*` family in the stdlib reference.
- [DATABASE.md](DATABASE.md) — stores, and what an image contains.
- [WASM.md](WASM.md) / [BROWSER_INTEROP.md](BROWSER_INTEROP.md) — the browser bridge.
- `src/net.rs` — the per-target fetch; `src/paged_reader.rs` — the page cache.
