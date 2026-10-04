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

## Next — two levers, priced, not built

1. **One read for a repeated element read** — `arr[j - 1]` is read in the test and again in the
   body.  LLVM does not merge them because the read's cold path (a negative or out-of-range
   index) can raise, which writes state.  Priced together with the plain step: 1.93 → 1.62 M
   (−16 %).  A generation-time rule: a pure element read whose identical twin DOMINATES it —
   same vector root, same index expression — with no write to that vector, to any variable
   of the index, and no call between, reuses the first value; the first read must be
   unconditional where it stands (not under a short-circuit), so hoisting it adds no read.
   Reach: 43 sites in 21 files of the benches and libraries (an `if`/`while` testing `v[e]`
   and the body reading `v[e]` again), among them two more insertion sorts in `hex_recover`
   (`out[ps - 1] > out[ps]`).
2. **No bounds test where loft proves the index in range** — the rest of the gap
   (1.62 → ~0.8 M).  The facts are loft's: `is_i` runs over `1..len(arr)`, the hoist already
   proved the loop cannot change `arr`'s length, and `j - 1` stays in `[0, is_i - 1]`.  It needs
   a RELATIONAL fact (`index < len(v)`), which `generation::range` does not carry today — its
   ranges are numeric.  `(R-BoundedNest)` has the unchecked read for one accumulate shape;
   this is its general form.  Design before building: which relations (`< len(v)` of a vector
   whose length the hoist proved stable), where they come from (a counter's end being
   `len(v)`; a cursor seeded from such a counter and stepped down under a bounded loop), and
   the falsifier (`LOFT_HOIST_VERIFY=1` compares the unchecked read with the checked one).
