# The C backend under 3× its Rust twin — making 3× the bug bar for the next release

**The owner's goal (2026-10-28):** at the next release, a routine over 3× its Rust twin on the
C backend is a bug issue (one issue per problem class, as `feedback`: ship gaps are bugs, the
checked lane's cost is optimised, not filed).  The C is the one compiled target going forward;
the rustc `--native` build is out of scope and will be shelved.  `DESIGN_DECISIONS_PLATFORM.md`
C136 already makes a routine over 3× a defect, for native's `--ship`, with the stable release
waiting only on 5×; moving that entry to the C and to 3× for the release is the owner's call
and is not made here.

## Assessment

**Reachable, on the SHIP lanes, both compilers — with margin first.**

| lane | median | worst (row) | over 3× | over 2.7× |
|---|---|---|---|---|
| gcc ship (`cship`) | 1.41× | 2.78× (`map_json`) | 0 | 1 |
| clang ship | 1.37× | 2.80× (`interp_fib`) | 0 | 2 (`interp_fib`, `par`) |
| gcc checked (`c`) | 1.48× | 3.03× (`slope_path_with_undo`) | 1 | 3 |
| clang checked | 1.60× | 3.06× (`interp_fib`) | 1 | 5 |

(`python3 bench/stats.py --lanes rust,c,cship [--cc clang]`, 82 routines with bench 20's three,
commit `02096be4a`, box held quiet.)

Rows that moved for LAYOUT, not mechanism — each reproduced with its switch, the hot function's
instructions identical across the two builds: `binary_read` (gcc ship +15.6 % with
`@FR-R-Inline`: its read loop sits in a `.cold` section whose placement moved) and
`binary_map` (+5.3 %, the same bench); `fibonacci` (clang +6.6 %: `n_fib` unchanged, its start
moved from a 32- to a 16-byte boundary).  Noted with evidence, not coded around.

**The bar needs margin to be reliable.** `stats.py` repeats within a few percent, but a compiler
upgrade or a layout shift moves single rows by 5–10 %.  With the worst ship rows at 2.93× and
2.80×, a 3× bar would flap.  The bar can be declared once a full measurement shows **every ship
row at or under about 2.7× on both compilers** — three ship cells stand above that line now
(`map_json` gcc, `interp_fib` and `par` clang).  The checked lanes keep 3× as a goal, not a bug
bar: their extra over ship is the cost of the guarantees, which is optimised, not filed.

## What is left, in order

1. **`slope_path_with_undo`** (gcc ship 2.48×, gcc checked 3.03×; clang 2.12× / 2.69×).  Per path step the C runs
   ≈ 1,053 instructions against Rust's ≈ 434 (four chunk walks, two seven-field hex reads, one
   write, one 11-field append — the same work; measured 2026-10-29 on `7517d13ad`).  Store
   addressing inside the map helpers prices at zero; the gap is `loft_rem`'s null test plus the
   helpers staying out of line (≈ 45 %), the undo push through the store (≈ 20 %), and a
   remainder not yet attributed — [`top5-memory.md`](top5-memory.md).
2. **`map_json`** (2.78× gcc ship, 2.81× gcc checked; clang ≈ 2.45×).  Generic runtime JSON (`loft_json.h`), not emitted
   code: the lexer `loft_jl_token` 17 %, `loft_pr_struct` 12 %, `loft_jp_validate` 8.4 % (every
   string is validated after the lexer already scanned it — skippable for a string with no
   backslash and no control byte, the only bytes the validator acts on), `loft_jl_quoted` 6 %,
   a heap text per token (keys included: `malloc` / `free` / `loft_jv_free` ≈ 6 %), and
   `strlen` of every field name while writing (`loft_show_name`, ≈ 8 %).  Plain runtime C, no
   rule work — the lowest-risk margin of the three.
3. **`par` / `par_reduce`** (`par` 2.62× gcc, 2.74× clang ship; `par_reduce` clang checked 2.89×).  `par_reduce`'s checked cost
   is a hash-mixing loop, `h = (h * K + c) & M; h = h ^ (h >> 13)`, 48 times per element: `h` is
   loop-carried, so no static proof applies.  A guard testing `h ∈ [0, M]` once at the loop's
   entry would make the loop plain — an extension of `@FR-R-GuardedChain` (a loop-carried masked
   local as a guarded leaf, `&` `^` `>>` beside `+ - *`), general for PRNG and noise loops.
   `par`'s gap is NOT the worker hand-off (under 0.4 % of its cycles): it is the NaN test the
   source's `?` asks for per Newton step (≈ 42 %) and the row loop staying scalar where Rust runs
   two rows per `divpd` (≈ 55 %) — [`top5-memory.md`](top5-memory.md).

4. **`interp_fib`** (bench 20, the self-host interpreter's loop: clang checked 3.06×, ship 2.80×;
   gcc 2.29× / 2.24×).  Its calls go through three frame vectors and a `put` per push; gcc took
   `put` into the loop under `@FR-R-Inline` and the op fetch through a held header
   (`@FR-R-Header`'s rebind clause).

## Measuring

- Both compilers, both lanes: `python3 bench/stats.py --lanes rust,c,cship` and again with
  `--cc clang`; about 20 minutes each.
- On the dedicated laptop, with the box quiet: tell the other local sessions before and after
  (they hold builds and tests in between).  A run taken under load reads 10–50 % wrong.
- Keep builds and scratch off `/tmp`: every session runs as one user and shares one `/tmp`
  quota.  Use `TMPDIR=<checkout>/target-exp/tmp` and a `--target-dir` on the root disk.
- Price a change by instruction count (`valgrind --tool=callgrind --toggle-collect=<fn>`)
  while the box is busy; confirm with time once it is quiet.

## Built on the way (branch `laptop-c-perf`, 2026-10-28)

Format writes straight into the destination text (`char_roundtrip` −27 %); SipHash helpers and
the integer-key lookup inlined; call operands evaluated in place (a guarded chain fed a field or
a key had silently stayed checked); the key-ordered hash walk sorts by radix (`hash_walk`
−61 %); records read and written through their address — narrow fields, the last-use clause,
record parameters, value-record reads, copied sub-records; struct-enum literals skip the
prefill; reserved vector room left unzeroed; the file cursor in registers; and `@FR-R-Apart`
instances 2, 3 and 4 ported from native (the short-lived second layout beside the store:
`truncate_to` −70 %).  Then: a read-only record parameter passed as a `const` pointer
(`@FR-R-ValueRecord`), a checked multiply by a constant that no longer divides at run time, small
functions declared `inline` (`@FR-R-Inline`), and a vector the loop rebinds kept under a header
derived again after each rebind (`@FR-R-Header`): bench 20 −69 % (gcc) / −17 % (clang),
`slope_path_with_undo` gcc ship 2.92× → 2.48×.  Details in the commits and in
`doc/claude/formal/rewrites.md`.
