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
| `bench_speed.py` geometric mean (94 routines), this machine | 2.03, worst 12.8 | 0.87, worst 1.68 (`sum`, a 0.4 ms row), c94efcd89; `map_json` 1.84 -> ~1.27 after 115206415 |
| the same, the laptop (quiet) | 2.20 | 0.99, worst 2.27 (62012dfbf, before the last three commits) |
| arms of `run` | 222 | 267 |
| `run_image_survey.py` | 1186 of 1194 | 1193 of 1194 — the one left differs only in its `store_memory()` report (1b-fill-port.md difference 3) |

## What changed in the language, not only the runner

- `(R-ValueRecord)` in the IR: a record-consuming site no longer declines the function for every
  site (`value_records_twinned`; switch `LOFT_NO_VR_TWIN_SITES=1`), and a return buffer followed
  by hidden work buffers is dropped at its index.  Its lend of a record parameter is a fixpoint
  now (`lendable_params`): the first cut lent to a callee that wrote through the parameter
  (silent-wrong, `F-ParamHeap/2137`, fixed in 603a2a2a0 before any merge).  Measured -11 % C,
  -14 % interpreter on a small-record cell.  Census blessed; its channel is `make rewrite-census`.
- The C runtime finds a free store slot from a lower-bound hint (`loft_hint`), same numbers.
- `(R-ValueLocal)`'s tuple-parameter gate keys a native's write by record type and takes a
  recursion's least fixpoint (c94efcd89): the runner's tuple parameters 17 -> 273, geometric
  mean 0.92 -> 0.87, and every program with small-record helpers gets the same admissions.
  Guard: `tests/scripts/a-tuple-parameter-is-declined-only-where-its-record-can-be-written.loft`.
- The C writer hands a lent tuple carrier to a tuple parameter as it is (115206415,
  `LOFT_NO_LEND_HANDOFF`).
- The runner now carries reflection (`type_of`, `type_named`, `field_value`) and the process
  and environment natives (`eprint`, `exit`, `panic`, `env_variables`, the directory queries).

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
3. The rows over 1.25 here: `sum` 1.68 (both sides run the same vectorised block sum; `-O3` on
   the shim and a four-chain unroll both measured no gain or worse), `par_text` 1.55,
   `map_json` ~1.27, `replace` 1.37, `par_map_float` 1.32, `record_append` 1.29 — read each one's histogram first
   (`LOFT_FILL_TRACE=1`, the one-routine images: keep `main`'s setup, drop the other timing
   blocks).
4. **`par` runs on one thread in the runner** (1b-fill-port.md § the `par` row): the bench pins
   each routine to one core, so `par_text` reads 1.55 there, but on four cores the runner takes
   3.0 s against the Rust interpreter's 1.08 s (task-clock = elapsed: one CPU).  Workers on
   threads need runner state per thread over a store table the C runtime does not share
   safely today — a design item.
5. An empty vector local makes its store at the declaration, on both backends — the runner's
   `show::write` paid one per formatted value until it stopped declaring one it rarely fills
   (6c065851c).  A store made at the first append would remove the class.
6. Not done on this tip: a local `make ci` (GitHub CI is the gate here), and the language suite
   cell `a-small-record-handed-on-as-a-record-keeps-its-tuple-elsewhere.loft` (written, passes
   both backends and both switch positions; not in loft-suite yet — switching that checkout's
   branch was refused, it needs the owner's go).
7. Reds this branch carries and cannot close alone: three loft-suite cells red on `--native`
   only (two `B-Copy`, one `H-Free`) — `origin/main` 5330b3690 (#2482) fixed them, and the branch
   does not contain it yet, so they close when it is brought onto `origin/main` (a rebase of a
   pushed branch: the owner's call); decision C162 has no guard under `tests/` (its cells are in
   loft-suite); the `@FR-N-Shape` ratchet (1180 -> 1183).

## How to measure

Instructions, not time, on a loaded box: `perf stat -e instructions:u` on the shipped runner
and on `loft --interpret` (whole program).  For attributed profiles compile the emitted C with
frame pointers (`loft --native-c --ship --native-emit x.c`, then `cc -O2 -g
-fno-omit-frame-pointer x.c crt/native-auto/libcrt_shim_*.a`) and `perf record --call-graph fp`.
After a `default/*.loft` `#loft` edit: `make compiled-stdlib`, or `loft --interpret` runs every
compiled stdlib function as bytecode and the ratios flatter the runner.
