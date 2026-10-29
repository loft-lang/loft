<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# The five slowest C routines, by memory-model cost

An ANALYSIS of the five routines with the worst ship ratio on the C backend: which part of
each gap comes from loft's memory model (stores, records, headers, claims, per-value
allocation) and which part comes from something else.  Measured on commit `7517d13ad`
(branch `laptop-c-perf`, 2026-10-29).

## Method

- The table: `python3 bench/stats.py --lanes rust,c,cship`, then the same with `--cc clang`,
  82 routines, box quiet.
- The breakdown: `make c-profile ARGS="<bench> --lane cship"`, then a driver per routine (the
  emitted C with `main` cut down to that routine) compared against a cut-down Rust twin.
- Each candidate was priced by editing a copy of the emitted C, checking that the printed
  hash did not change, and counting `perf stat -e instructions:u,cycles:u` pinned to one
  P-core.  A second session was building during part of this, so cycle counts were noisy:
  each ratio compares runs from the same batch, and instruction counts carry the argument.

## The table

| routine | gcc ship | clang ship | gcc checked | clang checked | memory-model share of the gap |
|---|---|---|---|---|---|
| `map_json` | 2.78 | 2.42 | 2.81 | 2.41 | ≈ 20–25 % |
| `par` | 2.64 | 2.76 | 2.62 | 2.74 | ≈ 3 % |
| `interp_fib` | 2.07 | 2.72 | 2.28 | 2.94 | ≈ 75 % |
| `slope_path_with_undo` | 2.63 | 2.14 | 3.19 | 2.67 | ≈ 20 % |
| `grouped_fill_find` | 2.51 | 1.93 | 2.80 | 2.04 | ≈ 85 % |

## The shared mechanism: a header lost to the loop's own growth

A held vector header (`loft_vh`: the data address and the length) lets a loop read elements
the way Rust reads a `Vec`.  A loop that GROWS a vector holds no header for it, because the
append may move the data.  So every access goes back to the store: the thread-local table,
the store's buffer, the record, the length.  The append itself is the only point where the
data can move, so a header derived again right after that append stays correct.

| routine | where it bites | priced |
|---|---|---|
| `interp_fib` | the value stack and the frame vectors (`f_def`, `f_pc`, `f_base`): up to four store resolutions per interpreted op, mostly in `put` | held headers, derived again after the own append: gcc −40 % instructions, −31 % cycles; clang −35 % / −39 % |
| `grouped_fill_find` | the hash table's header is read back from the store on every insert (probe, `add_at`, `loft_h_grow_if_full`, which costs 57 instructions even when nothing grows) | not priced separately; the insert is 516 instructions against Rust's 159 |
| `slope_path_with_undo` | the undo push resolves the vector three times (`loft_vec_append`, `loft_rec_ptr` with a lock check, `loft_vec_finish`) | an inline push path with one resolution: −0.25 of the ratio, −69 instructions per step |

## Per routine

### `interp_fib` (bench 20)

Per run, fib-only drivers, hash `1a6d` in every build:

| build | instructions | cycles |
|---|---|---|
| Rust | 2.11 G | ≈ 0.43 G |
| gcc, as emitted | 4.74 G | 0.82 G |
| gcc, held headers | 2.84 G | 0.57 G |
| gcc, held headers + op address from the header | 2.71 G | 0.54 G |
| clang, as emitted | 5.47 G | 1.06 G |
| clang, held headers | 3.53 G | 0.65 G |
| clang, held headers + op address from the header | 2.99 G | 0.53 G |

1. **Stack and frame vectors resolved per access (≈ 65 % of the gap).**  See above.
2. **The op record's address resolved a second time** (gcc 7 %, clang 18 % of the gap).
   `ops[pc]` is read through the held header `h3`, then `loft_rec_ptr(op)` goes back through
   the store table for the same address.  An element taken from a held header has the
   address `h.base + pos - 8` already.
3. **The 16 % `memcpy` copies nothing.**  It is the inlined `memcpy` that `loft_get_int`,
   `loft_set_int`, `loft_u32` and `loft_pa_i64` use for unaligned access.  The run loop holds
   views only (`op`, `ops`, `callee_prog`).
4. **Why clang is slower than gcc:** the store-resolution chains raise register pressure,
   and clang spills the held header to the stack and reloads it on every op.  With both
   fixes, clang (0.53 G cycles) matches gcc (0.54 G).

What is left (≈ 1.25×) is not memory model: the op `match` is an if-chain, each arm maps the
null tag again, and the `?? panic` null tests.

### `grouped_fill_find` (bench 15, `k_grouped`)

Instructions per element (one insert and one lookup); IPC is the same in all three (3.24 gcc,
3.46 clang, 3.18 Rust), so the gap is instruction count, not cache misses.

| part | gcc | clang | Rust |
|---|---|---|---|
| whole element | 1090 | 963 | 443 |
| record mint (claim, zero, defaults, claims bit) | 158 | 152 | ≈ 8 |
| append to the vector of record ids | 104 | 113 | 0 |
| hash insert, growth included | 622 | 522 | 318 |
| lookup | 205 | 176 | 117 |

1. **The insert re-reads the table's header from the store (+269).**  Native's typed keyed
   path (`@FR-R-TypedKeyed`) already does this for lookups; inserts into a group with one
   integer-keyed hash member want the same.
2. **One store record per element (+150).**  Each element is claimed, zeroed, given its
   defaults and marked in the claims bitset.  Rust keeps the elements inline in a `Vec`.
3. **A second vector of record ids (+104).**  Rust has none.
4. **The allocator leaves its fast path for good (+88).**  `loft_bump_wild` refuses whenever
   the free tree is non-empty (`loft_store.h`, around line 840).  Growing the table and the id
   vector frees the old blocks into the same store, so from then on every claim walks the
   free tree (118 instructions) and carves a block.  Dropping that test: −8.0 % on gcc and on
   clang, same result.  It changes which block a claim gets, and the layout must stay the
   same on every backend (`@FR-H-Wilderness` holds the tail beside the tree on the
   interpreter and native), so the change belongs in the shared allocator rule.
5. **Lookup (+88):** the header decoded again per lookup, plus one extra SipHash round.

Not memory model: the digest (`loft_ref_digest` reads the key generically and hashes the seed
as a data word: 167 instructions on gcc against Rust's ≈ 85).  Growth itself is cheaper in loft
(it keeps the stored tags and does not rehash).  gcc is worse than clang because it keeps
`loft_group_finish` and `loft_hash_insert` out of line as a constprop clone with structs passed
through memory (≈ 48 per element), and does not fold `loft_sip_le(b, 7) | b[7] << 56` into one
load (≈ 24; an 8-byte `memcpy` there saves 2.0 % on gcc).

### `slope_path_with_undo` (bench 17, `c_slope_path`)

1,053 C instructions per path step against Rust's 434 (the 1,350 in `c-3x-bar.md` predates
the rebind clause).  The gap in ratio points (1.33 above Rust):

| cause | share | priced |
|---|---|---|
| `loft_rem` keeps its null test in the ship build, and the map helpers (`map_get_hex`, `map_set_hex`, `map_ensure_chunk`, `undo_push`) stay out of line | ≈ 45 % | together 1,053 → 717 instructions per step; the null test alone −115, inlining alone −118 |
| the undo push through the store (memory model) | ≈ 20 % | −69 instructions, −0.25 of the ratio; `loft_vec_append` inlined alone −0.12 |
| not attributed | ≈ 35 % | after every edit: 652 against 434 instructions; probably the 64-byte `loft_tup5` results returned through memory and the null tests on reads |

`s.us_redo = []` per push calls `loft_vec_clear_release`, which returns at once on an empty
vector (24 instructions); a length test before the call removes it.  The `memcpy` share (11 %)
is the `Hex` record's fields moving between results and the undo entry; Rust makes the same
moves.  Store addressing inside the helpers priced at zero: a walk through the header's base,
one header read for the hex element, frame push and pop removed — none of them moved the count
by more than 9 instructions per step.

### `map_json` (bench 17)

The C spends 0.71 ms writing and 3.70 ms parsing, against Rust's 0.36 ms and 1.19 ms; parsing
is 88 % of the gap.  Shares below are of the C's parse time.

1. **A temporary JSON value per scalar field (≈ 14 %).**  `loft_pr_into` zeroes a 96-byte
   `loft_jv`, `loft_jp_value` fills it, `loft_pw_walk` writes it into the store slot,
   `loft_jv_free` zeroes it again (`memset` alone 4.7 %).  An integer field read straight into
   its slot: parse 3.70 → 3.40 ms, routine −7.2 %, same hash.
2. **A heap text per string token, keys included (≈ 7–11 %).**  Rust allocates a `String` per
   key too, so about half of this is extra.  An escape-free string can stay a view of the input.
3. **Store writes per record (≈ 4.5 %):** `loft_pw_absent_fields` twice, a `calloc`/`free` of a
   `found` array, which a stack bitmap replaces.
4. **The copy of the parsed `Map` into the caller's buffer** (0.7 %).

Not memory model (≈ 75 % of the gap): the tokenizer is loft's general language lexer (number
bases, separators, identifiers, line ends) where Rust runs one byte loop; strings are validated
again after the lexer (`loft_jp_validate` 5.9 %) and numbers scanned twice; a field name is found
by a linear `strlen` + `memcmp` over every field (`loft_pr_struct` 16 %); the writer pushes byte
by byte and measures every field name with `strlen`.

### `par` (bench 11)

All threads summed, `--n 40`, on the cores `stats.py` gives the bench:

| build | instructions | cycles |
|---|---|---|
| C ship, gcc | 1.83 G | 2.78 G |
| C ship, clang | 1.91 G | 2.84 G |
| Rust | 0.45 G | 1.01 G |

99.6 % of the C's cycles are in the row loop (`loft_parbody0`).  The worker hand-off
(`loft_par_run`, thread start, the result buffer, `loft_par_drop`) is under 0.4 % of cycles.

- **Memory model (≈ 3 %):** the element read through the store (`loft_vec_get` then
  `loft_get_int`), about 20 instructions a row against 400 for the Newton loop.  A plain pointer:
  −3.6 % instructions, −1.5 % cycles.  Rust derives the value from the range and reads no vector.
- **The `?` on `x / guess` (≈ 42 %):** a NaN test and a branch per step.  Removing it:
  2.78 → 2.00 G cycles.  The Rust twin has no `?`, so this is a difference in what the two
  programs say as much as a cost.
- **No vectorisation (≈ 55 %):** Rust runs two rows per `divpd` with the 50 steps unrolled.
  The C stays scalar under gcc and clang even with the NaN test gone and the loop unrolled; gcc
  reports *"loop nest containing two or more consecutive inner loops cannot be vectorized"*.

## The order to build

1. **`@FR-R-Header`, own-append clause** — hold a header across a loop that appends to that
   same vector, and derive it again right after the append.  `interp_fib`, the undo push in
   `slope_path_with_undo`, and the insert path of `grouped_fill_find`.
2. **Element address from a held header** — `h.base + pos - 8` in place of `loft_rec_ptr`.
   `interp_fib` (clang most).
3. **The ship lane's integer `%` and `/`** — the null test stays only where an operand can be
   the sentinel; and a larger inline budget for small leaf helpers.  `slope_path_with_undo`.
4. **JSON parse into the slot** — a typed fast path per scalar field kind, keys compared as
   views, a stack bitmap for the found fields.  `map_json`.
5. **The allocator's fast path with a non-empty free tree** — a layout change, so it is made
   in the shared allocator rule on every backend at once.
6. **`par`'s NaN test and vectorisation** — not memory-model work; it is the loop shape gcc and
   clang refuse to vectorise.
