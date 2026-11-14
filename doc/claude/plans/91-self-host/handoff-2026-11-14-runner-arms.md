<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
<!-- size-exempt: a dated handoff record, read once (DOC_QUALITY § Maintainer docs 2) -->
# Handoff 2026-11-14 (night) — the runner's speed: arms for references, texts and two-byte slots

Branch `tuxedo-pln204-int32`, pushed.  The owner's goal for this arc: finish the interpreter
in loft on the bytecode runner and bring it to the Rust interpreter's speed or under it.
Rules unchanged: loft or C only, no `.rs`.  How the loop is built and what an ` #arm ` form
is: [1b-fill-port.md](1b-fill-port.md) § What a template is, § Speed.

## Where it stands

| Measure | Before this arc | Now |
|---|---|---|
| integer kernels, net user instructions against `loft --interpret` (fib / mix / collatz) | 2.01 / 1.32 / 1.17 | 0.49 / 0.93 / 0.86 |
| benchmark suite (`selfhost/bench/bench_speed.py`, 94 routines), geometric mean of time ratios | 6.05 (on the 32 first rows) | 2.10, worst 13.7 |
| operators that are arms of `run` | 90 of 366 | 222 |
| `run_image_survey.py` | 1186 of 1194 agree | the same, the same 8 |

What was done, in order: positions tested once a step and read through `crt::peek_*` /
`poke_*`; `OpCall` and a plain `OpReturn` as arms over a frame list in held memory; reference
arguments read field by field from their slots; the ` #arm ` template form; text arguments as
views in their slots (`crt::view_*`); arms for two-byte slots; the natives' handlers looked up
once (`State.handlers`); `find` / `rfind` / `starts_with` / `ends_with` / `byte_at` in place;
`OpTextNullJump`'s method no longer copies the text it tests.

## How to measure

- This machine runs at load 10: times and cycle counts move by a factor of two.  Compare
  builds by `perf stat -e instructions:u` (a large run less a tiny one), or ask the laptop
  session — `laptop-eager-nautilus` in `ListAgents`, it calls itself loft-c2 — which agreed
  to run `python3 selfhost/bench/bench_speed.py --repeat 3` on a quiet machine when sent a
  pushed sha.  It cannot message this session first.
- `LOFT_FILL_TRACE=1 <runner> <image>` prints every operator and native; pipe through
  `awk '{c[$5]++} END{for(k in c) print c[k], k}' | sort -rn` for a routine's histogram.
- Sampling a call chain does not work here (no frame pointers in the shipped build, gdb
  cannot attach): reproduce in a ten-line program and trace it.

## Open, by what the benchmark shows (ratios over 3)

1. **Keyed collections** (`15_stdlib_keyed` 7–11×, `08_word_count` 9×, `21_tree/tree_query`
   10×, `23_lru` 6×): `OpGetRecord`, `OpHashAdd`, `OpNewRecord`, `OpFinishRecord`, `OpFreeRef`,
   `OpDatabase` are `State` methods over `DbRef` values and `types::` lookups per call.
   Each needs one C routine taking the slot positions, and an arm form.
2. **`16_consumer_shapes/f32_build` 13.7×, `mesh_emit` 7×**: single-precision operators
   (`f32` slots are not an arm's width — `slot_width` has no `f32`) and record appends.
3. **Text building** (`13_stdlib_text/byte_walk` 11×, `join` 6×, `format_num` 4×,
   `18_consumer_crawler/quadratic_out` 6×, `format_specs` 5×, `19_stdlib_par/par_text` 11×):
   `OpFormatText`, `OpFormatFloat`, `OpAppendStackText`, `OpFormatStackInt`,
   `OpAppendCharacter`, `OpGetTextSub`, `OpConstStoreText` (a through arm) and the natives
   that still copy (`to_lowercase_dest`, `replace_dest`, `trim` is in place).
4. **`OpStaticCall`**: every native leaves `run`, returns to `fill_run::run_to` and re-enters.
   A native that needs nothing of `run_to`'s tables could be called from the arm.
5. **`20_interpreter` 4.5–5×, `17_consumer_moros_dryopea` 3–10×, `21_tree` 3–4×**: read the
   histogram first; `OpRefFromChildRec`, `OpSetDbRef`, `OpConstRef`, `OpCopyRecord` are the
   likely ones.
6. The general cure for 1–5 is the C writer admitting `state::DbRef` to `(R-ValueRecord)`;
   asked of loft-c2, queued behind its strand 12 work.

## Not done

- `make ci`, clippy, `cargo test --release --test selfhost`, `make wasm`, `make surface-gen`
  on this tip.  `default/01_code.loft` and `02_files.loft` changed (`#loft` lines only), which
  moves the wasm stamp.
- `scripts/windows_smoke.tsv` has no row for `selfhost/bench/bench_speed.py` on this branch;
  loft3-ac added it on `tuxedo-pr2495-next` (PR #2495, which carries `62e585c67`).  Take
  theirs at the join.
- `plans/91-self-host/README.md` still says "766 of 1,078 … not re-measured" after the join
  took the other side: correct it with 1186 of 1194 and the writer's numbers.
- Everything in [handoff-2026-11-14-bytecode-writer.md](handoff-2026-11-14-bytecode-writer.md)
  § Open work (the writer's 104 functions, the image tables, the front end's IR) is untouched
  by this arc.
