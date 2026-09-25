# The keyed class — why it is 4–8× Rust

Analysis of the portal's slowest complete class (`hash`, `sorted`, `index`; every row over
the ceiling), taken 2026-09-21 on x86-64 from `bench/15_stdlib_keyed`.  It names the
mechanisms, prices what could be priced, and ranks what to build.

**L1–L7 are BUILT (2026-09-21)** — see [§ What L1–L4 bought](#what-l1l4-bought) and
[§ What L5–L7 bought](#what-l5l7-bought): the class median went 6.02× → 4.31× → **3.49×**.
No row is within the bar yet; [§ What is left](#what-is-left) says what stands between.
The sections before those describe the tree the levers were built against.

## The regime: instruction-bound, not memory-bound

The lane holds 5,000 entries — cache-resident, which is the size every keyed consumer
routine in `census.tsv` runs at (a 64-glyph atlas, 400 forms, 5,000 instances, 10⁴–10⁵
cells).  There a Rust `HashMap` lookup is ~10 ns and loft's is ~62 ns, and callgrind says
why: over one round of the lane loft executes **145.7 M instructions against Rust's
16.8 M (8.7×)**, and per routine the instruction ratio tracks the time ratio.

| routine | loft instr/item | Rust instr/item | instr × | time × |
|---|---:|---:|---:|---:|
| `hash_fill` | 3,797 | 488 | 7.8 | 6.1 |
| `hash_find` (hit + miss) | 924 | 177 | 5.2 | 5.9 |
| `hash_update` (hit) | 695 | 186 | 3.7 | 4.2 |
| `hash_remove` | 6,578 | 781 | 8.4 | 6.4 |
| `hash_text_keys` | 7,525 | 968 | 7.8 | 7.6 |
| `sorted_fill_walk` | 4,124 | 478 | 8.6 | 3.9–5.6 |
| `index_fill_find` | 9,080 | 686 | 13.2 | 4.9 |

(Instruction counts were taken before the lane's element types were split — see the last
section: the hash rows in record-backed mode, `sorted` as an `ordered`, whose time ratio is
the 3.9; 5.6 is the inline `sorted` the row measures now.)

So the class is slow because of PATH LENGTH.  @PLN135 closed the hash with *"what remains
is two random memory accesses, which is physics"* — measured at 1 M entries, where that is
true (the record read is 82 % of a 183 ns lookup).  It does not transfer to this regime,
and this regime is the one programs are in.

## Where the instructions go

**A lookup (924 instr averaged over a hit and a miss; Rust 177).**

| part | instr | why |
|---|---:|---|
| hashing | 244 | `key_hash` 120 + two out-of-line `SipHasher13::write` calls at 62 each |
| probe loop | 401 | `hash::find` 253 + `FastKey::matches` 148 (out of line, 3.83 calls per lookup) |
| `Stores::find` | 82 | a runtime match over the collection's type row, the absent test |
| the miss path | 165 | 330 per MISS: see (3) |
| caller | 48 | `Content` slice built per call, `or_null` |

1. **The seed is a message word, fed through the byte-slice `write`.**  SipHash-1-3 is
   what Rust's `HashMap` runs too, so the algorithm is at parity — but loft's hasher takes
   the per-table seed as a first 8-byte `write` and the key as a second, each through the
   general tail-buffering `write` (62 instr for a word whose compression round is 14), not
   inlined.  245 instr per hash against Rust's 141.
2. **Linear probing at a maximum load of 0.75, one out-of-line compare per occupied
   probe.**  The lane's table sits at load 0.72, and the measured compares are **2.30 per
   hit and 5.36 per miss** — exactly what linear probing predicts there, and far from the
   1.39 / 1.13 the same 5,000 keys cost at load 0.44.  Which of the two a program gets is
   luck: the table doubles from whatever block the first `claim(12)` handed back (12 words
   in one program, 28 in the lane), so the ladders differ and the same `n` lands at 0.44
   or 0.72.  Each compare is a call to `FastKey::matches` plus an arena slot decode;
   hashbrown answers the same miss with one 16-byte group compare.  The bucket index is
   `hash % count` with a non-power-of-two `count` — a 64-bit `div` per lookup.
3. **Every MISS runs the lazy-store machinery before it may answer null** (`--native`,
   `codegen_runtime::get_record_lookup`): the element type's name is copied into a fresh
   `String`, two `Mutex`es are locked (`lazy_fetch_refusal` clones an `Option<String>`,
   `lazy_fetch_fn` scans a table), and `lazy_loft_source` and `fetch_missing` each hash
   the collection's address into `lazy_sources` — in a program that binds no lazy source
   at all.  30 % of `hash_find`'s time in the sampled profile.  A miss is not rare: it is
   every find-or-insert, every dedup, every `if h[k] == null`.

**An insert (3,779 instr; Rust 488).**

4. **3.6 SipHashes, two probe walks and three allocations per insert.**  `dedup_keyed`
   clones the key descriptor `Vec`, materialises the key into a `Vec<Content>`
   (`get_key`), hashes it and probes for a duplicate; `hash::add` then re-reads the key
   from the record, hashes it AGAIN and probes again for the free slot; growth re-hashes
   every entry (amortised 1.6 per insert — hashbrown pays this one too).  Around that,
   ~900 instr of type-table-driven record machinery (`record_new` →
   `nullable_field_parent` → `sub_record_type` → `field_ref` →
   `set_default_value_nullable` → `record_finish` → `insert_record` → `link_siblings`)
   re-derive per element what the compiler knew at the append.
5. **A removal re-hashes its cluster and, for a record-backed hash, frees a store block.**
   `hash::remove`'s backward shift computes a full SipHash for every entry after the hole
   up to the next empty slot — a walk whose length grows as 1/(1−α)².  Where the element
   type is record-backed (below) each removal is also a `Store::delete`: a free-tree
   insert and rebalance, ~1,900 instr per removal.

**The ordered kinds (`index` 9,080 instr/item, `sorted` 4,124; Rust's `BTreeMap` 686 / 478).**

6. **A comparison costs 90–100 instructions.**  `key_compare` / `compare` run a
   `(Content, type_nr)` match, a bounds-checked store lookup and a validated read per
   call, not inlined — 37 % of `index`'s instructions, 24 % of `sorted`'s.  `hash::find`
   got a pre-resolved `FastKey` for equality (@PLN135 arc B); ordering never did.
7. **`index` descends twice per insert and never stops early.**  It is a red-black tree
   with one node per store record: 13–16 dependent node visits per descent where a
   B-tree makes ~5.  `tree::find` does not stop at an equal key (it runs to the leaf and
   steps back), and every insert is a dedup `find` and then a `put` — 25.5 key compares
   plus 11 record compares per item.
8. **`sorted` inserts by moving its inline elements.**  ~490 instr/item of `memmove` at
   5,000 × 16 B, plus the dedup search before the insert search.

**Text keys** pay all of (1)–(4) plus `arena::slot` walking the chunk directory per probe
(582 instr/item) and a copy of the key text per insert (the twin borrows `&str`).  A
**composite key** (`16_consumer_shapes/composite_hash`, 8.1×) has no `FastKey` at all and
takes the generic comparator on every probe.

## What was priced

Three temporary patches (`keyed-pricing-probes.diff`, reverted): **A** an inline one-word
`write_u64` (digest byte-identical), **B** the miss answers at once when the collection
has no lazy binding, **C** an integer fast path at the top of `key_compare` / `compare`.

| routine | before | after | |
|---|---:|---:|---|
| `hash_find` | 649 µs | 466 µs | **−28 %** (B ≈ −20, A ≈ −8) |
| `hash_remove` | 2,160 | 1,888 | −13 % |
| `hash_fill` | 1,173 | 1,045 | −11 % |
| `hash_text_keys` | 999 | 900 | −10 % |
| `index_fill_find` | 2,869 | 2,625 | −8.5 % |
| `hash_update` | 216 | 201 | −7 % |
| `sorted_fill_walk` | 1,331 | 1,245 | −6.5 % |

C is small for a reason worth keeping: the fast path still costs 58 instr per call,
because the call, its five arguments and the store lookup stay.  The comparator has to be
resolved once per DESCENT and inlined into it, as `fast_key` is for the probe loop.

## What to build, in order

| | lever | size | expected |
|---|---|---|---|
| L1 | a miss with no lazy binding answers at once (both backends' `get_record`) | XS | `hash_find` −20 %, every find-or-insert (measured) |
| L2 | inline one-word SipHash write; seed absorbed once per call | XS | every hash row −7…−11 % (measured) |
| L3 | insert hashes ONCE: the dedup probe hands its hash and its free slot to `add`; the key is read through a `FastKey` off the record (no descriptor clone, no `Vec<Content>`) | S | `hash_fill` ≈ −20 % (est. −800 instr) |
| L4 | a pre-resolved ORDER comparator hoisted out of `tree::find` / `put` / `sorted_find` / `ordered_find`; an exact lookup stops at the equal key; the insert's dedup folded into `put`'s own descent | S | `index` ≈ −35 %, `sorted` ≈ −15 % (est.) |
| L5 | the probe loop: maximum load 0.75 → ~0.55 (a writer's policy, no format break: 40 KB instead of 27 KB at 5,000 entries), the compare inlined per key kind, `%` replaced by a per-table multiplier | S–M | misses ≈ −40 %, hits ≈ −15 % (est.) |
| L6 | `--native` lowers a lookup or an append on a statically known `hash<T[integer]>` to a typed entry point: no `Content` slice, no `Stores::find` dispatch, no type-table walk per element | M | the remaining ~250 instr of a lookup, ~900 of an insert |
| L7 | removal: carry the home bucket instead of re-hashing the cluster | S | `hash_remove` ≈ −10 % (est.) |
| L8 | `for e in hash` sorts its walk DECORATED: one key read per record into `(key, index)` pairs, sorted; the comparator form resolved two stores and dispatched on the kind per comparison | XS | `hash_walk` 540 → 257 µs/op, 5.93× → 2.83× (measured) |

L1–L4 are the clear cases — each shows in the statistics and has one visible reason.
Estimated together they take lookups from ~6× to ~3.5× and the fills to ~4×; reaching 2×
needs L5 and L6, because after them the remaining distance IS the generic entry path.
The estimates are instruction ledgers, not measurements; only L1, L2 and the C half of L4
were run.

## What L1–L4 bought

Same box, same lane, seven pinned samples a row, spreads under 2 %:

| routine | before | after | | × Rust |
|---|---:|---:|---:|---|
| `hash_fill` | 1,130 µs | 711 µs | **−37 %** | 6.10 → 3.87 |
| `hash_find` | 620 | 418 | **−33 %** | 5.94 → 4.03 |
| `index_fill_find` | 2,928 | 1,961 | **−33 %** | 4.90 → 3.31 |
| `grouped_fill_find` | 1,527 | 1,035 | −32 % | 6.69 → 4.59 |
| `composite_hash` (consumer lane) | 1,006 | 696 | −31 % | 8.12 → 5.30 |
| `hash_remove` | 1,813 | 1,289 | −29 % | 6.44 → 4.62 |
| `hash_text_keys` | 1,019 | 758 | −26 % | 7.61 → 5.62 |
| `sorted_fill_walk` | 1,813 | 1,636 | −10 % | 5.59 → 5.07 |
| `hash_update` | 218 | 199 | −9 % | 4.18 → 3.85 |
| `word_count` (engine lane) | 20.96 ms | 19.94 ms | −5 % | 4.68 → 4.47 |

What was built, and where:

* **L1** — `Stores::lazy_bound`, asked first by both backends' `get_record`
  (`codegen_runtime::get_record_lookup`, `State::get_record`).  The interpreter's miss was
  paying for a walk of the program's definitions (`has_lazy_driver`) the same way.
* **L2** — `SipHasher13::write_u64` takes a whole word on a word boundary as one inline
  round; a word after a text still takes `write`.  The digest is pinned by
  `tests/siphash_std_parity.rs`, whose compound cells cross both arms.
* **L3** — `hash::probe_for_insert` answers the duplicate AND the free bucket on one walk,
  `hash::add_at` files the entry; the key is compared through `keys::fast_key_of` (no
  descriptor clone, no `Vec<Content>`), record against record for a compound key.  A found
  duplicate, a missing table and an already-filed record keep the two-walk form.
* **L4** — `keys::FastOrder` (the order half of `FastKey`, direction applied once) resolved
  per search in `tree::find`, `vector::sorted_find` / `ordered_find` and their new
  record-keyed fronts; `tree::find_exact` stops a FULL-key lookup at the equal node; a
  refused `tree::add` names the duplicate and leaves the tree unchanged, so an `index`
  insert no longer looks its key up first.  A text key cannot be held across `tree::put`
  (it rebalances the store the text lives in), so a text-keyed `index` insert still orders
  generally.

`sorted` moved least, as priced: its insert is the `memmove` (8), which no comparator
touches.  `hash_fill` moved more than the ledger said — the ledger counted the second hash
and walk, and the descriptor clone, the key `Vec` and two of the three allocations went
with them.

**Bisecting and falsifying.**  `LOFT_NO_FAST_ORDER=1` and `LOFT_NO_ONE_PROBE_INSERT=1`
restore the general forms at run time, on both backends.  `LOFT_KEYED_VERIFY=1` checks
every pre-resolved comparison, every exact lookup and every one-probe insert against the
general form as it is made.  The guard is `tests/scripts/158-keyed-fast-paths.loft` (14
cells, every answer by hand; three sabotaged fast paths each fail five of them) with
`tests/keyed_fast_paths.rs` running it on the general paths and under verification; the
interpreter's whole script corpus passes under `LOFT_KEYED_VERIFY=1`.  That sweep found one
defect, in the check itself — `find_exact` read an ABSENT collection's reserved id as a
root (loft#1213's shape); a root is now a link like any other, and a negative one is no
node.

**Found on the way, not built:** an `ordered` (a `sorted` over an element type some
`index` or `hash` also holds) does NOT displace a duplicate key — `ordered_finish` ignores
what its search found — while the inline `sorted` replaces in place.  So declaring an
unrelated `index<T[…]>` changes what `sorted<T[…]>` does with a repeated key
(`2:2 5:3 5:1 9:4`, length 4, where the inline form holds three).  It predates this pass
and the general paths answer the same; the cells here pin neither.  Filed as loft#1572,
with a verified workaround (remove the key first).

## What L5–L7 bought

Against the L1–L4 build, same box and lane:

| routine | L1–L4 | L5–L7 | | × Rust |
|---|---:|---:|---:|---|
| `hash_remove` | 1,289 µs | 970 µs | **−25 %** | 4.62 → 3.47 |
| `grouped_fill_find` | 1,035 | 804 | **−22 %** | 4.59 → 3.53 |
| `hash_fill` | 711 | 563 | **−21 %** | 3.87 → 3.07 |
| `hash_update` | 199 | 171 | −14 % | 3.85 → 3.30 |
| `hash_find` | 418 | 364 | −13 % | 4.03 → 3.51 |
| `hash_text_keys` | 758 | 666 | −12 % | 5.62 → 5.01 |
| `composite_hash` (consumer lane) | 696 | 478 | **−31 %** | 5.30 → 3.87 |
| `index_fill_find` | 1,961 | 1,900 | −3 % | 3.31 → 3.20 |
| `sorted_fill_walk` | 1,636 | 1,642 | 0 | 5.07 → 5.08 |

* **L5 — the probe loop.**  A table is rebuilt at HALF full (`hash::is_full`; a writer's
  policy, so no format break — `LOFT_NO_HALF_LOAD=1` restores 0.75).  The lane's table
  went from load 0.72 to 0.25 and a lookup from 3.83 key compares to 1.22.  The
  loop-invariant half of decoding a bucket slot is read once per walk (`hash::Entries`),
  and the walk is compiled once per key kind (`hash::probe`, `find_fast`).
  ⚠ What this pass learnt about `#[inline]`: as a HINT it was declined for
  `FastKey::matches` and for the order comparator, and the out-of-line copies re-dispatched
  on the key's kind per bucket at three times the cost of the read — with the hint alone
  a lookup got SLOWER per probe while making a third as many.  They are
  `#[inline(always)]` now; the same fix took `index` 6,211 → 5,413 instructions an item.
* **L6 — the typed lookup** (`@FR-R-TypedKeyed`, `--native`): a lookup in a `hash` with one
  integer key is emitted as `OpGetHashLong`, the key handed over as a value: **613 → 444
  instructions** a lookup (Rust: 177).  `LOFT_NO_TYPED_KEYED=1` restores the general call.
  The general entry had become most of a lookup once L5 cut the walk to ~1.2 buckets.
* **L7 — removal.**  The ledger found what the estimate had not: `Stores::remove` and
  `remove_owned` CLONED the collection's type row and its key descriptors, three
  allocations per removal, and a removal mapped its record back to an arena index twice
  (`arena::index_of`, a scan of the chunk directory) — once to find its bucket, once to
  free its slot.  Now the kind is a `Copy` enum read off the row, the entry is recognised
  by decoding the slots on its chain (`Entries::names`), the bucket value found is what
  `hash::free_slot` releases, and the back-shift measures its distances without the three
  divisions per entry it made.  "Carrying the home bucket" per entry, as this lever was
  first worded, would need a wider slot — a format break — and half load already cut the
  cluster a removal re-hashes to about one entry.

**What half load costs, measured where it could hurt — a million entries.**  Same binary,
`LOFT_NO_HALF_LOAD=1` for the old rule, shuffled keys:

| 1 M `integer` keys | rebuilt at 0.75 | rebuilt at 0.5 |
|---|---:|---:|
| lookup, hit | 121–140 ns | **108–126 ns** |
| lookup, miss | 130–151 ns | **88–101 ns** |
| fill, grown | 182–221 ms | 240 ms |
| fill, `reserve`d | 120–145 ms | 114–129 ms |
| bucket table, grown | 5.8 MB (load 0.69) | 11.5 MB (load 0.35) |
| bucket table, `reserve`d | 5.3 MB | 8.0 MB |

So it is a trade with a loser: an UNRESERVED fill of a very large hash is ~20 % slower
(one more rebuild, of a bigger table) and the bucket array doubles — 3–6 bytes an entry
beside the ~16+ the entry itself takes.  Lookups win at both sizes, and `reserve(h, n)`
removes the fill's half of the price.  The hash-size cell
(`tests/scripts/pln110-size-hash.loft`) exists to notice exactly this kind of change and
did NOT: it sampled at 3, 13 and 39 entries, which both rules answer alike.  It samples
both sides of every rebuild now, and fails at its first rung under the old rule.

**Declined, with its price.**  The digest becomes a bucket number by `digest % count`, a
64-bit division on every lookup's critical path.  Priced by a temporary probe (a
multiply-shift reduction in its place): `hash_update` −10 %, `hash_find` −7 %, removal,
text keys and the group −3 %.  It is part of the on-disk placement contract — the probe
moves every entry of every stored hash, and an exact reciprocal needs a per-table number
the table header has no word for — so it stays, in ONE place now (`hash::home_bucket`)
where a future format revision would change it.

**Not built: the append half of L6.**  A keyed append walks the type table per element
(`nullable_field_parent`, `sub_record_type`, `field_ref`, twice — once to mint, once to
finish): ~300 of an insert's 2,100 instructions, est. −8 % on `hash_fill`.  On `--native`
its emission runs through the mint and push rewrites (`hoist::mint_path`), so a typed
entry there is an emitter change with those rewrites' interplay to prove; a per-field
memo in the runtime would serve both backends but has to be invalidated by every path
that re-lays a type out.  Neither is a change to make at the end of a pass.

**Verification.**  The cells (`tests/scripts/158-keyed-fast-paths.loft`) pass on both
backends as built, with all four switches set, and under `LOFT_KEYED_VERIFY=1`, which now
also answers every typed lookup through the general entry and checks a removal's
recognised slot against the entry's own index; the interpreter's script corpus passes
under it.  Two more sabotages: a removal that never recognises its entry fails six cells
(`test_158_hash_remove_reinsert` "len 30", and every displacement — it unlinks through
the same walk); a typed lookup hashing the wrong value fails the native run, panics under
the verify naming both answers, and passes under `LOFT_NO_TYPED_KEYED=1`.

## What L8 bought (2026-09-24)

The walk row did not exist before this pass: `for e in hash` visits in KEY order (`Col-Order`),
an order built per pass by `hash::records_sorted` — a bucket sweep, then `sort_by(keys::compare)`.
Profiled on the row alone (`perf`, the walk 600 times over 5,000 records): `keys::compare` 54 %,
the sort's own machinery 15 %, the bucket sweep 5 %, the walk itself 21 %.  The comparator
per pair was two `store()` lookups, a `match` on the key's kind and two typed reads — for a
key that is one word.  Decorated (`keys::sort_records`): the key is read once per record
through `fast_key_of` into `(i64, u32)` or `(&str, u32)` pairs, `sort_unstable` orders them,
and the index is the tie-break, so equal keys keep the bucket order the stable sort kept (a
hash has none; the tie-break is what makes the two forms one order).  `hash_walk` 540 → 257
µs/op, 5.93× → 2.83×; the probe alone −54 %.  The switch is the family's
(`LOFT_NO_FAST_ORDER`), the falsifier the family's (`LOFT_KEYED_VERIFY` asserts the decorated
order against `keys::compare` pair by pair; the 188 corpus cells that walk a hash run clean
under it, and `a-hash-walk-orders-every-key-width` pins the four widths).  A float, a
compound or a 1-byte key keeps the comparator.

What remains of the row is the walk: 22 ns per element through `OpStep` → `step_ordered` →
`arena::slot` and two field reads, against the twin's 6 ns `HashMap` lookup — the
record-backed element read, § What is left's territory, not the sort.

## What is left

A lookup is 444 instructions against Rust's 177, and ~3.5× in time — the time ratio is
the higher one because the walk is a chain of DEPENDENT loads (table header → bucket →
chunk directory → entry) behind a division, where hashbrown reads one control group.

| | lever | what it is worth |
|---|---|---|
| — | the division in `home_bucket` | 7–10 % of a lookup; a store-format revision |
| — | a typed keyed APPEND (L6's other half) | ~300 of 2,100 instructions an insert |
| — | the arena's bookkeeping on insert (`arena::alloc` 176, `index_of` 93, the zeroing `memset` 90) | ~17 % of an insert; `index_of` disappears if the mint hands its index to the finish |
| — | the table rebuild re-hashing every entry from its record (`keys::hash`, ~1.4 per insert amortised) | ~11 % of an insert; hashbrown pays this too |
| — | `sorted`'s insert | a gap buffer or a chunked layout; the `memmove` is 50 % of the row |
| — | `index` as a red-black tree of store records | `put` + `balance` are 45 % of the row; a B-tree of inline keys is the structural answer |
| — | text keys | the key is COPIED into the record per insert and hashed through the byte-slice `write`; the twin borrows `&str` |

## Two findings about the measurement itself

**One element type shared by three collections put every row in record-backed mode.**
`Stores::finish` makes an element type record-backed as soon as ANY `hash` / `index`
field in the program holds it: a `sorted` over it becomes an `ordered` (4-byte record
ids), and a `hash` over it takes one store record per entry instead of an arena slot
(`record_new`).  The lane declared `hash<E[id]>`, `sorted<E[id]>` and `index<E[id]>` in
three unrelated structs, so it measured that mode everywhere without saying so.  It now
uses one element type per kind, and `grouped_fill_find` measures the group deliberately.
What the mode costs, measured: hash fill the same (1.13–1.18 vs 1.17 ms), hash removal +13–19 %
(a `Store::delete` per entry), and `sorted` → `ordered` is FASTER on a fill (1.33 vs
1.81 ms — it moves 4-byte ids, not 16-byte elements).  It is also a property of the
language worth knowing: a `vector<T>` field added anywhere changes the layout, and the
cost, of every `hash<T[…]>` in the program.

**The load factor at a given `n` depends on the first claim's block size**, so two
programs filling 5,000 keys can differ 1.7× on hits and 4.7× on misses with identical
code.  It is deterministic per program, so it is not run-to-run noise — but a lane's
ratio carries it.  L5 removes most of the spread.

## Method

`perf record --call-graph lbr` for the sampled shares (an LBR stack keeps stale outer
frames: attribute a sample to the FIRST routine walking up from the leaf);
`valgrind --tool=callgrind --toggle-collect='*::n_<routine>'` for exact instructions per
routine, with `--dump-instr=yes` and the symbol's entry address for exact call counts
(callgrind's own call counts are not gated by the toggle); `size(h)` printed inside the
lane for the table's real load; a probe-length simulation over the real `key_hash` to
separate "bad hash" from "high load" (the hash is uniform).
