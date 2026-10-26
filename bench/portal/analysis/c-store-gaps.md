<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# c-store-gaps — where the C99 lane loses, by mechanism

An ANALYSIS of the C lane against the Rust twins (`bench/stats.py --lanes rust,c,cship
--samples 5`, laptop), each routine over 3.5× set beside its committed native ratio
(`bench/portal/results/laptop.tsv`) and profiled as C: `-O3 -g -fno-omit-frame-pointer
-fno-inline-functions-called-once`, `perf record --call-graph fp` on the efficiency cores, every
sample folded under the routine `main` called and attributed to its innermost runtime frame.
Every run is filed under subject `c-store-port` (`make perf-local ARGS="--subject c-store-port"`).

## The finding

**The C is behind loft's own native build, not behind the store model.**  Native pays the same
store layout, claims bitset, null sentinels and SipHash; what it has and the C lacks is the Rust
generator's rewrites and a few runtime routines with a faster algorithm.  So every gap is a PORT,
with native as its oracle.

**Checked arithmetic is not the cost.**  The `cship` lane drops every arithmetic check; for 28 of
the 32 routines over 3.5× it moves the ratio by under 4 % (`build_vis` 27 %, `enum_match` 11 %,
`char_roundtrip` 8.5 %, `truncate_to` 8 % are the exceptions).  `loft_add` looks large in a
profile only because the inlined loop body's lines are attributed to it: the overflow builtin in
its place moved bench 14 by −2.3 % of instructions and bench 16 by −0.2 %.

## Built

| step | what | effect (c / rust) |
|---|---|---|
| return-buffer adoption | `@FR-R-RetAdopt` read from `hoist::ret_adopt` | `split` −23 % instructions |
| push window | counted loops (`hoist::push_loop`, `mint_loop`) reserve once, push through a `loft_win` | push 4.21 → 0.39, record_append 7.46 → 0.97, f32_build 7.90 → 0.64, grid 8.56 → 3.02, comprehension 4.93 → 2.48, mesh_emit 6.05 → 3.58 |
| complete-write mints | no default image copied where `hoist::complete_writes` proves every literal complete | inside the windows above |
| flat copy | `@FR-R-FlatCopy`: a record owning no heap gets no claim walk on copy or release | bench 17 −13.4 % instructions, hashes equal |

After the window (before the flat copy): median 3.40× → 2.89×, routines over 3.5× 38 → 32.

## What is left, by mechanism

c/rust is the checked C lane after the push window; nat is the committed native ratio.  The
bug bar reads `cship` (bench/README.md § When a gap is a bug), which is within 4 % of `c` for 28
of these 32, so the lists coincide.

| class | routines (c/rust · nat) | where the C time goes | native has | half · size |
|---|---|---|---|---|
| **record copy and record results** | slope_path_with_undo 19.1 · 3.4, truncate_to 7.9 · 1.7, emit_to_material 6.0 · 1.4, resolve_move 4.4 · 1.5, reload_and_record 6.7 · 2.6, mapfile_to_painted 6.4 · 2.2 | `loft_copy_claims` 28–52 % (now partly the flat copy); a function returning a record by value (`map_get_hex`) mints a store per call (`loft_database_typed`), copies the element in, and the caller reads one field | value records (`@FR-R-ValueRecord`), the hand-over and rotate (`@FR-R-HandOver`), apart records (`@FR-R-Apart`) | generator · M |
| **text walk and text build** | split_walk 12.0 · 1.0, join 10.2 · 1.3, replace 9.6 · 0.8, split 9.6 · 1.1, trim_lower 6.1 · 1.4, char_roundtrip 8.6 · 2.4 | `join` copies every element twice per pass (`loft_text_own` then `loft_text_set`, a malloc and free each); `replace` scans and pushes byte by byte; `split`/`trim_lower` walk a character at a time through `loft_text_char` + `loft_char_len`; `char_roundtrip` rebuilds `big + "{c}"` per character | `@FR-R-TextBorrow` (a walked element is a `&str`), `@FR-R-LazySplit` (split_walk builds no vector), `@FR-R-CharWalk` (ASCII arm), Rust's `str::replace` | generator S–M per rewrite, `replace` runtime XS |
| **float text** | quadratic_out 4.8 · 0.6, parse_num 7.4 · 1.0; the 40–50 % `snprintf` share of `catalog`, `panel_build`, `format_specs` and the par rows | `loft_push_shortest`: up to 17 `snprintf("%.*e")` + `strtod` round trips per number; `strtod` for every parse | Rust's shortest-digits printer and its fast parse | runtime · S (Ryu/Schubfach in C99) |
| **element access across calls** | talus_relax 6.8 · 1.2, build_vis 6.5 · 2.1, hydro_pitfill 4.4 · 1.9 | `loft_at`/`loft_vec_get`/`loft_vec_len` per access: a loop that calls a store-writing function (`shed`) holds no header, and the callee re-derives everything | `@FR-R-Header`'s kept clause (headers handed into `__kh` twins), nested headers (`@FR-R-RecPtr`) | generator · M |
| **uncounted pushes** | enum_match 9.2 · 2.4, binary_read 16.5 · 2.8, mesh_emit 3.6 · 1.9 | `loft_vec_append` + `loft_copy_bytes` per element in loops the counted window declines (a `while`, a push under `if`); `binary_read` also mallocs and frees a buffer per 2-byte read and reads the file's position through its store record | `@FR-R-Push` push headers (no reservation, a growth arm), the scalar file read | generator S + runtime XS |
| **keyed walk** | hash_walk 8.6 · 1.6 | `qsort` with a function-pointer compare over the entries, then a recursive merge sort through the generic key interpreter, per walk | the same orders (`@FR-Col-Order`), monomorphised `sort_unstable_by_key` | runtime · S |
| **kernels** | sum 6.4 · 2.1 | `loft_vector_sum_int` reads each element through `loft_vec_get` | `sum_blocks_i64`: base once, blocks a magnitude bound proves overflow-free | runtime · XS |
| **par** | par 10.9, par_text 8.8, par_reduce 8.2, par_records 6.5, par_map_float 6.3 | the C runs the rows one after another | threads | @PLN203 C4, not store work |
| **near native** | hash_update 3.9, composite_hash 3.8, hash_find 3.7, hash_fill 3.5, map_json 3.6 | SipHash and the probe, the JSON tokenizer | the same algorithms; 1.4–1.6× of native | — |

## The order to take them

1. **Record copy and record results** — the worst routine (19×) and five more; native's value
   record and hand-over remove the stores and copies outright.  Re-measure after the flat copy
   first: it already removes the claim walk of every heap-free copy.
2. **Uncounted pushes** — the push header for loops the counted window declines, beside the
   window already built; then the small file read.
3. **Float text** — self-contained runtime work with no generator change, and it is 40–50 % of
   four more routines under the bar.
4. **Text walk** — borrowing a walked text is the shared piece of join, split and trim_lower.
5. **Element access across calls**, the kernel, the keyed walk.

Each fix is hand-priced on the emitted C before it is built (`perf stat -e instructions` on a
fast core: timings of a single short run here swing several-fold).
