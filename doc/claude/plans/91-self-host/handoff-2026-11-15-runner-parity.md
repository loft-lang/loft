<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
<!-- size-exempt: a dated handoff record, read once (DOC_QUALITY § Maintainer docs 2) -->
# Handoff 2026-11-15 — the runner at the Rust interpreter's speed

Branch `tuxedo-pln204-int32`, pushed.  The owner's goal for the arc: the loft-in-loft
interpreter on the bytecode runner as fast as `loft --interpret` or faster, by GENERAL
optimisations, logic kept in loft (C only for marshalling primitives), the language as a whole
getting faster where the runner exposed a mechanism.  How the loop and an arm work:
[1b-fill-port.md](1b-fill-port.md) § What a template is, § Speed (which lists the cures).

## Where it stands

| Measure | 2026-11-14 night | Now |
|---|---|---|
| `bench_speed.py` geometric mean (94 routines), this machine | 2.03, worst 12.8 | 0.88, worst 1.80 (08688f618); 68 of 94 at or under 1.0 |
| the same, the laptop (quiet) | 2.20 | 0.99, worst 2.27 (62012dfbf, before the last three commits) |
| arms of `run` | 222 | 267 |
| `run_image_survey.py` | 1186 of 1194 | the same, the same 8 |

## What changed in the language, not only the runner

- `(R-ValueRecord)` in the IR: a record-consuming site no longer declines the function for every
  site (`value_records_twinned`; switch `LOFT_NO_VR_TWIN_SITES=1`), and a return buffer followed
  by hidden work buffers is dropped at its index.  Its lend of a record parameter is a fixpoint
  now (`lendable_params`): the first cut lent to a callee that wrote through the parameter
  (silent-wrong, `F-ParamHeap/2137`, fixed in 603a2a2a0 before any merge).  Measured -11 % C,
  -14 % interpreter on a small-record cell.  Census blessed; its channel is `make rewrite-census`.
- The C runtime finds a free store slot from a lower-bound hint (`loft_hint`), same numbers.

## Open, by size of what it would buy

1. **A part of a `const` parameter returned by value is deep-copied** (the `types::parts` /
   `record_fields` accessor shape): hand-priced at **90x** the inline read on the C backend
   (1.27 G vs 20 M instructions) and 6x on the interpreter, every loft program with accessor
   helpers.  `(F-Ret)` makes the result fresh; a call site that only reads the result while
   nothing writes the source cannot tell a view from the copy (the condition `(R-CopyView)`
   uses for locals).  A rewrite: such a callee returns the place as a view to such sites.
   Not started; needs the design pass (`design-protocol`) and a formal rule.
2. **A C routine cannot return a text but through a byte vector** (`room()`, a store per call):
   `crt::show_leaf` (`map_json` 2.25), `stack_str`, `get_text`.  A `#c_twin` signature returning
   the runtime's `loft_text` would remove the buffer; it touches `c_signature.rs`'s consumers.
3. The rows over 1.3 here: `map_json` 1.80, `sum` 1.68 (a 0.4 ms routine), `par_text` 1.51,
   `replace` 1.36, `record_append` 1.31, `par_map_float` 1.30 — read each one's histogram first
   (`LOFT_FILL_TRACE=1`, the one-routine images: keep `main`'s setup, drop the other timing
   blocks).
4. Not done on this tip: `make ci`, `make wasm` + `make surface-gen` (default/01_code.loft's
   `#loft` lines moved the wasm stamp), the language suite cell
   `a-small-record-handed-on-as-a-record-keeps-its-tuple-elsewhere.loft` (written, passes both
   backends and both switch positions; not in loft-suite yet — switching that checkout's
   branch was refused, it needs the owner's go).
5. Branch-carried reds from the merged compiled-bodies work, not this arc: the feature
   catalogue (four uncatalogued files), the `@FR-N-Shape` ratchet (1180 -> 1183), the layer and
   test-subject ratchets, `main`'s nightly rustc-native suite reds (two `B-Copy`, one `H-Free`).

## How to measure

Instructions, not time, on a loaded box: `perf stat -e instructions:u` on the shipped runner
and on `loft --interpret` (whole program).  For attributed profiles compile the emitted C with
frame pointers (`loft --native-c --ship --native-emit x.c`, then `cc -O2 -g
-fno-omit-frame-pointer x.c crt/native-auto/libcrt_shim_*.a`) and `perf record --call-graph fp`.
After a `default/*.loft` `#loft` edit: `make compiled-stdlib`, or `loft --interpret` runs every
compiled stdlib function as bytecode and the ratios flatter the runner.
