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

| lane | median | worst (row) | over 3× |
|---|---|---|---|
| gcc ship (`cship`) | 1.42× | 2.93× (`slope_path_with_undo`) | 0 |
| clang ship | 1.42× | 2.70× (`par`) | 0 |
| gcc checked (`c`) | 1.52× | 3.30× (`slope_path_with_undo`) | 1 |
| clang checked | 1.65× | 3.26× (`enum_match`) | 2 |

(`python3 bench/stats.py --lanes rust,c,cship [--cc clang]`, 79 routines, 2026-10-28 09:00,
before the batch below.)  Since that run: `truncate_to` −70 % / −72 % (gcc / clang),
`enum_match` −11 %, `binary_read` −11 % (clang), `build_vis` −12 % / −8.5 % and `build_walls`
−6.5 % / −7.8 % (instructions), `reload_and_record` −13 % (gcc).

**The bar needs margin to be reliable.** `stats.py` repeats within a few percent, but a compiler
upgrade or a layout shift moves single rows by 5–10 %.  With the worst ship rows at 2.93× and
2.80×, a 3× bar would flap.  The bar can be declared once a full measurement shows **every ship
row at or under about 2.7× on both compilers**.  The checked lanes keep 3× as a goal, not a bug
bar: their extra over ship is the cost of the guarantees, which is optimised, not filed.

## What is left, in order

1. **`slope_path_with_undo`** (gcc ship 2.93×, gcc checked ≈ 3.2×).  Per path step the C runs
   ≈ 1,350 instructions against Rust's ≈ 430 (four chunk walks, two seven-field hex reads, one
   write, one 11-field append — the same work).  The checks are ≈ 5 % of it (priced: plain
   arithmetic in the map helpers saves 5 % gcc, 3 % clang).  The rest is store access spread over
   the helpers: each map walk re-derives the chunk vector's header and resolves each chunk and
   hex through the store.  Taking the walked element's address from the header was priced and
   is SLOWER in the C (the compilers already hoist the buffer pointer).  The next lever is
   structural: the map helpers' chunk walks.
2. **`map_json`** (2.80× on both gcc lanes).  Generic runtime JSON (`loft_json.h`), not emitted
   code: the lexer `loft_jl_token` 17 %, `loft_pr_struct` 12 %, `loft_jp_validate` 8.4 % (every
   string is validated after the lexer already scanned it — skippable for a string with no
   backslash and no control byte, the only bytes the validator acts on), `loft_jl_quoted` 6 %,
   a heap text per token (keys included: `malloc` / `free` / `loft_jv_free` ≈ 6 %), and
   `strlen` of every field name while writing (`loft_show_name`, ≈ 8 %).  Plain runtime C, no
   rule work — the lowest-risk margin of the three.
3. **`par` / `par_reduce`** (2.6–2.75× ship; clang checked 2.86×).  `par_reduce`'s checked cost
   is a hash-mixing loop, `h = (h * K + c) & M; h = h ^ (h >> 13)`, 48 times per element: `h` is
   loop-carried, so no static proof applies.  A guard testing `h ∈ [0, M]` once at the loop's
   entry would make the loop plain — an extension of `@FR-R-GuardedChain` (a loop-carried masked
   local as a guarded leaf, `&` `^` `>>` beside `+ - *`), general for PRNG and noise loops.
   `par`'s gap is the worker hand-off.

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
`truncate_to` −70 %).  Details in the commits and in `doc/claude/formal/rewrites.md`.
