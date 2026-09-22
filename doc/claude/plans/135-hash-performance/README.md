<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 135 — A hash insert that allocates once and never re-hashes

## Status

> **2026-09-21 — that verdict is about the 1 M-entry regime, and only that one.**  At
> 5,000 entries — cache-resident, which is where the keyed routines of every surveyed
> consumer run — a lookup is bound by PATH LENGTH (924 instructions against a Rust
> `HashMap`'s 177), not by the two memory accesses this plan ends on.  That regime was
> analysed and four levers built under @PLN158: `bench/portal/analysis/keyed.md`.
> Nothing below is withdrawn; it measures a different question.

**CLOSED (2026-08-10) — arcs A, B, C, Q2 and H all shipped, and nothing further is
worth building.** H is in the tree and measured, and the measurement corrects this
plan's central prediction: see
[§ What H actually bought](#what-h-actually-bought-2026-08-10--insert-and-bytes-not-lookup).
Read that section before re-deriving anything from the Q1 / Q5 numbers below, which are
correct as measurements and were over-read as a forecast. The two items loft#809 still
lists as open were then measured too, and neither earns its cost —
[§ What is LEFT after H](#what-is-left-after-h-measured-2026-08-10--and-why-none-of-it-pays).

| | before | after | |
|---|---|---|---|
| insert 1M, reserved | 330 ms | **258 ms** | **1.28x** |
| store bytes / entry | 27.67 B | **18.6 B** | **−33%** |
| claimed records, 2000 entries | ~2000 | **9** | table + directory + 6 chunks |
| random lookup, 1M | 184 ns | 183 ns | **unchanged** |

**What H must be built as changed on 2026-08-09, and the reason was never the byte
layout.** Drafted as one contiguous array, H moves every entry when it grows and
invalidates every outstanding reference into it — a stability callers have today and would
lose (**Q5**), which is an ownership question in the area ranked weakness #1, not plumbing.
**Q5 is measured and answered: a CHUNKED arena keeps the win** — 86 ns at 64K entries/chunk against a dense array's 77 and today's 200, and that
is an upper bound (see [§ The Q5 measurement](#the-q5-measurement-2026-08-09--chunking-keeps-the-win)).
So H can keep every live entry at the address it was handed out at, which is what makes it
compatible, without giving up the locality it exists to buy. **Q4** (load factor) is
settled inside H.

**Q1 — answered: H, not D.** (And the ANSWER was right while the forecast attached to it
was not — H's win is insert and bytes, not the lookup this paragraph predicted. See
[§ What H actually bought](#what-h-actually-bought-2026-08-10--insert-and-bytes-not-lookup).) See
[§ The Q1 measurement (2026-08-09)](#the-q1-measurement-2026-08-09--it-is-one-access-and-it-is-a-byte-problem).
The record read is **82%** of a 1M random lookup and the bucket read is 8%, so D spends
the one format break available on the small term; H's ceiling (measured against a dense
`vector<Entry>`, which IS its layout) is roughly half of today's cost. **Retire D** unless
H plus the abandoned-table fix still leave a gap D would close.

**Q2 — shipped: a store written before a placement change now REFUSES instead of
misreading.** The @PLN97 layout identity commits to how bytes are shaped, not to where a
keyed collection puts an entry — so before this, a store written pre-H and read post-H
passed the gate and was then misread, with lookups finding nothing or a neighbour and no
error anywhere. `src/placement.rs` carries a token per collection KIND into that same
identity: `placement::tag` → `layout_dump` → `layout_algo_hash` → the `.dschema` sidecar →
`schema_gate_ok` on every `store_load`, with the paged and remote loaders gating on the
same value before range-reading foreign bytes. **Absence means the baseline**, so the dump
is byte-identical, no recorded layout hash moves, and not one store already on disk is
invalidated — which is why it could ship before H rather than with it. Per KIND, so
bumping the hash token leaves plain structs and vectors loading exactly as before.
`sorted` / `ordered` deliberately get no token: their placement is key order, which a
reader re-derives by comparing keys it can already read. Guards:
`placement_contract_is_pinned` (arc D and H move the bucket constants; arc E moves the
`key_hash` digests) and `a_changed_placement_token_refuses_the_store`.

Filed and fixed while pinning that contract: [loft#827](https://github.com/loft-lang/loft/issues/827)
— `key_hash` ran on `std::hash::DefaultHasher`, whose algorithm std does not promise across
releases, while the seed stored in the file makes every reader re-derive buckets from it.
So a toolchain upgrade alone could move placement, with no loft change and no token to
bump. `keys.rs` now owns a byte-identical `SipHasher13`, so `placement::HASH` does not bump
and no store is invalidated.

**The layout-neutral half is DONE (2026-08-09).** The same probes found that growing a
hash **abandoned every previous bucket table** — neither replacement site returned the old
claim, and `hash.rs` never called `Store::delete` at all. Fixed: `install_table` repoints
the field and frees the predecessor in one step. A grown 1M hash went **49.26 MB → 34.02
MB (−31%)**, within 3% of the 33.00 MB the same content costs pre-sized, with **no lookup
regression** (244 ns vs 252 ns min, 1M shuffled — the denser table offsets the longer
probe chains). Guard: `data_structures.rs::hash_growth_frees_the_table_it_replaces` counts
claimed records; `tests/scripts/135-hash-table-rebuild-frees-the-old-table.loft` is the
both-backend correctness half. **So arc H must now be measured against 34 MB, not 49.**

**Also measured:** arc E's lookup win is 1 ns of 36 (step 4), so **Q3 dissolves** — the
hash-DoS argument only ever existed to license E. **Q4 is NOT settled** by these probes.
And **lookup ORDER outweighs every arc**: 87 ns ascending vs 225 ns shuffled at 1M, and
every figure in this plan before 2026-08-09 was taken in insertion order.

Measured on `--native-release`, 1M `integer` keys, before → after A + B + C:

| | before | after | C# |
|---|---|---|---|
| insert | ~933 ms | **~505 ms** (~**350 ms** with `reserve`) | ~28 ms |
| lookup | ~90–101 ms | **~66–84 ms** | ~22 ms |
| bucket table | 10.2 MB | 10.2 MB (**5.3 MB** with `reserve`) | — |

The measurement below is the plan's foundation and was taken on one machine with both
runtimes present — it is not a cross-machine ratio. Re-read it before re-deriving
anything: two of #809's own claims did not survive it.

**Arc A's matrix found three key-width defects that had nothing to do with
performance** — the equivalence cells asked "does a `float` / `u8` / `i16` key behave
the same before and after?" and the answer was that none of them behaved at all. Two
are fixed here, one is filed. See [§ What the matrix found](#what-the-matrix-found).

## Goal

Close the integer-key `hash<T[key]>` gap to `Dictionary<long,long>`, starting with the
77% of it that is not the hash function and needs no store-format change.

## Effort + design

- **Effort:** MH (A: S · B: S · C: S · D: M · E: S · H: MH) — **only H remains**
- **Design:** ~ (Q1 answered → H; Q2 shipped; Q3 dissolved; Q5 answered → chunked arena, measured; **Q4 open**, settle it inside H — all 2026-08-09)
- **Value category:** Q (internal quality — performance with a clear payoff)
- **Last touched:** 2026-08-09

## The measurement (2026-08-08)

Ubuntu 24.04 x86_64, 24 cores · loft 2026.8.0 `--native-release` · .NET 9.0.316 ·
1M inserts + 1M lookups, `integer` keys. Corpus:
`Moros-Economy-Development/loft_eval/bench/hash_bench.loft` and a matched
`Dictionary<long,long>` port. The consumer's reported ~980 ms reproduces here.

| | loft | C# | ratio |
|---|---|---|---|
| insert | ~874–954 ms | ~28 ms | **~32x** |
| lookup | ~67–101 ms | ~22 ms | ~4x |
| total | ~980–1010 ms | ~50 ms | ~20x |

**Attribution — measured by timing `hash::add` from inside, not modelled:**

| term | cost | share of the 954 ms insert |
|---|---|---|
| `hash::add` in total (bucket table + SipHash + probing + all 17 resizes) | 215 ms | 23% |
| — of which resize re-hash (1.79M re-inserted entries) | 171 ms | 18% |
| — of which ordinary per-insert hash work | ~44 ms | 5% |
| **everything outside the hash** — scratch record + field writes + `OpCopyRecord` | **~740 ms** | **77%** |

Supporting figures:

- `vector<Entry>` inline append (same records, no hashing): ~100 ms/1M — so the hash
  table is not what makes an `Entry` expensive; the **scratch-plus-copy** is.
- 17 resizes; final load **0.42**; linear probing averages **1.37 probes** for both
  insert and hit. An initial ~8.5-probe estimate was falsified by simulating loft's own
  growth policy — probe length is not a cost here.
- SipHash in isolation: 26.5 ns/op through the `Content` slice; a seeded murmur
  finalizer: 2.36 ns/op. Bucket distribution of the two is equivalent (max bucket and
  empty-slot counts within noise across `elms` 16 … 2,097,150, and across strided key
  families).
- Arc E prototyped end-to-end behind an env flag: `hash::add` 215 → 121 ms, insert
  954 → 840 ms, lookup 88 → 67 ms, combined bench 982–1011 → 849–872 ms (~13%).
  Patch kept out of the tree; it moves bucket placement (see Q1).

**Candidates measured and REJECTED** — recorded so they are not re-proposed:

- *Drop the per-insert `keys.clone()`.* `structures.rs`'s `Parts::Hash` arm clones the
  `Vec<Key>` on every insert to dodge a borrow conflict, which reads like one heap
  allocation per insert. Replacing it with a split field borrow is a three-line change
  and measures **no effect** (insert ~583–600 ms vs a ~540–623 ms baseline — inside the
  noise). A one-element `Vec` clone is not what this path is spending on.
- *Give insert one hash instead of two.* Each insert hashes the same logical key twice
  through two derivations — `dedup_keyed` → `find` → `key_hash` (Content side), then
  `hash::add` → `hash_set` → `keys::hash` (record side). Collapsing them is layout-neutral
  and sound, but it is worth ~4%: per-insert hash work is ~44 ms of the 215 ms, the
  rest being resize. Fold it into arc D — which will already have the hash in hand — and
  do not spend a standalone arc on it.

**What the generated code actually does per `cache += Entry { key: i, val: i*2 }`:**

```rust
let elm = OpNewRecord(cell, var_hb, 78, 65535);   // claim the entry record
ref_1   = OpDatabase(cell, ref_1, 77);            // claim a SECOND scratch record
set_int(ref_1, 0, i); set_int(ref_1, 8, i*2);     // write the fields into the scratch
OpCopyRecord(ref_1, elm, 32845);                  // copy scratch -> entry
OpFinishRecord(cell, var_hb, elm, 78, 65535);     // link into the hash
```

`OpDatabase` is `clear` + `claim` (an LLRB free-space search) + `set_known_type` +
`set_default_value`. C# writes into `_entries[_count++]` — a bump-pointer store into one
contiguous array, no allocator involved, and its resize is an `Array.Copy` plus a
sequential re-bucket off the **cached** `hashCode`, so it never re-hashes a key.

**Two #809 claims the measurement contradicts** — recorded so they are not re-derived:

1. #809 frames this as a lookup problem. Lookup is 4x; **insert is 32x** and ~90% of the
   wall clock.
2. `hash.rs`'s comment says load factor 0.75. The *trigger* is 0.75; `room*2-1` then
   overshoots to 0.42. The tradeoff already spent is memory, not speed.

## Composition matrix — Stage A

Arcs A / B / D / E / H are **behaviour-preserving**, so the matrix is an *equivalence*
matrix, not a feature matrix: every cell asserts identical results before and after, on
both backends, plus no leak. Arc C adds a surface (a capacity hint) and needs a real
feature matrix.

Axes that must move together — the composition the fix touches:

| axis | cells |
|---|---|
| key type | `integer`, narrow (`u8` / `i32` / `i16`), `text`, `float`, multi-key |
| entry shape | scalar-only fields, a `text` field, a `reference` field, a nested collection |
| value expression | fresh struct literal (the retarget case), a variable, a function call, a copy of an existing record |
| collection op | `+=` insert, `[k]` hit, `[k]` miss, remove-then-reinsert, iterate |
| size | below first resize (< 12), across ≥ 2 resizes, ≥ 1M |
| persistence | in-memory, persisted then re-read **in a second process** |
| backend | `--interpret`, `--native` |

The persistence row is the one that catches D / E / H: a store written by the old
binary and read by the new one must **refuse loudly**, never miss silently. Probes go in
`probes/`, graduate to `tests/scripts/` per arc as each arc lands.

Arc A's matrix is `probes/a-retarget-matrix.loft`, graduated to
`tests/scripts/135-keyed-insert-retarget.loft`. Arc C's feature matrix (it adds a
surface, so it needs one) is `tests/scripts/135-reserve-hash.loft`: every cell fills a
reserved collection and its unreserved twin and demands they agree on length, on every
record, and on every miss. Arc B needs no matrix of its own — it changes how a
comparison is reached, not what it answers, and every keyed test in the suite is its
equivalence check.

## What the matrix found

The **key type** row of the matrix — `integer`, narrow, `text`, `float`, multi-key —
was written to prove the retarget changed nothing. It found instead that three of
those key widths had never worked, silently, on shipped loft. All three predate this
plan (verified against a pristine `HEAD` worktree, and on insert paths arc A does not
touch), and all three lose records without a diagnostic:

| key width | symptom | status |
|---|---|---|
| `float` / `single` | `keys::get_key` had no arm, so it read ONE BYTE and called it a `Content::Long`. Every key whose low byte is zero — 0.5, 1.5, 2.0, … — became the SAME key: a `sorted<T[k]>` collapsed to its last insert, a `hash<T[k]>` probed a bucket `hash_ref` never fills. Both backends. | **fixed here** |
| `u8` / `u16` / `i32` / `u32` | the native generator's `emit_content` had no arm, so every `--native` lookup searched for `Content::Long(0)`: a `hash` missed present records, a `sorted` answered a reference whose fields all read null. Interpreter correct. | **fixed here** ([loft#811](https://github.com/loft-lang/loft/issues/811)) |
| `i8` / `i16` / `integer limit(min, max)` | the record side decodes through `get_short`/`get_byte` with the field's start hardcoded to `0`, while the lookup side reads the raw value — so the two differ by exactly that start and never compare Equal. Both backends. | **fixed** ([loft#812](https://github.com/loft-lang/loft/issues/812)) — `Key` carries `start`, derived in `determine_keys_for` from the one place that already knew it. The scope was wider than filed: `i16` is `Parts::ShortRaw`, which was read as a sign-extended `i16`, so ordering did NOT survive it (a `sorted` came out `0,1,2,-3,-2,-1` on `--native`) and a `u16` key ≥ 32768 was equally unfindable |

The regression guard for the two fixes is `tests/scripts/811-collection-key-widths.loft`.

The lesson worth keeping: an **equivalence** matrix is not a weaker instrument than a
feature matrix. Asking "same before and after?" across the composition axes exercised
key widths no performance question would have reached, and the cells that had never
worked answered "same" — identically broken on both sides — only because the cell
asserted a VALUE and a LENGTH rather than agreement between two runs.

## Sub-arcs

| Item | Worth | Format break | Status |
|---|---|---|---|
| **A** — build the entry in place (retarget the struct literal at the entry record; drop the scratch + `OpCopyRecord`) | measured **933 → 536 ms** on 1M inserts (−43%) | no | **Done** |
| **B** — hoist the key dispatch out of the probe loop (no new opcode; see below) | measured **~10 ns of a ~33 ns** cache-resident lookup (−20…25%) | no | **Done** |
| **C** — capacity hint: `reserve(h, n)` pre-sizes the bucket table | measured **618 → 352 ms** on 1M inserts, and half the table memory | no (additive) | **Done** |
| **Q2** — placement token in the @PLN97 layout identity, so an old store refuses instead of misreading | nothing on its own; it is what lets H spend a format break safely | no (absence = baseline) | **Done** |
| **D** — cache the hash in the bucket slot (`(u32 rec, u32 hash)`) | 171 ms + most per-probe cost | **yes** | **Retired by Q1** — attacks the 8% term and adds bytes to the 82% one |
| **E** — seeded integer hash + division-free bucket index | ~13% combined (measured); **insert-only — the lookup half is 1 ns of 36**, and arc D removes most of the insert half too | **yes** | **Retired by Q1** — revive only with a written P253 argument (Q3) |
| **H** — CHUNKED entry arena behind `Parts::Hash` (not one array — see Q5) | subsumes A/B/D + locality; **measured 2.3x** on random lookup, ceiling 2.6x | **yes** | **The only arc left, and now unblocked** — Q5 answered, Q2's refusal mechanism shipped; settle Q4 inside it; measure against 34 MB, not 49 |

## Phase ordering

1. **A — done.** The retarget turned out to be one branch, not a new mechanism: the
   BRACKETED spelling `h += [Entry { … }]` already built the entry in place (@P277
   intercepts before the RHS parse and hands it the element var as its target), so the
   working bytecode existed as a real runnable source shape and only the BARE
   `h += Entry { … }` was missing it. The bare form reached `parse_object` with the
   COLLECTION local as its target; `parse_object` rejects a target that is not
   `Reference(<this struct>)` and allocates a throwaway work-ref instead, which is why
   the P188 branch's intended `substitute_value(Var(collection) → Var(elm))` retarget
   never fired. `parser/expressions.rs::parse_assign_op` now takes the same pre-parse
   branch for a bare literal, gated on peeking `<element-name> {`. A variable, call or
   field-read RHS keeps the deep copy, as does a qualified (`pkg::Entry {`) or generic
   (`Pair<integer> {`) spelling — slower, never wrong.
2. **B — done, but NOT as a new opcode.** The plan proposed `OpGetRecordLongKey`; the
   measurement says the win does not need one. What costs is that `hash::find` re-runs
   `key_compare`'s `(Content, type_nr)` match *for every record probed*, while a probe
   only ever asks *is this the key* about the *same* key. `keys::fast_key` resolves the
   field offset and the value once, before the loop, and `FastKey::matches` reads the
   field directly. So it is a runtime change in `hash.rs` + `keys.rs`: no opcode, no
   bytecode surface, no parser change — and **both backends get it**, since `hash::find`
   is shared. It also pays on INSERT, because dedup runs one `find` per insert.

   The estimate that argued *against* building it was wrong, and cheaply so: the
   arithmetic said "≈5 ns of 97, not worth it", the 25-line env-gated probe said 10 ns
   of 33 at cache-resident sizes. Build the probe.
3. **C — done.** `reserve(h, n)`, extending the vector builtin (loft#710) to a hash's
   bucket table. Additive, contract-safe, and it buys memory as well as time: the
   growth ladder doubles *past* its trigger and lands at load 0.42, while a reserved
   table sits at the 0.75 it asked for.
4. **Re-measured — and the plan's expectation for this step did not survive.** A + B + C
   put insert where it predicted (~350 ms reserved, ~505 ms not), but **lookup is not
   near C# and cannot be got there from here**:

   | table size | ns / lookup |
   |---|---|
   | 1,000 | 30 |
   | 10,000 | 26 |
   | 100,000 | 33 |
   | 1,000,000 | **83** |

   The ~50 ns a 1M table adds over a cache-resident one is **cache misses** — the bucket
   slot and then the record, two random accesses over ~24 MB. No layout-neutral arc
   touches that: it is exactly what arc D (cache the hash in the bucket slot, so a probe
   rejects without reading the record) and arc H (contiguous entries) are for.

   **The fixed ~26 ns is NOT mostly SipHash — measured in situ, and this line used to say
   it was.** Swapping SipHash for a seeded 2-multiply splitmix at BOTH hash sites moves a
   cache-resident lookup by **1 ns of 36** (36 → 35, reproduced exactly across runs), and
   nothing resolvable at 1M. The 26.5 ns/op figure above is `key_hash` timed *in
   isolation*; it does not survive being measured where it actually runs.

   The instrument was falsified before the number was believed: with the same probe made
   degenerate (every key to bucket 0) the same benchmark goes 37 ns → 16,244 ns, so the
   fast path is unquestionably live. Do not re-derive this from the isolation figure — an
   isolated hash microbenchmark overstates this path by more than an order of magnitude.

   **Consequence for Q1:** arc E is an INSERT optimisation, not a lookup one. Its measured
   value is ~24% of insert, and ~80% of *that* is the resize re-hash — which arc D removes
   anyway by caching the hash. So D largely subsumes E's real win, while E alone carries
   the P253 security question (Q3). Rank accordingly: E is not the cheap lookup win the
   ordering above implied, and shipping D may leave E not worth its format break at all.

   So the lookup gap is now **measured to live entirely in D / E / H**, all three of
   which cost a persisted-store format break. That makes Q1 the next thing, not an
   optional one.
5. **Q1 and Q2 — both done (2026-08-09).** Q1 chose H over D+E. Q2 extended the @PLN97
   layout identity with a per-kind `placement` token, so an old store refuses rather than
   misreads; it shipped ahead of H because absence means the baseline, which invalidates
   nothing already on disk.
6. **H — next, and unblocked.** Build it as a CHUNKED arena, not one array: Q5 measured
   that chunking keeps the win (86 ns vs a dense 77 and today's 200) while leaving every
   live entry at the address it was handed out at, which is what keeps it compatible.
   Settle Q4 (load factor) inside it — a load-factor change is a layout change either way,
   so it costs nothing extra there and cannot be spent separately.

## The Q1 measurement (2026-08-09) — it is ONE access, and it is a byte problem

Step 4 left Q1 resting on "two random accesses — the bucket slot and then the record".
That split had never been measured, and it is what chooses between D and H. Measured, it
is not a split: **the record read is 82% of a 1M random lookup and the bucket read is
8%.** Probes: `probes/h-locality.loft` (the ablation), `probes/h-random-access-floor.loft`
(what the same payload costs read densely), `probes/q1-store-footprint.loft` (bytes per
entry, read off a persisted image). Ablation patch: `probes/q1-ablation.patch`.

Machine: AMD Ryzen AI 9 HX 370, 24 threads, 61 GB · `--native-release` · min of 5.
**Not the box the 2026-08-08 table was taken on** — read the deltas, not the absolutes.

### Where a lookup's time goes

`LOFT_PROBE_HASH` drops one memory access at a time (`skiprec` = no record read,
`floor` = bucket 0 and no record read, so neither can miss). Subtracting gives the split.
The rows below are the `nofield` shape — the loft program tests the `DbRef` and never
reads a field, so the only record access left is the one inside `find`.

| n, order | total | fixed | bucket read | **record read** |
|---|---|---|---|---|
| 10k ascending | 35 | 22 | 10 | 3 |
| 10k shuffled | 37 | 22 | 11 | 4 |
| 100k shuffled | 61 | 22 | 12 | 27 |
| 1M ascending | 87 | 23 | 18 | 46 |
| **1M shuffled** | **225** | 22 | 18 | **185 (82%)** |

The `field` variant's subtraction is NOT valid — its program-level `e.val` survives
`skiprec`, so `off − skiprec` there measures a second, already-cached read. Use `nofield`.

**Lookup ORDER is the axis that was missing, and it is worth more than every arc.**
Keys go in as 0..n and records are claimed in that order, so looking them up as 0..n
walks memory ascending and prefetches. Every figure in this plan before today was taken
that way. A real consumer does not: the icosahedron edge-midpoint cache behind #809 looks
up in mesh order. 1M ascending 87 ns, 1M shuffled 225 ns. (The first cut of the probe
computed the permutation INSIDE the timed loop and charged its 64-bit modulo — ~35 ns —
to the shuffled row; both orders now walk a precomputed vector, so what is left between
them is the access pattern alone.)

### What arc H can buy, measured without building it

A `vector<Entry>` IS arc H's layout — elements inline, insertion-ordered, word-addressed
— so the same 1M elements read in the same shuffled order out of a dense vector is H's
ceiling. (`e = v[k]` emits `OpGetVectorNullable`, a cursor: no copy, confirmed with
`LOFT_REPORT_COPIES`.)

| 1M, shuffled | ns |
|---|---|
| `vector<integer>` (8 MB) | 26 |
| **`vector<Entry>` (16 MB) — H's ceiling** | **93** |
| `hash<Entry[key]>` | 204 |

So H's ceiling is roughly half of today, and the floor it stops at is the machine: a
dense 16 MB array still costs 93 ns to read randomly, four times C#'s reported 22 ns for
the whole lookup. Whatever remains after H is not addressable by layout.

### Why: a hash spends 2–3x the bytes its payload needs

`store_persist_bind` writes the collection's store, so the file is the store image —
every claimed block, reachable or not. That makes bytes-per-entry an exact, external
measurement rather than an RSS guess (peak RSS cannot tell a live table from the second
copy a rehash transiently holds).

| 1M entries of `{integer, integer}` | store | B/entry | of which buckets |
|---|---|---|---|
| `vector<Entry>` (dense) | 16.00 MB | 16 | — |
| `hash`, `reserve`d | 33.00 MB | 33 | 5.33 MB |
| `hash`, grown — **before** the table-free fix | 49.26 MB | 49 | 10.24 MB |
| `hash`, grown — **after** | 34.02 MB | 34 | 6.24 MB |

Run one configuration per process. An earlier cut of `q1-store-footprint.loft` ran every
shape from one `main` and its grown row was WRONG — a collection's store SLOT is reused
between iterations, so the second fill inherited the first's buffer and growth ladder. It
read 49.26 MB both before and after the fix while a single-shot run read 49.26 → 34.02.
The harness was measuring itself; the probe now takes its shape and size as arguments.

That is the mechanism behind the 185 ns. It is a working-set problem, and **arc D does
not shrink the working set** — it adds 4 bytes per bucket slot. H is the only arc that
touches the term that carries the cost.

## The Q5 measurement (2026-08-09) — chunking keeps the win

Q5 asks whether H can keep entries STABLE, which today's per-entry claim gives for free
and a single contiguous array takes away. The candidate is an arena that grows by
appending a chunk and never reallocates one that already holds entries. It is only worth
building if chunking does not spend the locality H exists to buy — so measure that first,
the same way § What arc H can buy measured H's ceiling: the layout is a shape the language
can already build.

`probes/q5-chunked-arena.loft`, 1M `Entry{integer,integer}`, shuffled order, one
configuration per PROCESS, min of 3, `--native-release`. Same box as the Q1 table; read
the deltas, not the absolutes.

| shape | ns | vs today |
|---|---|---|
| dense `vector<Entry>` — H's ceiling | **77** | 2.6x |
| **chunked, 65536/chunk** | **86** | **2.3x** |
| chunked, 16384/chunk | 93 | 2.2x |
| chunked, 4096/chunk | 119 | 1.7x |
| chunked, 1024/chunk | 108 | 1.9x |
| chunked, 256/chunk | 126 | 1.6x |
| `hash<Entry[key]>` — today | 200 | — |

**Chunking costs ~12% of the ceiling, not the win.** At 64K entries per chunk the
candidate lands at 86 ns against a dense array's 77 and today's 200 — so Q5's stability
requirement is affordable, and the dilemma in Q5 was false: H does not have to choose
between keeping references valid and being fast.

**The figure is an UPPER BOUND.** The chunked rows pay an outer-vector hop — read the
chunk reference, then the element — that the real design does not have: there the bucket
slot holds `(chunk_rec, offset)` and names the chunk record directly, so an entry read is
still ONE random read. loft cannot express a bare (record, offset) pair, so the hop is
charged here; the real thing can only be faster than these numbers.

**Chunk size is the dial, and it is monotone-ish, not flat**: 256 → 126 ns, 65536 → 86 ns.
Bigger chunks buy locality and cost tail waste (a 64K-entry chunk is 1 MB, so a hash with
one entry rounds up to a chunk). The 4096 row reading above 1024 is inside run-to-run
noise at this sample size — treat the trend, not the individual cells. Entry bytes are
identical across every chunked row (16.00 MB, same as dense), so the difference is layout
alone.

The `bytes` column for the `hash` row is `size(h)` — the bucket table ONLY, per
`table_bytes`' allocation-local contract — so it is not comparable with the dense and
chunked totals, which are entry bytes. § Why: a hash spends 2–3x the bytes its payload
needs has the comparable per-entry figures.

### Found while measuring: growing a hash ABANDONS every previous bucket table

`add`'s growth path and `reserve` both `claim` a new table, `rehash_into` it, and
repoint the field — and neither returns the old claim. `hash.rs` never calls
`Store::delete` (`radix_tree.rs` does, so the convention exists; this is an omission).
The blocks stay CLAIMED and unreachable, which is why the grown row above costs 16.26 MB
more than the identical content pre-sized while its bucket table is only 4.9 MB bigger.

Two independent confirmations: the code has no `delete`, and `store_reclaim` on a
freshly grown, never-bound 1M hash recovers **0 bytes** — the space is not free, it is
claimed. A persisted grown hash carries its dead tables to disk forever.

**FIXED (2026-08-09).** `install_table` repoints the hash's field and frees the
predecessor as one step — the order is load-bearing, because `Store::delete` repurposes
the block's body as a free-tree node, so a free before the repoint would leave the field
naming bytes that are already something else. Both replacement sites route through it.

Measured: a grown 1M hash 49.26 MB → **34.02 MB**, and the freed space is reused rather
than returned, so the final table also lands smaller (10.24 → 6.24 MB) and the load factor
rises 0.39 → 0.64. That last part is a real behaviour change and was checked rather than
assumed: 1M shuffled lookup 244 ns after vs 252 ns before (min of 3) — the denser table
pays for the longer probe chains. `reserve`d fills are byte-identical before and after,
which is the control: `reserve` on an EMPTY hash has no predecessor, so the fix must not
move it.

The other half of the question — freeing a block someone still reaches is worse than
leaking it — is what the guards are for. Iteration snapshots record numbers up front
(`build_hash_sorted_vec`), so `for x in h { h += … }` rebuilds the table under a running
loop; `check_iter_safety` does not list `Type::Hash`, so that is reachable user code and
it is a cell in the script test.

### What this settles

- **Q1 → H, and D is not worth a format break.** D attacks 8% and adds bytes to the term
  that carries 82%. H's measured ceiling is ~2x on random lookup and it subsumes A/B/D.
- **Q3 dissolves.** It only existed to license arc E, which step 4 already measured at
  1 ns of 36 on lookup; nothing here revives it. No security argument needs writing.
- **Q4 is not resolvable from this data.** Reserved-vs-grown at 1M shuffled came out 225
  vs 200 on the instrumented binary and 213 vs 226 on the clean one — both orderings
  observed, so the load-factor question stays open and needs its own probe.
- **Order belongs in every future measurement.** A benchmark that looks up in insertion
  order understates this path by ~2.5x, and every figure in this plan predating today
  was taken that way.

## How to build H — the surface, read off the code (2026-08-09)

Scoping pass before writing any of it. Three findings change the size of the job.

**1. `--native` shares this runtime, so H needs NO separate codegen.** `OpGetRecord` in
`codegen_runtime.rs` calls `stores.find(&data, db_tp, key)` — the same
`database/search.rs` → `hash::find` the interpreter uses, and `OpNewRecord` /
`OpFinishRecord` delegate the same way. Fix the runtime and both backends move together.
This was the main reason H looked MH-shaped; it is smaller than it reads.

**2. The per-entry overhead is one word, and it is not spare.** `record_new`'s keyed arm
claims `1 + ceil(size/8)` words per entry. Word 0 holds the size header in bytes 0–3 and a
**back-pointer to the owning collection in bytes 4–7**; the payload starts at offset 8,
which is why an entry `DbRef` carries `pos: 8`. So `{integer,integer}` costs 24 B where a
dense `vector<Entry>` costs 16, and `claim_block` takes a whole free block rather than
split when the remainder is under a third — that is the measured 27.67 B/entry.

**The back-pointer is READ, not just written**: `database/search.rs:116` and `:146` test it
to decide whether a record is live, and `state/io.rs:750` reads the same offset as a stored
TYPE for a different record kind. An arena has no per-entry word to put it in, so H must
give it a home — per CHUNK is the obvious one, since every entry in a chunk shares an
owner — and must keep `search.rs`'s liveness test answering the same question.

**3. Entry creation and entry freeing must land in the SAME commit.** `record_new` claims
the entry; `allocation.rs`'s `Parts::Hash` arm enumerates `hash::records()` and frees each
entry as an `OwnedChild`. Redirect creation into an arena without changing the free path
and the collector frees interior arena bytes as if they were records — silent heap
corruption, the failure this subsystem is ranked weakness #1 for. They are one change.

### Layout

The bucket slot stays **4 bytes**. It holds a 1-based ENTRY INDEX, not a record number;
`(chunk, offset)` is arithmetic (`chunk = (idx-1) >> SHIFT`, `off = ((idx-1) & MASK) *
stride`) against a chunk directory that is a handful of `u32`s and therefore always cache-
resident. That is not a shortcut around the Q5 measurement — it is what the Q5 probe
measured: `vector<vector<Entry>>` pays exactly this directory hop, so the 86 ns figure
already includes it. Widening the slot to 8 bytes to hold `(chunk_rec, offset)` directly
would remove a hop that costs nothing and double the bucket table, which is the trade arc D
already lost.

Table record: `LEN` and `SEED` keep their offsets; add `DIR` (the chunk-directory record)
and `NEXT` (the append cursor) before `BUCKET0`, and bump `BUCKET0`. The directory is its
own record so it can grow without moving entries — **growth appends a chunk and never
reallocates a filled one, which is the whole of Q5's answer.**

### Touch points

| file | what changes |
|---|---|
| `hash.rs` | arena alloc + index decode; `add`/`find`/`remove`/`records`/`count`/`table_bytes`/`rehash_into` |
| `database/structures.rs` | `record_new` keyed arm → arena slot for `Parts::Hash`; `finish_record` |
| `database/allocation.rs` | the `Parts::Hash` owned-child arm, and `copy_claims` (@P318) |
| `database/search.rs` | the liveness test that reads offset 4 |
| `paged_reader.rs` | its read-only port of `find` |
| `placement.rs` | bump `placement::HASH` — the mechanism Q2 shipped for exactly this |
| `fill.rs` | nothing, if `count`/`table_bytes` keep their contracts |

### Gates

`loft introspect` on BOTH backends before and after (CODEGEN_METHOD.md); the boundary
matrix on `--interpret` first, then both; `placement_contract_is_pinned` MUST fail and be
re-blessed with the new constants — a token nobody bumps is a comment; and
`a_changed_placement_token_refuses_the_store` must still refuse a pre-H store rather than
misread it. Re-measure against **34 MB**, not 49.

## What H actually bought (2026-08-10) — insert and bytes, not lookup

Built, and measured against the installed `v2026.8.0` as a before-oracle, alternating
A/B on a quiet box (24 cores, load 3.6), 1M `integer` keys, `--native-release`:

| | before | after | |
|---|---|---|---|
| insert (reserved) | 330 ms | **258 ms** | **1.28x** |
| store bytes / entry (slope over 100K→800K) | 27.67 B | **18.6 B** | **−33%** |
| claimed records, 2000 entries | ~2000 | **9** | table + directory + 6 chunks |
| random lookup | 184 ns | 183 ns | **unchanged** |

**The predicted 2.3x lookup win did not happen, and the reason is in the reasoning, not
the build.** This plan inferred it from two measurements that are both correct:

* Q1's ablation — *the record read is 82% of a random 1M lookup*;
* Q5's shapes — a dense `vector<Entry>` reads in 80 ns where the hash takes 200.

The inference from them is what fails. **80 ns is ONE random read; a hash lookup makes
TWO** — the bucket slot, then the entry. Packing the entries changes WHERE the second
read lands; it does not make it stop missing, because a lookup by construction has no
locality to exploit. Q1's 82% says that read is expensive — never that its being
scattered is what made it so, and the two are different claims that the ablation cannot
tell apart. Density pays for SEQUENTIAL or clustered access, which is exactly what the
`vector<Entry>` probe measured and a hash never does.

What the arena does remove is the per-entry `Store::claim` — a header word, the
`claim_block` rounding, and one allocator round trip per insert — which is what
[loft#809](https://github.com/loft-lang/loft/issues/809)'s title names, and it shows up
on **insert and on bytes**. That is a real win and worth the format break; it is not the
win this plan advertised, and the two should not be conflated in whatever comes next.

**Chunk sizes double only to `arena::CAP_CHUNK`, then stay fixed.** Uncapped, the tail
waste is proportional — a collection just into chunk `k` has up to half of it empty — and
that measured 27.33 B/entry, i.e. it ate the entire per-entry saving, while also making a
store's SIZE depend on construction order. Capping bounds the waste at one partly-filled
chunk. It is the difference between −1% and −33%, and it was invisible until the footprint
was measured against the before-oracle rather than reasoned about.

**A hash has TWO kinds of entry**, which this plan's § How to build H did not name. A
PRIMARY hash allocates its own entries from the arena. A SECONDARY index — a sibling
field's `other_indexes` — is a second route to records the PRIMARY owns, and it may
neither move nor free them. The discriminator is the stride the table records, because
that IS the distinction: a table that allocated its entries knows their width, a table
that borrows records has none to know. `stride == 0` means borrowed. Found by an assertion
in `add`, not by reading the code.

## What is LEFT after H, measured (2026-08-10) — and why none of it pays

[loft#809](https://github.com/loft-lang/loft/issues/809)'s own summary lists three
things as genuinely open. H closed the first. The other two were measured before
anything was built for them, and neither earns its cost.

**The from-scratch re-hash at every resize — worth ≤12% of one insert case.**
`reserve(h, n)` already removes it entirely, so the only case it costs anything in is
an unreserved fill. Measured, 1M `integer` keys, `--native-release`, alternating
reserved against grown so the gap IS the resize:

| round | reserved | grown | resize |
|---|---|---|---|
| 1 | 274 ms | 366 ms | 92 ms |
| 2 | 307 ms | 350 ms | 43 ms |
| 3 | 358 ms | 386 ms | 28 ms |

Median ~12% of a grown insert, and **0% of a reserved one**. Removing it means
caching the hash in the bucket slot (arc D): the slot goes 4 → 8 bytes, so the table
DOUBLES — 5.3 MB → 10.7 MB at 1M — and it costs a second persisted-store format
break. Spending a break and doubling the table for ≤12% of the case a one-line
`reserve` already fixes is not a trade worth making. **Arc D stays retired.**

Note H itself already reduced this: `rehash_into` re-reads each entry's key, and
entries are now dense, so the walk it does is over a 16 MB arena rather than
scattered records.

**The two random accesses — cannot become one without giving up a correctness
property.** A lookup reads the bucket slot, then the entry; both are random, and no
layout removes either while entries keep the address they were handed out at. Putting
the entries INSIDE the bucket table is the design that removes one, and it is exactly
what Q5 refused: growth would then move every entry and invalidate every outstanding
`DbRef` — a wrong READ, which `COMPATIBILITY.md` forbids. Caching a key fingerprint in
the slot (D again) skips the entry read on a colliding probe but not on a HIT, which
is the case being timed.

Measured, for anyone tempted to re-derive it — the same 1M lookups in two orders:

| order | ns/lookup |
|---|---|
| ascending | 99–118 |
| shuffled | 175–179 |

Ascending is not "cache-friendly hashing"; it is the ENTRY read becoming
prefetchable, because entries were inserted in key order and the arena is dense.
The bucket read stays random in both. That ~1.7x is also a warning about the
report's original 20x: **a comparison whose two sides use different access orders
is not a comparison**, and this plan's pre-2026-08-09 figures were all taken in
insertion order.

**A hoist was tried and dropped.** `entry_ref` re-read the directory field and the
directory's size header on every probe, both loop-invariant — arc B's shape exactly.
Hoisting them measured nothing: alternating A/B, 1M shuffled, median 214 ns before
and 231 ns after, i.e. inside the noise, because the loop is two cache misses and
those reads are hot. It is not zero work, but it is below what this box can resolve,
and an unmeasured change to the heap's hot path is not worth its review cost.

**Recommendation: close #809.** Its headline — the per-entry store claim — is gone,
insert is 1.28x and bytes 33% lower. What remains is the cost of two random memory
accesses, which is physics rather than a defect, and the arcs proposed against it
cost more than they return.

## Open design questions

1. **(blocks D, E, H) Approach `Dictionary` or match it?** — **measured 2026-08-09; the
   answer is H.** See § The Q1 measurement above: the record read is 82% of a random 1M
   lookup, D does not shrink it, and H's ceiling is roughly half of today's cost. Retire
   D unless the abandoned-table fix plus H still leave a gap D would close.
   Original framing follows. D+E keeps per-entry records
   and buys back the resize and hash terms. H restructures `Parts::Hash` onto an inline
   contiguous entry array — C#'s shape, still word-addressed and pointer-free, so still
   mmap-able and range-fetchable — and subsumes A/B/D while fixing the locality that
   none of the others touch. **They conflict:** a bucket-slot change spent on D is
   thrown away by H, and each costs one persisted-store break. Decide before spending it.
2. **~~How does an old store refuse?~~ — SHIPPED (2026-08-09).** Resolved as the first of
   the two options below: a per-kind `placement` token in the layout dump, not a
   `SIGNATURE` bump, which would have refused every store ever written including ones
   with no hash. See § Status for the mechanism and its guards. Original framing: The
   @PLN97 layout identity covers storage layout
   (field positions, sizes, endianness), not bucket placement — so today a changed hash
   would be *misread*, not rejected. Options: add a bucket-algorithm token to the layout
   dump (rejects only stores that use a hash), or bump `SIGNATURE` "Sto1" → "Sto2"
   (rejects every store, including ones with no hash). Prefer the former.
3. **~~Does arc E's seeded integer hash preserve the P253 property?~~ — DISSOLVED
   (2026-08-09).** The question exists only to license arc E, whose lookup win is 1 ns of
   36 (step 4) and whose insert win arc D/H takes anyway. Nothing needs to be argued
   unless E is revived. Original framing: The seed is mixed in
   before a full murmur3 finalizer, so bucket collisions still depend on a seed the
   program never reveals. Distribution was measured equivalent to SipHash. What is NOT
   yet argued: whether xor-then-mix resists a differential attack as well as SipHash's
   keyed construction. Needs a written argument before it ships, not just a histogram.
4. **Should the load factor move? — still open, and the 2026-08-09 probe did NOT settle
   it** (reserved-vs-grown flipped between the instrumented and clean binaries; needs a
   probe that varies load factor directly rather than via `reserve`). 0.42 wastes ~2.4x the bucket memory it needs. Raising
   it costs probe length (currently 1.37) and buys cache locality. Only worth touching
   inside D or H, since it is a layout change either way.
5. **(blocks H) Do entry references survive growth?** — **new, 2026-08-09.** Today they
   do, and H removes that. Growth reallocates only the BUCKET table: `rehash_into` moves
   `u32` record numbers between tables and never touches an entry's bytes, so an entry
   keeps its own claim and its `DbRef{rec, pos: 8}` stays valid for as long as the entry
   lives. Verified: `e = h[1]` read back correctly after ~2000 further inserts (many
   doublings).

   H makes an entry a POSITION inside one contiguous array — `DbRef{rec: <array claim>,
   pos: <slot offset>}`, the shape a vector element already uses — so growing the array
   moves every entry, and every outstanding reference into it goes stale. `e = h[k];
   h += [other]; e.val` then reads moved-or-freed bytes. That is the vector-reallocation
   hazard, which the deps/lifetime system already governs for vectors, arriving somewhere
   it has never applied.

   So H is not only a format break, it is an OWNERSHIP change, and it lands in the area
   [`OWNERSHIP_MODEL.md`](../../OWNERSHIP_MODEL.md) and CLAUDE.md rank as weakness #1.
   **This question, not the byte layout, is the expensive half of H.**

   **Leading candidate — a CHUNKED arena, not one array.** Making the entry reference
   borrow (growth invalidates it, as a vector's does) is the obvious answer and the wrong
   one: it silently breaks programs that work today, which
   [`COMPATIBILITY.md`](../../COMPATIBILITY.md) forbids, and it breaks them into a *wrong
   read* rather than an error. But the dilemma is false. H needs entries DENSE; it does
   not need them in ONE allocation. An arena that grows by appending a new chunk — never
   reallocating a chunk that already holds entries — keeps every live entry at the address
   it was given, so a `DbRef{rec: <chunk claim>, pos: <offset>}` stays valid for the
   entry's whole life, exactly as today's per-entry claim does. The bucket table keeps
   doubling and rehashing; bucket slots are indices and cost nothing to move.

   That buys most of what H is for. The measured gap is ~27.7 B/entry against a dense
   vector's 16 (33 total − 5.33 of buckets), and the difference is per-record header — the
   thing chunking removes. What it gives up is cross-chunk contiguity, so the chunk size
   sets how close H gets to the 93 ns `vector<Entry>` ceiling. **Probe before building:**
   a `vector<Entry>` read in shuffled order, chunked at several sizes, measures the whole
   candidate without writing any of H (the same trick § What arc H can buy already used —
   the layout it proposes IS a shape the language can already build).

## Cross-arc dependencies

- **@PLN97** (layout contract) — Q2 extended the layout identity with a per-kind
  `placement` token (`src/placement.rs`, shipped). H cannot ship without it; it is now
  there, so H is free to move the bucket constants and bump `placement::HASH`.
- **loft#808** (shipped) — arc A is the same defect class at a third boundary. Compound
  values materialise into heap records at the local (free for tuples), at the **return**
  (#808, fixed), and at the **collection insert** (arc A). Reuse its technique.
- **@PLN102 arc A** (`COMPATIBILITY.md`) — `CONTRACT_VERSION` is 0, so a format break is
  permitted now; this is the pre-freeze window the doctrine names for exactly this.

## See also

- [`loft-lang/plans#135`](https://github.com/loft-lang/plans/issues/135) — `@PLN135`, the tracker issue.
- [loft#809](https://github.com/loft-lang/loft/issues/809) — the source report.
- [loft#808](https://github.com/loft-lang/loft/issues/808) — the shipped sibling; arc A's technique.
- [`DATABASE.md`](../../DATABASE.md) — stores / `DbRef` / `Parts`.
- [`COMPATIBILITY.md`](../../COMPATIBILITY.md) — the store-layout surface and the contract-0 window.
- [`PERFORMANCE.md`](../../PERFORMANCE.md) — benchmark homes.
- `Moros-Economy-Development/loft_eval/` — the consumer corpus (`bench/hash_bench.loft`,
  `probes/csharp-baseline/Program.cs`, `bytecode-comparisons/809-integer-key-hash.*`).
