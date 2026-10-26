<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# c-store-gaps — where the C99 lane loses, by mechanism

An ANALYSIS of the C lane (`bench/stats.py --lanes rust,c --samples 5`, laptop, 2026-10-26, at
`99b797739`): 79 routines against their Rust twin, median 3.40×, 37 over 3.5×.  Each over-bar
routine was set beside its committed NATIVE ratio (`bench/portal/results/laptop.tsv`), and the
benches 12–18 were profiled as C (`-O3 -g -fno-omit-frame-pointer
-fno-inline-functions-called-once`, `perf record --call-graph fp` on the efficiency cores),
each sample folded under the routine `main` called and attributed to its innermost runtime
frame (`target/cstore/prof/tools/fold.py`, scratch).

## The finding

**The C is behind loft's own native build, not behind the store model.**  Of the 37 routines
over 3.5× (hash_fill, at 3.50×, is listed too), 33 are more than 2.5× slower than `--native` on
the same program, most 3–15×.  Native
pays the same store layout, claims bitset, null sentinels and SipHash; what it has and the C
lacks is (a) the Rust generator's rewrites that keep a vector's header, a walk's text or a record
copy out of the loop, and (b) a few runtime routines the C implements by a slower algorithm.
So the work is a PORT in both halves, each item with a native counterpart to measure against.

## By mechanism

c/nat is the C's time over native's; the routines are those over 3.5× of Rust.

| class | routines (c/rust · c/nat) | where the C time goes | native has | half |
|---|---|---|---|---|
| **vector push** | push 4.2 · 14.5, f32_build 7.9 · 8.9, record_append 7.5 · 7.5, grid 8.6 · 5.6, comprehension 4.9 · 4.1, mesh_emit 6.1 · 3.2, enum_match 17.7 · 7.5 | `loft_vec_push_slot` / `_push_int` / `_push_single` / `loft_vec_append` 35–60 %, a `memcpy` call per element 8–28 % | `@FR-R-Push` push window: base, length and room held across the loop, the length written back once | generator (+ runtime) |
| **text walk / build** | split_walk 12.1 · 11.7, replace 9.6 · 12.0, split 9.6 · 8.9, join 10.4 · 8.3, trim_lower 6.0 · 4.5, char_roundtrip 8.3 · 3.5 | per character `loft_text_char` + `loft_char_len` + checked `loft_add`; `loft_push` / `loft_reserve` / memmove in replace and join | `@FR-R-CharWalk` ASCII arm with the null test asked once, `@FR-R-LazySplit` (split_walk builds no vector), a walked parameter borrowed | generator |
| **element access** | talus_relax 6.8 · 5.5, sum 6.5 · 3.1, build_vis 6.3 · 3.0, binary_map 5.0 · 3.1, hydro_pitfill 4.4 · 2.3, truncate_to 7.7 · 4.5 | `loft_at` / `loft_vec_get` / `loft_vec_len` / `loft_store_of` per access: a table index, the vector field, the record, the bound, every time | `@FR-R-Header`'s kept clause and nested headers (`@FR-R-RecPtr`); the C holds a header only in a loop that grows no store, one level deep | generator |
| **record copy** | slope_path_with_undo 21.4 · 6.3, emit_to_material 6.3 · 4.7, resolve_move 4.4 · 2.9, reload_and_record 6.9 · 2.6, mapfile_to_painted 6.6 · 3.0 | `loft_copy_claims` 28–52 % — every text and sub-record of a copied record claimed again | move rewrites that never copy (`@FR-R-MoveAppend`, swaps, value records) | generator, then runtime |
| **float text** | quadratic_out 4.8 · 8.6, parse_num 7.3 · 7.7, catalog, panel_build, format_specs | `loft_push_shortest`: up to 17 `snprintf("%.*e")` + `strtod` round trips per number (glibc's bignum `__mpn_*`); `strtod` for every parse | Rust's shortest-digits printer and fast-path parse | runtime |
| **keyed walk** | hash_walk 8.6 · 5.3 (hash_fill 3.5 · 2.6) | `qsort` with a function-pointer compare over `loft_hent`, then a recursive merge sort through the generic key interpreter, per walk | the same orders (`@FR-Col-Order`), sorted by monomorphised `sort_unstable_by_key` | runtime |
| **file read** | binary_read 18.5 · 6.6 | `loft_read_file`: a `malloc` per read, the file's state read and written through its store record | the runtime's buffered handle | runtime |
| **par** | par 10.9 · 4.1, par_text, par_reduce, par_records, par_map_float | the C runs the rows in order | threads | @PLN203 C4, not store work |

Hash find / update / composite (1.4–1.6× native) are close: the SipHash and probe are a
faithful port and cost what native's do.

## The order to take them

1. **Vector push** — the most routines, the profile names the functions outright, and the
   shared admission already has the tier: `opt::header::hoistable_vectors` is called with
   `HoistTiers::default()` (push off), so a loop that pushes holds nothing.  Port the push
   window: a held `loft_vp` (store, record, length, room, base) refreshed by the out-of-line grow,
   the element written at a constant width (no `memcpy` call), the length stored at loop exit.
   Price first: hand-edit `14_stdlib_vector`'s `v_push` loop in the emitted C.
2. **Element access** — the held header across a loop that grows ANOTHER store (the kept
   clause), then nested headers; the same `loft_vh` plus the admission's facts.
3. **Text walk** — the CharWalk arm and LazySplit; a walked text parameter borrowed instead of
   copied into the loop's local (`loft_text_set(&for_text, self)` per call).
4. **Float text** — a shortest-digits printer (Ryu or Schubfach, C99) and a fast parse path.
   Self-contained, no generator change.
5. **Record copy** — first find which native rewrite removes each copy, then port it; tune
   `loft_copy_claims` only for the copies that remain.
6. **Keyed walk**, **file read** — small runtime changes, one routine each.

Not priced yet: every class above is read off a profile and a code comparison; each fix is
hand-priced on the emitted C before it is built (the `split` adoption predicted its −23 %
exactly that way).
