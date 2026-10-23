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
  [B6 a minimal binary](#b6--optional-a-minimal-binary)
- [Order](#order) · [Risks](#risks) · [Open questions](#open-questions-for-the-owner)

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

## B0 — the layers

| step | change | check |
|---|---|---|
| B0.1 | **A fix first.**  `non_sentinel`'s memo `RETURNS` (`non_sentinel.rs:123-133`) is cleared only by `Output::new` (`mod.rs:2403`), yet `leaf_inline::rewrite_program` asks `non_sentinel_vars` before it (`leaf_inline.rs:1214`) and clears only the range memo (`:1157`): a process compiling several programs on one thread (the test harness, the REPL, the LSP) can answer from an earlier program's definition numbers — the failure the comment at `:123` describes.  Write the two-program reproduction first; then one `begin_program()` clearing every analysis memo, called at `leaf_inline.rs:1157` and `mod.rs:2403-2404` | the reproduction, red before, green after, both backends |
| B0.2 | `crate_layers.py`: forbidden layer pairs (`FORBIDDEN`, checked both ways, beside `BUDGET` at `:101`) | a self-test of the script |
| B0.3 | the new layers: `opt` (the five IR passes), `interp` (`runtime`'s interpreter modules: `compile`, `compiled_stdlib`, `fill`, `state`, `native`, `native_gate`, `lib_ids`, `extensions`, `parallel*`, `debugger`, `engine_host`, `live_*`, `profiler`, `stack`, `c_call`, `codegen_runtime`, `wasm_gl`), `lower` (empty), `rust` (`generation`, `create`, `native_lib` until B2, `introspect` until B2); today's crossings registered in `BUDGET` as the ratchet | `check` passes; no code change |

## B1 — `opt`: what every backend asks

Smallest closure first.

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

| step | change | check |
|---|---|---|
| B3.1 | `src/lower/mod.rs`: `pub struct ProgramPlan { reachable, dup_fn_names, dead_by_fn, value_records, apart, dv, ps, sw }` and `pub fn plan_program(data, stores, opts) -> ProgramPlan` — the computation of `output_functions` (`mod.rs:10878-10950`) and of 9161–9217, moved; `Output` holds `plan: Rc<ProgramPlan>` and its eight fields become reads of it | protocol |
| B3.2 | the ~30 `*_disabled` switches on `Output` → `lower::Switches`, read once per program; a second emitter honours the same switches | protocol; `NATIVE_SWITCHES.md` unchanged |
| B3.3 | the plan computations whose files write no Rust (`decode_view.rs`, `decode_structure_scan.rs`, `stream_walk.rs`, `apart_value.rs`, `apart_records.rs`, `destination.rs`, `view_return.rs`, `hand_over.rs`, `keyed_entry.rs`, `keyed_place.rs`, `file_hold.rs`, `ranged_call.rs`) → `src/lower/`, one per commit; where one is a method on `Output`, it becomes a free function over `(data, stores, &ProgramPlan, def_nr)` first, in the same commit | protocol each |
| B3.4 | the PER-FUNCTION decisions computed during emission (`vec_headers`, `rec_ptrs`, `ptr_windows`, the hoist tiers, …): a `FnPlan` built before a function is written, for each decision that can be made before writing — measured one by one; a decision that depends on what was already written stays in `rust` and is listed | protocol; the list in this file |
| B3.5 | the runtime surface: a census of every runtime item emitted Rust names (beyond `CODEGEN_RUNTIME_FNS`: store methods, `loft::state::`, `loft::native_lib::`), generated from the corpus's emission; the input to the C backend's runtime decision (@PLN91) | the census, committed as a report |
| B3.6 | `lower ↔ interp` forbidden at 0; `lower → rust` budget 0 | `check` |

## B4 — the crates

After @PLN191 has extracted `core` and `front`, in its manner (one crate per step, re-exports keep
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

## B6 — optional: a minimal binary

A by-product, built when wanted: a `native-codegen` feature (default on) gates `rust` and the
`--native` / `--native-release` / `--native-emit` / `--native-wasm` / `--html` /
`--native-android` / `build-native` arms (`main.rs:7835/7838/7915/7924/7945/~7934/8922`), which
answer one line without it; `make minimal` via `scripts/lean_build.sh --minimal`.  Its artifact
identity already equals the full build's (`cache.rs:231`: `feature_signature` (:119) lists
`threading`, `wasm`, `mmap`, `png`, `native-extensions`, `random`; `strict-stores` changes no
layout, `keys.rs:2836`) — B6 adds a test that every `[features]` name (`Cargo.toml:55-115`) is
classified as identity-relevant or not, and a CI comparison of both builds' corpus output.
Measured: the shipped binary is 25.2 MB (21.1 stripped), the generator ~12 % of its machine code.

## Order

B0 → B1 → B2 → B3 → B5 (the probe, from @PLN91) → B4 when @PLN191 is ready; B6 whenever wanted.
B1 and B2 make the interpreter a defined part on their own; B3 is what the C backend needs.

## Risks

- **B1.11 is the long one** (5.6k lines, ~5 commits), cut along the closure's own grouping.
- **B3.3–B3.4 turn methods into functions:** the step protocol's byte-identity is what keeps that
  honest; a decision that cannot leave the writer is listed, not forced.
- **Memos and switches move with their module**, never duplicated (B0.1's class of defect).
- **The runtime is Rust-shaped** (`Abi::Cell`); how a C program reaches the stores — a C runtime,
  or the Rust one through a C ABI (`loft-ffi` is one) — is @PLN91's decision; B3.5 gives it the
  facts.

## Open questions for the owner

1. B3.4's ambition: share the per-function hoisting decisions with the C backend, or start C
   with the program-level plans only and a plainer emission?
2. B6: wanted now, later, or not at all?
