<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# c-store-gaps — where the C99 lane loses, by mechanism

An ANALYSIS of the C lane against the Rust twins (`bench/stats.py --lanes rust,c,cship
--samples 5`, laptop), each routine over 3.5× set beside its committed native ratio
(`bench/portal/results/laptop.tsv`) and profiled as C: `-O3 -g -fno-omit-frame-pointer
-fno-inline-functions-called-once`, `perf record --call-graph fp` on the efficiency cores, every
sample folded under the routine `main` called and attributed to its innermost runtime frame
(since then `make c-profile`, which does this with DWARF call graphs).
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
| block sum | `vector::sum_blocks_i64` ported into the `sum` kernel | sum 6.8 → 2.0 |
| text runtime | `loft_replace` by `memchr`; `.P` in exact integers; the shortest float form from 15 digits with its neighbour step; Clinger's fast parse; an inline hash walked by one sweep and an introsort | replace 9.6 → 1.50, quadratic_out 4.75 → 0.53, format_num 1.81 → 0.36, format_specs 2.80 → 1.10, hash_walk 8.6 → 2.97 |
| repeat fill | `Stores::fill_from_template` ported: one exact resize, doubling block copies | enum_match 12.63 → 2.08 |
| loop verdict | native's `hoist::hoistable`: callee headers through `__inv` twins, bases re-derived under growth unless `StoreFacts` proves the store apart, the loop versioned on distinct stores, record push headers (`loft_ph`) | talus_relax 6.42 → 1.65, mesh_emit 5.57 → 1.20, build_vis 4.75 → 2.31, hydro_pitfill 3.41 → 1.66, sort_floats 2.54 → 1.75; reload_and_record unmoved (4.2: its time is the ordered hash walk and the record results) |
| held file cursor, deep window | `@FR-R-FileHold` (`lower::file_hold`) with a 4 KB cursor buffer and a release on every exit; `hoist::deep_window_ok`'s window for uncounted pushes; unheld reads into a stack buffer | binary_read 16.4 → 1.60 |
| text walks | `@FR-R-LazySplit`, `@FR-R-TextBorrow` (walk variables and the plain locals of `hoist::borrowed_store_texts`), `@FR-R-CharWalk` (the ASCII arm, the null test asked once) | priced: join 10.2 → 2.6, split walk −38 % cycles |

After the window (before the flat copy): median 3.40× → 2.89×, routines over 3.5× 38 → 32.

## Priced, not built

Hand edits of the emitted C, the hash unchanged each time (bench rows on cpu 6, the probe by
`perf stat`):

| edit | routine | before → after | c/rust |
|---|---|---|---|
| a walked vector element read as a view (`@FR-R-TextBorrow`): no `loft_text_own` + `loft_text_set` per element | join | 58.7 → 14.9 µs | 10.2 → 2.6 |
| `loft_replace` jumps between matches with `memchr` and pushes the runs whole (runtime only) | replace | 27.9 → 3.8–4.1 µs | 9.6 → 1.3–1.4 |
| the ASCII arm of the char walk, the null test and the length asked once, `self` borrowed instead of copied (`@FR-R-CharWalk`) | split probe (`target/cstore/sp.loft`) | 1.10 G → 0.68 G cycles | ≈ 9.6 → 6 |

After the walk fix, `split` spends ~25 % in the element push and its text claim
(`loft_set_text`, `loft_vec_append`, `loft_st_claim_inner`, `loft_bits_insert`): the
uncounted push header and the claim path are its next two steps.

## The next set, profiled

`make c-profile ARGS="<bench> --lane cship"` (DWARF call graphs, each sample charged to its
routine and then to its runtime function), the emitted C read beside native's
(`loft --ship --native-emit`), and each candidate priced with `bench/portal/ab.py` on the kept
unit, the hashes equal.

| mechanism | routines | what the profile shows | native has | priced | size |
|---|---|---|---|---|---|
| **the repeat fill** | enum_match | `[for _ in 0..4096 { 0 }]` is `loft_vec_append_copy`: an append, a byte copy and a finish per element, 36 % of the routine; the C also grows by doubling where the interpreter resizes once, so the store layouts differ | `Stores::fill_from_template`: one append, one exact resize, the template copied in doubling blocks, claims per element only for a type that owns heap | enum_match 186 → 44.7 µs (0.24×) | runtime XS |
| **record addresses** | mesh_emit, emit_to_material, slope_path | every field write is `loft_field` + `loft_set_float` through the store (9 per vertex in mesh_emit) | `@FR-R-RecPtr`: one address per pushed element, writes through it | — | generator S |
| **the held file cursor** | binary_read | `loft_read_file` 43 %: per 2-byte read the format checks, the position read from and written to the File record, a handle lookup, a `malloc`/`free` of a 3-byte buffer | `@FR-R-FileHold` (`lower::file_hold`, the cursor held across the loop) and a push window on the `while` | — | generator S + runtime XS |
| **stack vectors** | mesh_emit (`ox`, `oz`) | element reads through `loft_vec_get` | `@FR-R-Apart` (`ApVec`) | — | generator M |
| **record results** | resolve_move, emit_to_material, slope_path | `loft_store_make`, `loft_copy_record`, `loft_copy_claims`, `loft_remove_claims_mode` | value records, hand-over, apart records | — | generator M |
| **ordered hash walk** | reload_and_record, mapfile_to_painted | `loft_hkey_sort` + `loft_hkey_lt` 15–31 %; the rest is the walk's lookups and pushes (the loop verdict) and the copy into the history (record results) | `hash::inline_sorted::<N>`, monomorphised per key count | a 2-key element (24 bytes for 40): −7 %, inside the noise — not the lever | runtime S |

With the loop verdict built, record results are what most of the remaining routines over the
bar share: slope_path, emit_to_material, truncate_to and mapfile_to_painted copy records per
result; record addresses (`@FR-R-RecPtr`) are the smaller half.

**A finding the timed sweep made.**  `len(text)` counts characters, and a loop that tests
`i < len(t)` counts the whole text per iteration on EVERY backend — the self-tail-call cell of
`R-TailSelf` took 5.7 s as C, 1.4 s on the interpreter and seconds on native.  `(R-TextLen)` in
`formal/rewrites.md` is the rule (a memo per loop, cleared by every write).  The C's
byte-at-a-time `loft_chars` is the smaller half: a word-at-a-time count priced the same cell
4.2 → 2.5 s; Rust's lane count is the form to port.

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
5. **Element access across calls**, the keyed walk.

Each fix is hand-priced on the emitted C before it is built (`perf stat -e instructions` on a
fast core: timings of a single short run here swing several-fold).
