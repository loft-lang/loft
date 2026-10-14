# vector-write — what an insertion sort's inner loop pays, and the three levers

An ANALYSIS of `bench/10_sort` (3.45× its Rust twin on the 2026-10-02 macOS run), the slowest
of the original engine benches.  Every number is `--native-release` on arm64 macOS, priced by
editing the emitted Rust with `bench/portal/hand_price.sh`, the hash unchanged throughout.

## The finding

The bench's temporary (3 000 integers per op) is not the cost: building it is O(n), the
sort is O(n²) — about 2.25 M element shifts per op.  The cost is the inner loop
`for _ in 0..is_i { if arr[j - 1] > key { arr[j] = arr[j - 1]; j -= 1; } else { break; } }`,
and none of it needs a new representation: a slice loop over the SAME store bytes runs at
1.26× Rust.

| form of `insertion_sort` | ns/op | vs Rust |
|---|--:|--:|
| as emitted | 2.04 M | 3.4× |
| `j - 1` computed once | 1.78 M | 3.0× |
| … and plain instead of checked arithmetic | 1.50 M | 2.5× |
| … and the writes straight through the held element base | 1.21 M | 2.0× |
| the loft source as `while j > 0 && arr[j - 1] > key`, unedited | 1.19 M | 2.0× |
| a Rust slice loop over the store's elements (the ceiling) | 0.76 M | 1.26× |
| the Rust twin | 0.60 M | 1.0× |

## Why — read off the assembly

LLVM already merges the two `j - 1`s and keeps the header and base out of the loop.  What
remains per step: the null-sentinel select of `op_min_int` (`j == MIN ? MIN : j - 1`), a bounds
test on the read and on the write, the separate trip counter, and a locked-store copy of the
loop.  The select is the root: it breaks LLVM's induction analysis on `j`, so no range fact
about `j` survives and every check stays.  The `while j > 0` spelling supplies `j ≠ MIN`
itself, which is why that form is 40 % faster without any edit.  Inlining `vec_set_at` changes
nothing (measured, reverted).

## Built — the cursor under counted loops is plain (2026-10-04)

`(R-Range)`'s accumulator clause now admits a literal step under any loops whose trips are
bounded (a counted loop: `hi - lo + 1` over its counter's range; nested loops multiply), not
only under a character walk.  `j -= 1` emits `wrapping_sub`: 2.24 → 1.93 M ns/op (−12 %).
formal/rewrites.md § The accumulator clause; cells `tests/scripts/158-walk-accumulator.loft`
a10, a12–a18.

## Built — an element read answers every index inline (2026-10-04)

The repeated `arr[j - 1]` was the first lever this page priced, proposed as a rule reusing the
first read.  The assembly showed LLVM already merging the two in-range LOADS; what it could
not merge was the outlined re-derivation each read carried for an index outside `[0, len)`
(`get_elem_hoisted_cold`, a call into the rlib that LLVM must assume writes).  Two of them
kept the inner loop at 17 instructions a step with the lock flag tested inside.  The
re-derivation is arithmetic on what the read already holds, so it is now folded in
(`vector::elem_index`, the definition `get_vector` uses): 13 instructions, the flag
unswitched out, 1.905 → 1.64 M ns/op (−14 %), hash unchanged — the hand-merged price with no
emitter rule, for every repeated element read.  formal/rewrites.md § (R-Cold), fold clause.

Priced beside it and not built: the write's lock test is a further −19 % (1.62 → 1.31 M) when
it is gone, and an unchecked write reaches 1.00 M — the second lever's prize.

## The write-lock test, priced on x86-64 by a build without it

`@FR-H-WriteLocked` is tested per element in two hoisted writers: `Stores::vec_set_at`
(`!h.locked || write_allowed(…)`) and `vector::rec_set` (`locked && !write_allowed(…)`).  The
flag is held beside the header or the address; the test is still in the loop.  Priced as a
CEILING by a scratch build of the same tree with both tests deleted (`git archive` to disk,
its own target directory, `bench/stats.py --routine … --loft … --lib-dir …`), both builds
timed in one sitting on the x86-64 laptop, every Rust lane within 1 % between them:

| row | this tree | no lock test | the test costs | ratio |
|---|--:|--:|--:|---|
| `10_sort` `sort` | 3.153 M | 2.370 M | 25 % | 3.53× → 2.65× |
| graphics `blend_pixel` | 1.668 M | 1.245 M | 25 % | 1.85× → 1.38× |
| `comprehension` | 10.48 k | 8.09 k | 23 % | 1.63× → 1.25× |
| `index_write` | 7.49 k | 6.32 k | 16 % | 1.95× → 1.64× |
| graphics `fill_rect` | 7.256 M | 6.519 M | 10 % | 2.68× → 2.44× |
| `record_update` | 10.28 k | 9.48 k | 8 % | 1.37× → 1.27× |
| `index_read` (control, no write) | 22.71 k | 22.64 k | 0 % | 1.91× |
| `grid`, `remove_front`, random `indices`, `hash_update`, `mesh_aabb` | | | within ±3 % | not this test |

So the test is all but 3–5 % of what `sort` and `blend_pixel` rose by on this host since the
row measured at `7df5a4f4b`, and it takes `sort` back under the bar; it is 16 of the 95 points
`index_write` rose by, and none of `grid`'s, `remove_front`'s or `indices`' — those rises
have another cause, not found.

**Built** in the push window's form: a locked header has no writable element
(`vec_set_at`'s bound is 0 for it) and a locked record address leaves `rec_set`'s fast path
by the same test as a null one, so the write carries no call and the cold path refuses it.
The emitted Rust is unchanged; measured on x86-64 against the tree before it, same sitting:
`10_sort` 3.53× → **2.77×**, `blend_pixel` 1.84× → 1.44×, `comprehension` 1.62× → 1.34×,
`index_write` 1.97× → 1.64×, `fill_rect` −9 %, `index_read` flat — most of the ceiling above.
Guard `tests/locked_writes.rs` (cells from the end and past the end of a locked vector, and
the record writer in a development run).

## Next — one lever, priced, not built

1. **No bounds test where loft proves the index in range** — the rest of the gap
   (1.62 → ~0.8 M).  The facts are loft's: `is_i` runs over `1..len(arr)`, the hoist already
   proved the loop cannot change `arr`'s length, and `j - 1` stays in `[0, is_i - 1]`.  It needs
   a RELATIONAL fact (`index < len(v)`), which `generation::range` does not carry today — its
   ranges are numeric.  `(R-BoundedNest)` has the unchecked read for one accumulate shape;
   this is its general form.  Design before building: which relations (`< len(v)` of a vector
   whose length the hoist proved stable), where they come from (a counter's end being
   `len(v)`; a cursor seeded from such a counter and stepped down under a bounded loop), and
   the falsifier (`LOFT_HOIST_VERIFY=1` compares the unchecked read with the checked one).
