<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN200 — the interpreter and the code generators as defined parts of loft

**Issue:** [loft-lang/plans#200](https://github.com/loft-lang/plans/issues/200).  **Builds on:**
[@PLN191](https://github.com/loft-lang/plans/issues/191) (the crate DAG, whose layer order this
plan amends).  **Prepares:** the ANSI-C backend, strand 5 of
[@PLN91](https://github.com/loft-lang/plans/issues/91) (its first probe, P-cemit, is this plan's
acceptance test).  Code references are file:line at commit `95d10dbe1`; a step re-reads its sites
before it moves them.

## Contents
- [Goal](#goal) · [Where it stands](#where-it-stands-measured-2026-10-23) · [The structure](#the-structure)
- [Invariants](#invariants) · [The step protocol](#the-step-protocol)
- Phases: [B0 layers](#b0--the-layers) · [B1 opt](#b1--opt-what-every-backend-asks) ·
  [B2 the interpreter](#b2--the-interpreter-as-a-defined-part) ·
  [B3 the compiled-target base](#b3--the-compiled-target-base-plans-apart-from-the-rust-writer) ·
  [B4 crates](#b4--the-crates) · [B5 acceptance](#b5--acceptance-a-second-emitter-beside-the-rust-one) ·
  [B6 a minimal binary](#b6--optional-a-minimal-binary) ·
  [B7 testing: reuse compiled blocks](#b7--testing-reuse-compiled-blocks)
- [Order](#order) · [Dependencies](#dependencies) · [Risks](#risks) · [Open questions](#open-questions-for-the-owner)

## Goal

loft as a pipeline of DEFINED parts, each with a stated contract, so a second code generator can
be added beside the Rust one without reaching into it or into the interpreter:

```
front    source → IR (Data)                                         parser, scopes, front passes
opt      IR → better IR, and the analyses every backend asks         NEW
interp   IR → bytecode → execution; adopting compiled artifacts     the interpreter, a defined part
lower    IR → the decisions a COMPILED target makes (which records    NEW: today inside the Rust writer
         are values, which loops hoist, which walks stream), as data
rust     decisions → Rust text → rustc (native, wasm, android, html)  the Rust writer
c        decisions → C text → cc                                     next (@PLN91 strand 5)
runtime  what compiled code calls, described for every emitter       runtime_abi + its census
```

- **The interpreter is one part** with a contract: what it takes (optimised IR, the store
  schema), what it provides (bytecode, execution, the interpreter's natives, adopting an
  artifact), and nothing it reaches into outside `core`/`front`/`opt`.
- **Optimisations stay possible for the interpreter:** anything it can use lives in `opt`, the
  layer every backend shares.  [INTERPRETER_PERFORMANCE.md](../INTERPRETER_PERFORMANCE.md)'s
  direction — move a native rule into the IR — lands there.
- **A second emitter shares the decisions, not the writer:** `lower` holds what a compiled target
  decides, as plain data; `rust` and later `c` spell it.

A minimal interpreter-only binary falls out (B6) but is not the aim.

## Where it stands (measured 2026-10-23)

**Layers** (`scripts/crate_layers.py:52-96`, order = list order, gated by
`tests/extraction_hygiene.rs:734`): `core < front < gen < runtime < facade`, zero upward
references (@PLN191 P2).  `gen` holds `generation` and five IR passes (`ctor_literal`,
`leaf_inline`, `self_replace`, `value_record`, `zero_count`) because they ask the generator's
analyses; `runtime` holds the interpreter (`compile`, `state`, `fill`) ABOVE `gen`.  The
generator references no interpreter module; emitted TEXT does name `loft::state::`
(`generation/mod.rs:513-519`, `calls.rs:1350`).

**The generator is half decision, half Rust spelling.**  Of `src/generation/`'s ~69k lines,
~31.6k are in files that write no Rust text — `hoist.rs` (19.5k), `range.rs`, `non_sentinel.rs`,
`decode_view.rs`, `decode_structure_scan.rs`, `stream_walk.rs`, `apart_value.rs`,
`apart_records.rs`, `destination.rs`, `view_return.rs`, `hand_over.rs`, `keyed_entry.rs`,
`keyed_place.rs`, `file_hold.rs`, `pre_eval.rs`, `ranged_call.rs`, … — and ~37.7k write it
(`mod.rs` 14.4k with 441 `write!`s, `emit.rs`, `coroutine.rs`, `dispatch.rs`, `calls.rs`, the
`*_emit.rs` halves, `ops/`).  Several rewrites already split plan from spelling
(`decode_structure.rs`/`_scan.rs`/`_emit.rs`, `stream_walk.rs`/`_emit.rs`,
`apart_records.rs`/`_emit.rs`).

**But the decisions live on the writer.**  `Output` (`generation/mod.rs:786-1320`) has ~170
fields mixing writer state (`indent`, `declared`, `counter`), ~30 `*_disabled` rewrite switches,
per-function analysis (`vec_headers`, `rec_ptrs`, `ptr_windows`, …) and the whole-program PLANS:
`reachable` 851, `dup_fn_names` 856, `dead_by_fn` 903, `value_records` 1289, `apart` 1291, `dv`
1302, `ps` 1305, `sw` 1307 — computed in `output_functions` (`mod.rs:10878-10950`) and at
9161–9217.  Most decision code is methods on `Output`.  A second emitter cannot share them
without carrying the Rust writer.

**What the interpreter and the IR passes take from the generator** (all pure analyses over
`Data`/`Stores`/`Value`; state is thread-local memos and `OnceLock` switches):

| from | site | item | defined at | closure inside `generation/` |
|---|---|---|---|---|
| `state/codegen.rs` | :4989 | `ops::refvar_text_stack_variant` | `src/text_calls.rs:107` (re-export `ops/mod.rs:46`) | none |
| `state/codegen.rs` | :3348, :3589 | `container_element_base` | `mod.rs:2116` | none |
| `state/codegen.rs`, `state/mod.rs` | :5289, :528 | `hoist::char_walks`, `CharWalk` | `hoist.rs:4494`, `:4473` | 3 items, ~176 lines |
| `state/codegen.rs` | :1758 | `hoist::borrowed_text_walks` | `hoist.rs:4700` | 58 items, ~1,112 lines |
| `value_record.rs` | :226, :244 | `hoist::value_records`, `is_nullable_layout` | `hoist.rs:14247`, `:13420` | 177 items, ~5,621 lines |
| `leaf_inline.rs` | :55, :1157, :1214, :1221 | `range::*`, `non_sentinel::non_sentinel_vars` | `range.rs:56-498`, `non_sentinel.rs:223` | 35 items, ~1,023 lines |
| `ctor_literal.rs` | :226, :824 | `hoist::ret_buffer_attr`, `hoist::loop_records` | `hoist.rs:15655`, `:15415` | none; 14 items, ~552 lines |
| `extensions.rs`, `lib_placement/dispatch.rs`, `compiled_stdlib.rs` | :845; :287/357/388/415; :123–124 | `reachable_functions`, `duplicate_fn_names`, `disambiguated_fn_ident` | `mod.rs:402`, `:463`, `:539` | none |
| `introspect.rs` | :19, :510 | `Output::new` (the Rust section only) | | |
| `native_lib.rs` | 20 sites | building AND adopting an artifact, one module | | see B2 |

**The runtime compiled code calls** is described in `src/runtime_abi.rs`
(`CODEGEN_RUNTIME_FNS`, ~120 helpers, each `Abi::Cell` — `&UnsafeCell<Stores>` — or `Abi::None`):
a description rather than the runtime, the right kind of seam, but Rust-shaped, and emitted code
also reaches past it (store methods, `loft::state::`).

## The structure

Layers bottom up, as `crate_layers.py` will hold them, then crates:

```
core < front < opt < interp
                 opt < lower < rust          interp and {lower, rust} are SIBLINGS:
                                             neither names the other
facade   (repl, lsp, lib_placement, the binaries' shared code) above both
```

`crate_layers.py` orders layers in one list; siblings need one addition — a set of FORBIDDEN
layer pairs checked in both directions (`interp ↔ lower`, `interp ↔ rust`), each held at zero.

Physically: `src/opt/` (new; the shared analyses, one file per subject), the interpreter's
modules gathered under a contract (`src/interp/mod.rs` re-exporting its entry points), `src/lower/`
(new; the program plan and the plans' computations), `src/generation/` (the Rust writer).  Every
move leaves a re-export at the old path, so no caller and no emitted name changes in the step.

## Invariants

| # | invariant | held by |
|---|---|---|
| I1 | `opt` names nothing in `interp`, `lower`, `rust`; `interp` and `lower`/`rust` name nothing of each other | `crate_layers.py check`, zero budgets; later cargo's graph |
| I2 | an analysis two parts ask is MOVED, never copied — one home | the step protocol and review: a copy passes I3 until the two drift |
| I3 | every move is behaviour-neutral: emitted Rust and bytecode byte-identical | `scripts/introspect_diff.sh <before> <after>` → `IDENTICAL n/n` over the corpus and the suite; `make rewrite-census` unchanged |
| I4 | no interpreter routine gets slower | `make interp-gap` per phase |
| I5 | an optimisation the interpreter can use lives in `opt`; a decision only a compiled target makes lives in `lower`; only spelling lives in `rust` | a `@C` design decision (B1.13), cited by `crate_layers.py`, `src/opt/mod.rs`, `src/lower/mod.rs` and a guard |
| I6 | `lower` produces data a second emitter can read: no Rust text, no `Output` | B3's checks; B5 |

## The step protocol

Every step is ONE commit:

1. Build the BEFORE binary (`cargo build --release --bin loft`, copy it aside).
2. Move the items; leave `pub use crate::<new>::{…};` at the old path — @PLN191 P2's pattern:
   `src/fill.rs:1078-1079`, `src/codegen_runtime.rs:22-31`, `src/rpc.rs:25-28`.  Widen a
   visibility only as far as the new home needs.
3. `cargo build`, both clippy variants; `scripts/introspect_diff.sh <before> <after>` prints
   `IDENTICAL`; `make rewrite-census` unchanged; `crate_layers.py check`, then `lower` to the
   tree's new counts.
4. The step's own tests; `find_problems.sh --changed`.

A step that is not byte-identical is not a move: it is split, or it becomes its own change with
its own guard.

What a clean build does not catch, and B1 met each:

- **The census compares two binaries, not the baseline.**  Run
  `LOFT_BIN=<before> scripts/rewrite_census.py --verbose` and the same without `LOFT_BIN`, then
  diff the two outputs.  The committed baseline can lag the tree, and its rises are not the step's.
- **The item set is a closure.**  Take every top-level item the root reaches by name, but not
  through a field access (`.x`) or a field declaration (`x:`).  Otherwise a field named like a
  function pulls a whole subsystem along.  Cut a large closure leaves-first, by strongly
  connected component, so every commit is closed over the commits before it.
- **An item carries what sits above it:** a plain `//` comment between its doc and the item,
  and a multi-line `#[expect(…)]`.  Left behind, they attach to whatever follows.
- **An item ends where its syntax ends.**  A `const` or `static` ends at its first `;`: a
  string constant continued over two lines does not end at the next column-0 `}`, and
  reading it that way takes the items after it along.  `unsafe extern "C" fn` is an item too.
- **A `cfg`-gated item needs its gate wherever it is re-exported.**  Check the wasm targets
  and `--all-features`, where the gated paths are compiled.
- **Visibility widens inside the item too:** the private methods of a moved `impl` and the
  private fields of a moved struct.
- **`cargo doc --no-deps --lib`** names the intra-doc links that no longer resolve.  An `opt`
  target gets its path; a target still in a backend becomes a plain code span.
- **`python3 scripts/decisions.py check`** (after `make index`) names a design decision whose
  recorded site moved.

## B0 — the layers

**Done (2026-10-23):** B0.1 fixed loft#2142 (`generation::begin_program`, guard
`leaf_inline::a_programs_non_null_facts_do_not_reach_the_next_program`); B0.2 added `SIBLINGS`
and `selftest` to `crate_layers.py`; B0.3 set the layers — the five modules of @PLN191's
RUNTIME set that sat in `runtime` went to `core`, below both siblings — with the crossings
opt → rust 8, interp → rust 22, rust → interp 25 as the ratchet B1 and B2 cut.

| step | change | check |
|---|---|---|
| B0.1 | **A fix first.**  `non_sentinel`'s memo `RETURNS` (`non_sentinel.rs:123-133`) is cleared only by `Output::new` (`mod.rs:2403`), yet `leaf_inline::rewrite_program` asks `non_sentinel_vars` before it (`leaf_inline.rs:1214`) and clears only the range memo (`:1157`): a process compiling several programs on one thread (the test harness, the REPL, the LSP) can answer from an earlier program's definition numbers — the failure the comment at `:123` describes.  Write the two-program reproduction first; then one `begin_program()` clearing every analysis memo, called at `leaf_inline.rs:1157` and `mod.rs:2403-2404` | the reproduction, red before, green after, both backends |
| B0.2 | `crate_layers.py`: forbidden layer pairs (`FORBIDDEN`, checked both ways, beside `BUDGET` at `:101`) | a self-test of the script |
| B0.3 | the new layers: `opt` (the five IR passes), `interp` (`runtime`'s interpreter modules: `compile`, `compiled_stdlib`, `fill`, `state`, `native`, `native_gate`, `lib_ids`, `extensions`, `parallel*`, `debugger`, `engine_host`, `live_*`, `profiler`, `stack`, `c_call`, `codegen_runtime`, `wasm_gl`), `lower` (empty), `rust` (`generation`, `create`, `native_lib` until B2, `introspect` until B2); today's crossings registered in `BUDGET` as the ratchet | `check` passes; no code change |

## B1 — `opt`: what every backend asks

Smallest closure first.

**Done (2026-10-23):** `src/opt/` holds `attrs`, `char_walk`, `iteration`, `range`,
`non_sentinel`, `loop_records`, `writes`, `body_writes`, `text_borrow`, `names` and
`value_records/` (`layout`, `sites`, `leaves`, `mod`), about 10,000 lines in all; `hoist.rs`
fell from 19,500 lines to 11,500.  Every step was checked IDENTICAL 2830/2830 by
`scripts/introspect_diff.sh`, so the `make interp-gap` report at B1.6 and B1.11 could not
move and was not rerun.  opt → rust is 0, and interp → rust fell from 22 to 14: B1.1–B1.6
removed five crossings, and `compiled_stdlib` and `extensions` asking `opt::names`
removed three more.  Where the code differs from the
table:

- B1.2 moved `projection_container_var` to `use_analysis` (front), where its one
  question already lived.
- B1.8's writer closure was 46 items, not the table's list: `view_field` and `sub_record`
  belong to the body write set.
- B1.10 also took `rust_fn_ident` and the call collectors.  The identifier is the contract
  between the library writer and the symbol loader, so B2.1's key items can ask it from
  `interp`.
- B1.11 was cut leaves-first by the call graph, not by the line ranges.  Its first cut,
  `body_writes`, sits beside `writes` rather than under `value_records/`, because the scalar
  hoist asks it too.
- B1.13 is @C153.  It is cited by `crate_layers.py`, `src/opt/mod.rs`,
  CODEGEN_METHOD.md § Where an analysis lives, and the guard
  `extraction_hygiene::library_modules_keep_their_layers`.  No `src/lower/` exists yet; B3
  cites it there.

| step | change | check |
|---|---|---|
| B1.1 | `state/codegen.rs:4989` calls `crate::text_calls::refvar_text_stack_variant` directly (as `parser/objects.rs:3675` already does) | protocol |
| B1.2 | `container_element_base` (`mod.rs:2116`) → `src/opt/projection.rs` | protocol |
| B1.3 | `ret_buffer_attr` (`hoist.rs:15655`) → `src/opt/attrs.rs` (23 emitter mentions keep the re-export) | protocol |
| B1.4 | `char_walks`, `CharWalk`, `text_written` (`hoist.rs:4451-4494+`) → `src/opt/char_walk.rs` | protocol |
| B1.5 | the iteration helpers `VectorIteration` 4291, `iteration_head` 4307, `element_read` 4329, `iteration_step` 4369, `bounded_iteration_index` 6063 → `src/opt/iteration.rs` | protocol |
| B1.6 | `range.rs` and `non_sentinel.rs` whole → `src/opt/` (`collect_escapes` to `pub(crate)`) | protocol; `make interp-gap` |
| B1.7 | `loop_records`, `LoopRecords`, `LoopRecord` (`hoist.rs:15259-15415+`) and their helpers (`all_scalar_record` 8425, `is_scalar` 9023, `group_covers_type` 10843, `refillable_plain` 11077, `mint_type_of` 11231, `is_var` 11496, `vector_field_offsets` 15280, `vector_field_numbers` 15300, `loop_record_zeros` 15323, `SCALAR_GETTERS` 806, `IN_PLACE_SET_OPS` 157) → `src/opt/loop_records.rs` | protocol |
| B1.8 | the WRITER analysis — `may_write_store` 8385 → `writes_store` 8389 → `blocks_header_hoist` 8635 → `call_writes_store` 8786 and subtree (the mint/path family 7699–7839, `HoistOwned` 354, `HoistTiers` 401, `PathKey`/`vector_path` 300/306, `view_field` 859, `sub_record` 925, the op tables 49/91/157/8419/8846/8961, switches 290/7839/8597) → `src/opt/writes.rs`, two commits (tables and paths, then functions) | protocol |
| B1.9 | `borrowed_text_walks` (`hoist.rs:4700`), `TextBorrow` 4441, `SplitTable` 9306, `text_walk_head` 4408, `as_emitted_by_tables` 4652, `text_escapes(_with)` 5949/5957 → `src/opt/text_borrow.rs` | protocol |
| B1.10 | `reachable_functions` (`mod.rs:402`), `duplicate_fn_names` (:463), `disambiguated_fn_ident` (:539) → `src/opt/names.rs` | protocol |
| B1.11 | `value_records` (`hoist.rs:14247`), `ValueRecords` 13285 + impl 13505, `ValueTuple` 13347, `is_nullable_layout` 13420 and the 177-item closure → `src/opt/value_records/`, bottom-up in ~5 commits: (a) body writes (`body_writes` 2405, `element_target` 2221, `setter_target` 2129, `callee_writes` 2625, `rebound_vars` 692, `link_targets` 718); (b) leaves and view fields 15821–16979; (c) shapes and presence 17140–18493; (d) sites and reads 18565–19156; (e) candidates, fixpoint, fn-ref groups 13285–15742, with `super::fnref::…` (`hoist.rs:14371/14398/14682`) as `crate::fnref::…` | protocol each; `make interp-gap` at the last |
| B1.12 | `opt → rust` budget 0 | `check` |
| B1.13 | the placement rule (I5) as a `@C` design decision (`./scripts/idx next-decision`) | `make decisions` |

## B2 — the interpreter as a defined part

**Done (2026-10-23):** the interpreter and the Rust writer name each other nowhere (interp ↔
rust 0, from 22 and 25), and every layer budget is empty.  Code outside the interpreter reaches
it through `src/interp.rs` alone, which `crate_layers.py`'s ENTRY rule holds at 0 (83
references moved).  The contract is [INTERMEDIATE.md § The interpreter's
contract](../INTERMEDIATE.md#the-interpreters-contract).  Where the code differs from the table:

- **B2.0** (added): `native_gate` and `lib_ids` moved to the `opt` layer.  Both sides ask
  them, which functions compile and which number a key names, so `@C153` puts them below both
  siblings.
- **B2.1**: the key went to `opt::native_key`, not to `interp`.  The Rust writer builds under
  the key and the interpreter adopts by it, and the sibling rule forbids both directions.
- **B2.2**: `toolchain.rs` is in `core`, because only the driver and the Rust writer ask it.
  It also took `find_loft_rlib`, which the toolchain hint names.
- **B2.3** split in two.  The ABI types (`LibArg`, `HostTable`, `HOST_ABI`, `IdTable`,
  `LAYOUT_FP_SYMBOL`) went to `host_abi.rs` in the runtime set.  `native_lib` re-exports them,
  so emitted code is unchanged.  `HOST_TABLE`, the host callbacks, export marking and the
  layout probe went to `native_adopt.rs`, with no re-export.  The compiled standard library
  now names `loft::native_adopt::HOST_TABLE`.
- **B2.4**: `adopt_shared_cdylib` answers `Fresh`, `Interpret`, or `Build` with a key that
  holds the build lock, and `build_shared_artifact` builds.  Neither sibling may call the
  other, so their composition went to the driver layer (`native_artifact.rs`) rather than
  staying in `native_lib`.
- **B2.5**: the Rust section is `Options::rust`, a function the driver hands in
  (`generation::introspect_rust`).  `introspect` itself moved to `interp`.  The 19 benches'
  `--show-rust` output matches B1's binary.
- **B2.6** is a file, `src/interp.rs`, with one re-export module per part.  `generate` and
  `export_set` left `compiled_stdlib` for `stdlib_codegen.rs` (rust).  The regenerated
  `compiled_stdlib_gen.rs` differs only in two header lines.
- **B2.8**: `lib_placement`'s marking stays in the driver layer.  That layer may name every
  layer, so moving it changes no count.  It returns with B6 (a binary without the driver's
  worker code), if that is taken up.

| step | change | check |
|---|---|---|
| B2.1 | `native_lib` KEY items → `src/native_key.rs` (`interp`): `library_export_set` 554, `library_candidates` 572, `is_synthetic_name` 640, `hidden_dest_type_name` 1702, `platform_cdylib_name` 2119, `auto_cdylib_stem` 2528, `mix_fp` 2546, `artifact_sources` 2851, `contributing_fingerprint` 2908, `type_layout_fingerprint` 2930, `fn_numbering_fingerprint` 3124, `source_content_hash` 3147, `source_newer_than` 3219 | protocol; `n2_cdylib`, `n3_use_native` |
| B2.2 | TOOLCHAIN items → `src/toolchain.rs` (`interp`): `rustc_available` 2420, `rustc_version` 2427, `NO_RUSTC` 2434, `toolchain_failure_hint` 2448 (asked by `main.rs:3228/10061`, `test_runner.rs:2419`) | protocol |
| B2.3 | ADOPT items → `src/native_adopt.rs` (`interp`): `mark_native_exports` 409, `mark_exports` 424, `BridgeProbe` 439, `probe_and_mark_exports` 481, `init_one` 535/543, `mark_library_native` 630, `LibArg` 732, `HostTable` 772, `HOST_ABI` 805, `HOST_TABLE` 808, `HOST_COROUTINES` 821, `interpreter_for`/`host_coroutine_*` 835–903, `IdTable` 1215, `LAYOUT_FP_SYMBOL` 1330, `layout_fp_of_relocated` 2578/2592, `LayoutProbe` 2613, `artifact_matches_layout` 2725, `layout_fp_off_path` 2768, `relocate_for_probe` 2810, `report_unprobeable` 2830.  `LibArg`/`IdTable` are named by EMITTED Rust (`emit.rs:2100-2118`): the `native_lib::` re-exports keep those paths | protocol; emitted text unchanged |
| B2.4 | `cached_or_build_shared_cdylib` (`native_lib.rs:2971-3117`) splits into `adopt_shared_cdylib(…) -> Adopt { Fresh(so), Stale(key), Missing(key) }` (the key 2978–3037 and fresh check 3053–3077, `interp`) and `build_shared_artifact` (3079–3116, `rust`); the old function is their composition, so `main.rs:9963/9025` are unchanged | `registry_e2e::a_released_library_artifact_runs_compiled_without_rustc`; `n3_parity` |
| B2.5 | `introspect::emit_rust` (`introspect.rs:504`) → `generation/introspect_rust.rs`; `introspect` keeps IR, bytecode, slots, ownership, and takes the Rust section as a function `main.rs:12632` passes in | `tests/introspect*.rs` |
| B2.6 | `src/interp/mod.rs`: the interpreter's entry points in one place — `compile::byte_code`, the `State` execution entries, the natives registry, `compiled_stdlib::{mark, names, bridge}` (run-time half; `generate`/`export_set` move to `rust`), the adopt API — re-exported; callers outside `interp` use it | protocol |
| B2.7 | the contract written down: a section of [INTERMEDIATE.md](../INTERMEDIATE.md) (or its own page if over its budget) — what `interp` takes and provides, what it may name | `doc_lint` |
| B2.8 | `interp ↔ rust` forbidden pair at 0; `lib_placement`'s run-time marking (`dispatch.rs:286`, `:377`) moves from `facade` to `interp` | `check` |

## B3 — the compiled-target base: plans apart from the Rust writer

**State (2026-10-24):** B3.1, B3.2, B3.3, B3.5 and B3.6 are done.  B3.4 waits for the first
emitter that wants a family (B5).  Every step was checked against main's binary: `--show-rust`
is identical on all 511 programs of `bench/` and `tests/scripts/`, and the rewrite census is
identical (103641 admissions).  Where the code differs from the table:

- **B3.1** holds seven plans, not eight: the value records, their dead buffers, and the
  apart, record-apart, decode-view, structure and stream plans
  (`lower::plan::ProgramPlan`, `plan_program`).  `reachable` and `dup_fn_names` stay the
  emitter's input, because which roots it starts from is its own choice.  `dead_buffers` moved
  to `opt::value_records` first.
- **B3.2** moved 64 switches to `lower::switches::Switches`.  `hoist_disabled` stays on
  `Output`, because the emitter also toggles it per function.  Eight switches set one at a
  time change the Rust exactly as they do on main.
- **B3.3** moved `stream_walk`, `decode_view`, `apart_value` with `apart_records`,
  `decode_structure` with its scan, `view_return`, `hand_over`, `keyed_entry`, `keyed_place`,
  `file_hold` and `destination` whole, each re-exported at its old path.  `destination`'s
  two helpers went to `opt` first.
- **`ranged_call` stays in the Rust writer.**  Its decisions read the emitter's state for the
  loop being written (the hoisted scalars, the variants already asked for), so it is a B3.4
  family and not a whole-program plan.
- **B3.5** is `scripts/runtime_surface.py` (`make runtime-surface`), committed as
  [RUNTIME_SURFACE.md](../RUNTIME_SURFACE.md).  It finds four modules outside the runtime
  set that emitted code names, and these are B7.3's work list:
  - `live_dispatch` (the live tier, in every default `--native` program);
  - `state::STRING_NULL`;
  - `native_lib::HostTable` and `native_lib::LibArg`, both `host_abi` items named through
    `native_lib`'s re-export;
  - `native_adopt::HOST_TABLE`, in the compiled standard library.

| step | change | check |
|---|---|---|
| B3.1 | `src/lower/mod.rs`: `pub struct ProgramPlan { reachable, dup_fn_names, dead_by_fn, value_records, apart, dv, ps, sw }` and `pub fn plan_program(data, stores, opts) -> ProgramPlan` — the computation of `output_functions` (`mod.rs:10878-10950`) and of 9161–9217, moved; `Output` holds `plan: Rc<ProgramPlan>` and its eight fields become reads of it | protocol |
| B3.2 | the ~30 `*_disabled` switches on `Output` → `lower::Switches`, read once per program; a second emitter honours the same switches | protocol; `NATIVE_SWITCHES.md` unchanged |
| B3.3 | the plan computations whose files write no Rust (`decode_view.rs`, `decode_structure_scan.rs`, `stream_walk.rs`, `apart_value.rs`, `apart_records.rs`, `destination.rs`, `view_return.rs`, `hand_over.rs`, `keyed_entry.rs`, `keyed_place.rs`, `file_hold.rs`, `ranged_call.rs`) → `src/lower/`, one per commit; where one is a method on `Output`, it becomes a free function over `(data, stores, &ProgramPlan, def_nr)` first, in the same commit | protocol each |
| B3.4 | the PER-FUNCTION decisions computed during emission (`vec_headers`, `vec_bases`, `rec_ptrs`, `ptr_windows`, the hoist tiers, …) move into a `FnPlan` built before a function is written — ONE DECISION FAMILY AT A TIME, in order of measured weight, each when the C backend wants it (decision below); a decision that depends on what was already written stays in `rust` and is listed | protocol per family; the family's bench rows on both emitters |
| B3.4a | the first family: the vector header and element base held across a loop (`vec_headers`, `vec_bases`, the element pointer per iteration) — most of the measured 3.6× below | protocol; the example's loop on both emitters |
| B3.5 | the runtime surface: a census of every runtime item emitted Rust names (beyond `CODEGEN_RUNTIME_FNS`: store methods, `loft::state::`, `loft::native_lib::`), generated from the corpus's emission; the input to the C backend's runtime decision (@PLN91) | the census, committed as a report |
| B3.6 | `lower ↔ interp` forbidden at 0; `lower → rust` budget 0 | `check` |

**Decision (owner, 2026-10-23): the C backend starts PLAIN.**  It reads the whole-program plans
(B3.1) and emits the general form; the per-function decisions reach it family by family
(B3.4, B3.4a first).  Measured on `total(ps: vector<P>) { for p in ps { s += p.x * p.y } }`,
200 passes over a million records, `--native-release`:

| emission | time |
|---|---|
| with the per-function decisions (Rust today: header and element base held, fields read at the element's address) | 186–191 ms |
| without them (every element and field through the general store lookup) | 682–698 ms — 3.6× |
| the interpreter | 8,169 ms |

So a plain C backend is correct from its first function, ~12× the interpreter on this loop, and
checkable against the interpreter over the whole corpus before any decision is shared; each
family that moves is then one measured step on both emitters.

## B4 — the crates

@PLN191 now targets ONE crate, `loft-runtime` (its R3; R2 is complete — the runtime set names
nothing outside it), and marks its compiler crates (P3–P7) superseded.  So `core` and `front`
become crates HERE, before B4.1, in @PLN191's manner (one crate per step, re-exports keep
every path, emitted Rust names unchanged).  The root `Cargo.toml` has no `[workspace]` yet;
`fuzz/Cargo.toml:14-17` and `tools/zt-c-web-staging/web/wasm/Cargo.toml:21` depend on `loft` and
keep doing so through the facade.

| step | change | check |
|---|---|---|
| B4.1 | `crates/loft-opt` | full gate; I3 |
| B4.2 | `crates/loft-interp` | full gate; I3 |
| B4.3 | `crates/loft-lower` | full gate; I3 |
| B4.4 | `crates/loft-rust` (the Rust writer and artifact building) | full gate; I3 |

## B5 — acceptance: a second emitter beside the Rust one

The structure is right when @PLN91's P-cemit probe — one loft function emitted as ANSI C and
built with `cc` — can be written as a NEW part (`src/c/` or `crates/loft-c`) that reads
`ProgramPlan`/`FnPlan` and the runtime description, with no edit to `rust`, `interp` or `opt`.
Every edit it would need is a defect of this plan, fixed here before strand 5 starts.

**Progress (2026-10-24).**  The probe grew into a backend: `src/c/` (layer `c`, a sibling of
`interp` and `rust`) writes whole programs — texts, stores, vectors, records, enums, floats,
characters, narrow integers, tuples, `&` parameters, function values and closures, deep copies
and store moves, the `{v}` renderer, content equality, constants — and its tests.  It reads
`use_analysis`, `scopes`, `opt` and the store layouts and edits nothing in `rust`, `interp` or
`opt` beyond two moves into shared homes: `test_runner`'s annotation parser (so `--emit-c` and
`--tests` read `@EXPECT_FAIL` alike) and `opt::header` (the header-hoist decision).  Measured
with `scripts/c_sweep.py` over the 2816 corpus programs, in the three macro modes: 1278 match
the interpreter, 0 differ, 0 fail to compile, 0 trip AddressSanitizer/UBSan; 1538 are refused
by name (355 of them do not compile in loft at all — expected-error programs and ones needing
a library).  The largest families still refused: the keyed collections (hash, sorted, index;
~350 programs across reasons), file I/O, generators, and `&` locals.  How it works and how to
extend it: [C_BACKEND.md](../C_BACKEND.md).

**⚠ NEXT — the quality of the C the generator writes: read this before the next feature (owner, 2026-10-24).**
The C backend validates the generator and gets no benchmark lanes or twins (@PLN201, owner),
so nothing here is a speed bar.  These are defects in the emitted C, found by taking one
program apart by hand: `bench/10_sort`'s `insertion_sort` at `7d370d10`, built `cc -std=c99 -O2`,
ran 8.8 s where `--native` ran 0.65 s.  Forcing the access helpers inline gave 1.3 s; holding
the vector header and element base in locals before the loop gave 0.34 s.  Removing the
arithmetic checks alone changed nothing.

1. **The hot accessors are out-of-line calls.**  gcc keeps `loft_vec_get` as a function, and its
   12-byte `loft_ref` comes back through the stack as two narrow stores and one wide load, a
   store-forwarding stall per element access.  `loft_set_int` is a call too.  Fix in
   `loft_rt.h`: the accessors `LOFT_HOT_INLINE` (below), and the element access answers the
   element's address, not a struct.
2. **Nothing is hoisted, and the C compiler cannot do it.**  An element write is a `memcpy`
   into the store's `uint8_t` buffer, which may alias the store table and the buffer pointer,
   so gcc reloads table → buffer → header → length after every write.  Native holds them in
   locals under `(R-Header)` (`vec_header` / `vec_base`).  That decision is B3.4's first
   family: move it into `FnPlan` and emit the base and length as locals before the loop.
3. **`loft_mul` tests overflow by DIVIDING** (`a > INT64_MAX / b`): a hardware divide per `*`.
   Replace it with the overflow macros below.

**The macro layer (`src/c/loft_cc.h`, included by `loft_rt.h`).**  The ONLY place the emitted
C names a compiler; the generator writes these macros and never a raw `__builtin` or
attribute.  Three compile-time modes, and a mode changes speed, never a value:

| mode | meaning |
|---|---|
| `-DLOFT_STRICT` | pure C99, clean under `cc -std=c99 -pedantic -Werror`: every macro its exact C99 fallback |
| default | gcc / clang / MSVC extensions where detected (`__has_builtin`, `__GNUC__`, `_MSC_VER`), the fallback otherwise |
| `-DLOFT_VERIFY` | default, and every PROOF the generator relied on is re-checked: a false one aborts naming the site (the C twin of `LOFT_HOIST_VERIFY=1`) |

Three kinds of site:

- **Safe hints** — ignoring one changes speed only: `LOFT_HOT_INLINE`, `LOFT_COLD`,
  `LOFT_LIKELY` / `LOFT_UNLIKELY`, and `LOFT_ADD_OVF` / `LOFT_SUB_OVF` / `LOFT_MUL_OVF`
  (`__builtin_*_overflow`; the C99 fallback multiplies directly when both operands are below
  2^31.5 and divides only above).  The CHECKED operators `loft_add` / `loft_sub` / `loft_mul`
  sit on them with unchanged semantics: a null operand or an overflow answers null.
- **Proof sites** — emitted only where a rewrite PROVED the fact (`(R-RangeArith)`,
  `(R-GuardedChain)` for a plain operator, `(R-Header)` for a held base):
  `LOFT_PLAIN_ADD/SUB/MUL(a, b, site)` and `LOFT_PROVED(cond, site)`.  A plain operator
  computes in `uint64_t`, so a wrong proof is a wrong value and never C's undefined behaviour.
  `restrict` and `__builtin_unreachable` / `__builtin_assume` belong here and nowhere else.
- **Ship sites (@PLN201)** — the ship tier's dropped check, chosen PER OPERATOR by the generator
  from the ledger exactly as the Rust generator chooses it: `LOFT_SHIP_ADD/SUB/MUL/NEG/SHL(a, b)`,
  the operator wrapping through `uint64_t`.  NEVER a plain signed `a + b`: Rust's `wrapping_add`
  is defined, a C signed overflow is not, and a licensed site that overflows after all must
  answer a wrong number, never let the compiler delete code around it.  `-DLOFT_SHIP_PROBE`
  maps every checked operator to its ship form, the C twin of `LOFT_RELEASE_PASS_PROBE=1`.
  The checked tier never emits a ship site; the ship build composes with `LOFT_STRICT`.

**What the macros become in LLVM** (clang `-O2`, read from `-emit-llvm` and the assembly;
gcc's builtins are the same operations):

| C the generator writes | LLVM IR | meaning |
|---|---|---|
| a plain signed `a + b` | `add nsw` | a PROMISE of no overflow: an overflow is poison and the optimiser may assume none.  Never emitted |
| `(int64_t) ((uint64_t) a + (uint64_t) b)` — `LOFT_PLAIN_*`, `LOFT_SHIP_*` | `add` | wraps, defined; the same IR as Rust's `wrapping_add` |
| `__builtin_*_overflow` — `LOFT_*_OVF` | `llvm.{s,u}{add,sub,mul}.with.overflow` | `{result, overflow bit}`; the same IR as Rust's `checked_*` / `overflowing_*`, so the checked C and the checked native are one operation |
| a clamp | `llvm.sadd.sat` and siblings | saturating; clang recognises a hand-written clamp.  Not loft's semantics — a clamped value is a wrong answer that looks right |

The checked multiply (`loft_mul` on the overflow macro) compiles to two compares against the
sentinel, `imul`, and `cmovno`: the overflow test is one conditional move with no branch, and
the two null compares cost more than it does.  LLVM has no sentinel: those compares are always
the generator's, and only a non-null proof removes them.  A `with.overflow` intrinsic gives one
bit per operation and the loop vectoriser does not vectorise across it, which is why
`157-native-4x`'s fast copies find an overflow by SIGN BITS (`@FR-R-FaultFlag`); what the ship
tier wins is the vectorised loop, not a cheaper scalar operator.  `-fwrapv` (signed overflow
defined, compiler-wide, not standard) is NOT the substitute for the `uint64_t` casts;
`-fsanitize=signed-integer-overflow` is a useful sweep cross-check on the emitted C, proving no
signed `+` slipped past the macros.

`scripts/c_sweep.py` runs the corpus under `LOFT_STRICT`, the default and `LOFT_VERIFY`; all
three must match the interpreter, and a difference between modes is a bug in a macro's
fallback.  A tested sketch of the first two kinds: 1587 operand pairs (the boundaries at 0, ±1,
±2^31.5, ±2^32, `INT64_MAX`, `INT64_MIN`) equal a 128-bit reference in all three modes under
gcc and clang, the strict header passes `-pedantic -Werror`, a false proof aborts under
`LOFT_VERIFY`.

Order: (1) the macro layer with the overflow macros, replacing the divide; (2) the inline
accessors; (3) `(R-Header)` through `FnPlan` to the C generator (B3.4); (4) the ship sites,
once @PLN201's ledger exists.  Each step checks the emitted C of `insertion_sort` (no call
left in its inner loop after (2), no reload of the store table after (3)) and the sweep in all
three modes.

**Where the order stands (2026-10-24).**  (1) done: `src/c/loft_cc.h` holds the three modes
and the safe-hint, proof and ship macros; `loft_rt.h` names no compiler.  The checked multiply
sits on `LOFT_MUL_OVF` (two computed operands; a constant operand keeps the portable check,
which folds to one compare).  `loft_add` / `loft_sub` keep the portable comparison rather than
`LOFT_ADD_OVF` / `LOFT_SUB_OVF`: the builtin hides the operands' ranges from the optimiser,
measured on clang as a masked accumulator's loop (`bench/02_sum_loop`'s `mix`) running 2.7×
slower.  `tests/c_emit.rs` checks the three overflow macros against Rust's checked arithmetic at
the boundaries in all three modes.  (2) done: the accessors are `LOFT_HOT_INLINE`; the element
access still answers a `loft_ref`, which the forced inlining scalarises.  (3) done without
`FnPlan`: `hoistable_vectors` and its gate moved to `opt::header` (the Rust writer re-exports
them; `introspect_diff` identical bar an absolute address in registry-library code), and the C
generator holds each admitted vector in a `loft_vh` before the loop with a `LOFT_PROVED` site at
every held access — injecting a push into a held loop aborts under `LOFT_VERIFY` naming the site.
`insertion_sort`'s inner loop has no call and no store-table reload left.  `scripts/c_sweep.py`
runs the three modes; the strict build is `-pedantic -Werror` clean on every carried program.
(4) waits for @PLN201's ledger.

## B6 — optional: a minimal binary

**Later, not now (owner, 2026-10-23).**  A by-product, built when wanted: a `native-codegen` feature (default on) gates `rust` and the
`--native` / `--native-release` / `--native-emit` / `--native-wasm` / `--html` /
`--native-android` / `build-native` arms (`main.rs:7835/7838/7915/7924/7945/~7934/8922`), which
answer one line without it; `make minimal` via `scripts/lean_build.sh --minimal`.  Its artifact
identity already equals the full build's (`cache.rs:231`: `feature_signature` (:119) lists
`threading`, `wasm`, `mmap`, `png`, `native-extensions`, `random`; `strict-stores` changes no
layout, `keys.rs:2836`) — B6 adds a test that every `[features]` name (`Cargo.toml:55-115`) is
classified as identity-relevant or not, and a CI comparison of both builds' corpus output.
Measured: the shipped binary is 25.2 MB (21.1 stripped), the generator ~12 % of its machine code.

## B7 — testing: reuse compiled blocks

The split's second payoff: a compiled block — a test program's binary, a library artifact, the
standard library's compiled code — can be keyed on what it actually depends on (its generated
source and the runtime it links) instead of on the whole loft build, so an edit that changes
neither reuses it.

**The principle (owner, 2026-10-23): a compiled unit is reused unless its loft code changed or
an optimisation pass changed what loft makes of it.**  The unit is the PACKAGE — the standard
library, each library, the program — as a crate is for `rustc`.  Its key is what it was made
from and what it must agree with, never "which loft binary":

| unit | key | rebuilt when |
|---|---|---|
| a package compiled for a native target | hash of its GENERATED code ⊕ each dependency's API fingerprint ⊕ the runtime INTERFACE version ⊕ the toolchain (`rustc` version, target, flags) | its source changed, or a pass changed its output, or a dependency's API changed, or the interface or the toolchain moved |
| a package's IR for the interpreter (B7.11) | its source ⊕ the `front` and `opt` fingerprints ⊕ the IR schema version | its source changed, or a front-end or optimisation pass changed |

So a compiled object is invalidated only by a change to its own code — an API change or a bug
fix in it — or to an API it uses, and the event that makes that change is the one that builds
the new object: a local edit rebuilds the edited unit, a library's publish builds that library's
object (the registry's CI, G2c's open half), a loft release whose passes change a unit's output
rebuilds it (G2c).  A dependency's bug fix, its API unchanged, rebuilds only the dependency.

**Inlining across a unit boundary is opt-in (owner, 2026-10-23).**  By default the line holds: a
call into another unit is a call, on every tier.  A function its author MARKS for inlining is
copied into its callers' units, like Rust's `#[inline]`; its body then belongs to its unit's API
fingerprint, so changing it rebuilds the units that inlined it.  Inside a unit nothing changes:
today's automatic inlining — `#[inline]` on every loop-free function (`Output::fn_inline_attr`,
[NATIVE_SWITCHES.md § Inlining hints](../NATIVE_SWITCHES.md#inlining-hints)) and the IR's
`leaf_inline` — stays, and stops at the boundary unless the callee is marked.

Keying on the generated code is what makes the rule exact without versioning passes by hand:
generating is milliseconds, compiling is the cost, and a pass that does not change a unit's
output leaves its hash — and its compiled form — alone.

**What the principle requires of the runtime.**  Today a library artifact statically embeds a
copy of `libloft` and shares the host's stores by memory (`main.rs`, `build-native`: "it
statically embeds libloft").  A fix inside a runtime function then never reaches an old
artifact, so ANY runtime change must rebuild every unit — the opposite of the principle.  A
reusable unit reaches the runtime through a VERSIONED INTERFACE instead: the store layout it
reads directly (field offsets, `DbRef`) and a table of the runtime functions it calls (the
`HostTable` of @PLN199 already carries some: `call_ref`, type and function ids, coroutines).  A
runtime function's body can then change without rebuilding a unit; only a change to the
layout or to a signature moves the version.  It is also the interface an ANSI-C unit needs.

**Measured (2026-10-23, macOS arm64):**

| fact | value |
|---|---|
| a test-tier `--native` compile (cache off) | ~0.7 s: ~0.4 s fixed (rustc start, the runtime's metadata, the link) + ~0.09 s per 1,000 generated lines |
| the standard library in a program's generated Rust (five corpus programs) | the same **3,303 lines** in each, ~70 %; the program's own code 300–1,350 lines; ~600 lines of preamble |
| programs a native corpus run can compile | 492 `tests/scripts` + 2,638 suite programs; plus `native`, `n2_cdylib`, `n3_*`, the drop gate's native cells |
| test binaries, each linking all of `libloft` | 451 |
| a library artifact at the shipped tier | ~7 s each (42 libraries in 4 min 54 s) |
| the last 400 commits touching `src/` | 62 % touch a module of @PLN191's runtime set (61 modules), 51 % `generation/`, 53 % the interpreter |

**Why every rebuild of loft throws compiled blocks away today:**

- the native program cache (`native_source_key_fingerprint`, `src/main.rs:4304`) folds in every
  runtime rlib a build could link (`native_utils::runtime_rlib_candidates`,
  `src/native_utils.rs:110`) — the rlib is all of `libloft`, so an edit to the parser, the
  interpreter or the generator invalidates every cached program, its generated Rust unchanged;
- a library artifact on a development build is keyed on `loft_build_fingerprint` (the rlib's
  content) and the executable's file time (`cache.rs:223`, `native_lib.rs:3008`), so every loft
  rebuild rebuilds every library artifact a test uses;
- every test program carries the standard library's 3.3k lines through `rustc` again.

| step | change | check |
|---|---|---|
| B7.1 | the baseline: per full gate, the number of `rustc` launches, their CPU time, and the native cache's hit rate after (a) a parser-only edit, (b) a generator-only edit, (c) a runtime edit — a report script, run before and after each of B7.3–B7.5 | the report |
| B7.2 | `cache::runtime_fingerprint()`: the content of the runtime a compiled block links — the `loft-runtime` crate's rlib once @PLN191 R makes it a crate; before that, a source stamp over the RUNTIME set's files (`scripts/crate_layers.py`) with the features, rustflags and `rustc` that shape it | a test: an edit outside the set leaves it, one inside moves it |
| B7.3 | the native program cache keyed on the generated Rust ⊕ `runtime_fingerprint()` instead of every rlib candidate.  **Precondition:** emitted Rust names nothing outside the runtime — today it names `loft::state::` (`generation/mod.rs:513-519`, `calls.rs:1350`) and `loft::native_lib::` (`emit.rs:2100-2118`); B3.5's census lists every such name and each is moved into the runtime or the key covers it | B7.1's hit rate after a parser-only edit; the corpus on both backends |
| B7.4 | a library artifact on a development build keyed on its generated source's hash ⊕ `runtime_fingerprint()` (a release bundle keeps its identity, B6).  @PLN199 G1 already made the generated source a function of the library alone, so the key is exact; loft#1776's stale-rlib case stays covered because the fingerprint is the runtime's content | `n3_use_native`, `registry_e2e`; B7.1 after a parser-only edit: no artifact rebuilt |
| B7.5 | the standard library compiled ONCE for the test tier: its functions emitted as one block (ids by key, as @PLN199 G0b made library code), keyed like B7.4, and a test-tier program calls into it instead of carrying 3.3k lines — ~40 % of each test compile by the measurement above.  `--native-release` and the library cdylibs keep whole-program compilation: there the optimiser's view across functions is what ships | the corpus's output identical; B7.1's CPU time |
| B7.6 | narrow the runtime set (@PLN191 R2): every module that leaves it is an edit that no longer invalidates compiled blocks — the report names the runtime modules the most commits touch | B7.1 over the history |
| B7.7 | test selection from the crate graph (after B4): `find_problems.sh --changed` maps a diff's crates to the subjects that can observe them | a diff confined to `rust` selects no interpreter-only subject |

| B7.7a | a unit carries ONLY its own code: today a library artifact compiles in every library it reaches (`native_lib.rs`, `cached_or_build_shared_cdylib`) and its key folds each dependency's name and version (`contributing_fingerprint`, `native_lib.rs:2908`), so a dependency's bug fix rebuilds every user.  A unit calls its dependencies' units (B7.12's linking) and folds each one's API FINGERPRINT — its exported signatures and the layouts of the types it exports, by key (`lib_ids`) — not its version or its code | a test: a dependency's body changes, its users are reused; its exported signature changes, they are rebuilt |
| B7.7b | the event builds the object: a library's publish builds its unit for every released loft whose interface it meets (the registry's CI; @PLN199 G2c builds them only at a loft release); a loft release builds only the units whose generated code its passes changed, and reuses the rest from the previous release by their keys | the release job's report: units rebuilt vs reused |
| B7.8 | the runtime INTERFACE defined and versioned: the store layout (`Store`, `DbRef`, the record header — offsets and sizes computed, not written by hand) and the signature of every runtime function a unit calls (`runtime_abi`'s table, the `HostTable`); `runtime_interface_version()` hashes both | a test: changing a layout or a signature moves the version; changing a function's BODY does not |
| B7.9 | units stop embedding the runtime: a unit's calls into the runtime go through the versioned table; the hot accessors that need only the layout (a field read or write at a `DbRef`) stay inline.  A performance trade — measured on the bench lanes (`make perf-check`) before it is taken, per accessor family | the bench rows; the corpus on both backends |
| B7.10 | the reuse guard, both directions: build a unit; rebuild loft with an unrelated change (a parser comment, an interpreter fix) and assert the unit is REUSED; change an optimisation pass so the unit's output changes and assert it is REBUILT; change a runtime function's body and assert it is reused and runs the new body | the test, falsified by keying on the loft build again |
| B7.11 | the interpreter's unit: a package's IR (parsed, scoped, optimised) cached per package and keyed as the table above says, after B4 gives `front` and `opt` fingerprints of their own; the IR codec and the start-up caches exist, keyed today on the whole program and the standard library | a program using three libraries starts with three cached images after one library's edit rebuilds only that one |
| B7.12 | native builds link packages as units: a library compiled once as an rlib (or, after B7.9, an object behind the interface) and linked by every program that uses it, on every tier; a call across units is a call | B7.1's CPU time; the bench lanes, read with B7.14's report |
| B7.13 | the inline mark: the loft annotation `#inline` on a `pub fn` (the spelling settled by the owner, 2026-10-23) that copies it into callers' units; `leaf_inline` and `fn_inline_attr` cross a unit boundary only for a marked callee, and a marked function's body hash joins its unit's API fingerprint | a test: a marked function inlined across units, its body changed, its callers rebuilt; an unmarked one called, its body changed, its callers reused |
| B7.14 | where marks matter: a report of the inlines that cross a package boundary today (`leaf_inline`'s admissions and the `#[inline]` hints on calls into another package, per bench routine) and each one's effect when the boundary holds — the list a library author marks from | the report; the bench rows that moved, each marked or accepted |

Not addressed by this plan: the 451 test binaries each linking all of `libloft` — a link cost
the split does not remove (the corpus leaving the crate is @PLN193).

## Order

B0 → B1 → B2 → B3 → B5 (the probe, from @PLN91) → B4 when @PLN191 is ready; B6 later, not scheduled.
B7.1 can run any time (the baseline); B7.5 needs only @PLN199's keyed ids and can come early;
B7.8–B7.10 (the versioned runtime interface and the reuse guard) follow B3.5's census and come
before B7.2–B7.4 rely on them; B7.11–B7.12 follow B4; B7.14's report comes before B7.12 holds the
line, so the marks B7.13 needs are known when it does.
B1 and B2 make the interpreter a defined part on their own; B3 is what the C backend needs.

## Dependencies

- **@PLN191:** B0 lands before its P3 (the first crate), so P3 cuts along `opt < interp`,
  `opt < lower < rust`; B4 waits for its `core` and `front` crates.
- **@PLN91:** B5's probe (P-cemit) and the C runtime decision are its.
- **The checks' inputs:** `../loft-suite` at `tests/loft-suite.pin` (for `introspect_diff.sh`),
  the bench library checkouts (the rewrite census, keyed by their resolved dependencies), the
  bench lanes (`make interp-gap`).
- **Not a dependency — owner ruling, 2026-10-23:** branches that change `src/generation/`,
  `native_lib.rs` or `src/state/` concurrently (the native-4x arc and others) adapt their code
  to the moves when they merge; the moves are not scheduled around them.

## Risks

- **B1.11 is the long one** (5.6k lines, ~5 commits), cut along the closure's own grouping.
- **B3.3–B3.4 turn methods into functions:** the step protocol's byte-identity is what keeps that
  honest; a decision that cannot leave the writer is listed, not forced.
- **Memos and switches move with their module**, never duplicated (B0.1's class of defect).
- **The runtime is Rust-shaped** (`Abi::Cell`); how a C program reaches the stores — a C runtime,
  or the Rust one through a C ABI (`loft-ffi` is one) — is @PLN91's decision; B3.5 gives it the
  facts.

## Open questions for the owner

None open.  Settled 2026-10-23: concurrent branches adapt on merge; inlining across units is
opt-in, marked `#inline`; the C backend starts plain (B3.4); B6 is LATER — not scheduled with
this plan's phases, taken up when wanted.
