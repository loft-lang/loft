<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN200 — a minimal interpreter: its own crates, the native generator apart

**Issue:** [loft-lang/plans#200](https://github.com/loft-lang/plans/issues/200).  **Builds on:**
[@PLN191](https://github.com/loft-lang/plans/issues/191) (the crate DAG, whose layer order this
plan amends) and [@PLN199](199-library-compiles-whole.md) (library artifacts, which the minimal
binary adopts).  Code references are file:line at commit `95d10dbe1`; a step re-reads its sites
before it moves them.

## Contents
- [Goal](#goal) · [Where it stands](#where-it-stands-measured-2026-10-23)
- [The design](#the-design) · [Invariants](#invariants) · [The step protocol](#the-step-protocol)
- Phases: [M0](#m0--contents-and-size-target) · [M1](#m1--the-opt-layer-the-interpreters-optimisations-without-the-emitter)
  · [M2](#m2--native_lib-adopting-an-artifact-apart-from-building-one) ·
  [M3](#m3--one-artifact-identity-for-the-full-and-the-minimal-build) ·
  [M5a](#m5a--make-minimal-from-the-single-crate) · [M4](#m4--the-crates) ·
  [M5b](#m5b--loft-run-on-the-crates) · [M6](#m6--shipping-it-owner)
- [Order](#order) · [Risks](#risks) · [Open questions](#open-questions-for-the-owner)

## Goal

A `loft` binary that parses, optimises, compiles to bytecode, interprets, and adopts compiled
library artifacts — built WITHOUT the native code generator, produced by one command
(`make minimal`), and later from crates of its own.

- **Every optimisation the interpreter can use stays available to it.**  The analyses both
  backends ask and the IR-phase rewrites move to an `opt` layer (later the crate `loft-opt`)
  that the interpreter depends on.  Only Rust emission and the rewrites that exist for emitted
  Rust stay with the generator (`native`, later `loft-native`).  This gives
  [INTERPRETER_PERFORMANCE.md](../INTERPRETER_PERFORMANCE.md)'s direction — move a native rule
  into the IR — a structural home.
- **The best routines still run compiled:** the standard library's loft bodies are compiled
  into the binary (`src/compiled_stdlib_gen.rs`, @PLN181), and a published library's artifact
  (@PLN199 G2c) is adopted, both with no `rustc`.  The program's own code is interpreted.

Not a goal: replacing the full `loft` (compiler, `--native`, tools); it stays, built on the same
code.  Shipping the minimal binary is M6, the owner's call.

## Where it stands (measured 2026-10-23)

macOS arm64, release builds:

| build | size |
|---|---|
| `loft` as `scripts/make-release.sh` builds it | 25.2 MB (21.1 MB stripped) |
| … machine code / symbol table | 15.4 MB / 4.7 MB — the bundle is not stripped |
| the generator's share of the machine code, by symbol (inlining smears it) | ~1.8 MB (~12 %) |
| `--no-default-features` (also without `dlopen`, so it adopts nothing) | 20.5 MB (17.2 MB stripped) |

A minimal binary near 15–17 MB stripped: a modest win.  The case is the BOUNDARY — a binary
that cannot emit Rust, a home for the interpreter's optimisations without the emitter, and
builds that do not recompile the generator.

**Layers today** (`scripts/crate_layers.py:52-96`, order = list order, checked by
`tests/extraction_hygiene.rs:734`): `core < front < gen < runtime < facade`.  `gen` holds
`generation` and five IR passes (`ctor_literal`, `leaf_inline`, `self_replace`, `value_record`,
`zero_count`); `runtime` holds the interpreter (`compile`, `state`, `fill`) ABOVE `gen`.

**What reaches into `src/generation/` from outside it:**

| from | site | item | where it is defined |
|---|---|---|---|
| `state/codegen.rs` | :4989 | `ops::refvar_text_stack_variant` | `src/text_calls.rs:107` (re-exported, `generation/ops/mod.rs:46`) |
| `state/codegen.rs` | :3348, :3589 | `container_element_base` | `generation/mod.rs:2116` — wraps `use_analysis::projection_container_var` |
| `state/codegen.rs`, `state/mod.rs` | :5289, :528 | `hoist::char_walks`, `CharWalk` | `hoist.rs:4494`, `:4473` |
| `state/codegen.rs` | :1758 | `hoist::borrowed_text_walks` | `hoist.rs:4700` |
| `value_record.rs` | :226, :244 | `hoist::value_records`, `is_nullable_layout` | `hoist.rs:14247`, `:13420` |
| `leaf_inline.rs` | :55, :1157, :1214, :1221 | `range::{Range, range, range_vars, begin_generation, Ranges}`, `non_sentinel::non_sentinel_vars` | `range.rs:56/151/498/476/62`, `non_sentinel.rs:223` |
| `ctor_literal.rs` | :226, :824 | `hoist::ret_buffer_attr`, `hoist::loop_records` | `hoist.rs:15655`, `:15415` |
| `compiled_stdlib.rs` | :123–124 | `duplicate_fn_names`, `disambiguated_fn_ident` (in `generate`, build time only) | `generation/mod.rs:463`, `:539` |
| `extensions.rs` | :845 | `reachable_functions` (in `reachable_unimplemented_natives`, run time) | `generation/mod.rs:402` |
| `introspect.rs` | :19, :510 | `generation::Output::new` (in `emit_rust`, the Rust section only) | |
| `lib_placement/dispatch.rs` | :287, :357, :388, :415 | `duplicate_fn_names`, `disambiguated_fn_ident` (run time) | |
| `native_lib.rs` | 20 sites | building AND adopting an artifact, one module | see M2 |

Every one of these is a PURE analysis over `Data`/`Stores`/`Value` — none of `hoist.rs`,
`range.rs`, `non_sentinel.rs` names `Output` or writes text.  Their state is thread-local memos
(`range.rs:462-471` `CALL_MEMO`/`LONG_DEPTH`; `non_sentinel.rs:123-133` `RETURNS`) and
`OnceLock` switches.  The closures, measured by a token call graph over `src/generation/`:

| item | closure inside `generation/` |
|---|---|
| `ret_buffer_attr`, `container_element_base`, `refvar_text_stack_variant` | none |
| `char_walks` | 3 items, ~176 lines (`text_written` `hoist.rs:4451`) |
| `loop_records` | 14 items, ~552 lines |
| `range` + `non_sentinel` | 35 items, ~1,023 lines (all of `range.rs` but `plain_form`; 8 of `non_sentinel.rs`; `CharWalk`, `iteration_head` 4307, `VectorIteration` 4291, `element_read` 4329, `iteration_step` 4369, `bounded_iteration_index` 6063 from `hoist.rs`) |
| `borrowed_text_walks` | 58 items, ~1,112 lines — 49 shared with `value_records`; the writer analysis `may_write_store` 8385 and its subtree |
| `value_records` | 177 items, ~5,621 lines (~29 % of `hoist.rs`), all in `hoist.rs` |
| the union | 233 items: 203 in `hoist.rs` (~6,656 lines, ~34 %), 20 in `range.rs`, 9 in `non_sentinel.rs`, 1 in `mod.rs` |

## The design

```
core      the IR types, stores, keys, … (@PLN191 core)
front     lexer, parser, scopes, the front-end IR passes (@PLN191 front)
opt       NEW: the analyses both backends ask (src/opt/…) and the IR-phase rewrites
          (value_record, leaf_inline, ctor_literal, self_replace, zero_count)
interp    NEW: compile (bytecode), state (interpreter + its bytecode emitter), fill, native
          (the interpreter's natives), compiled_stdlib's run-time half, artifact ADOPTION and KEYS
native    generation/ (emission, the native-only rewrites), artifact BUILDING, `--native`,
          compiled_stdlib's generation, create (fill.rs's generator)
facade    the binaries' shared code: repl, lsp, lib_placement, …
```

Physically: the analyses move into a new `src/opt/` directory (one file per subject), each
re-exported from its old `generation::…` path so no caller and no emitted name changes in the
same step.  Then a cargo feature `native-codegen` (default on) can leave `generation` and
everything above `interp` out: **`make minimal` exists from the single crate (M5a), before any
crate split.**  The crates (`loft-opt`, `loft-interp`, `loft-native`) come after, along the
same lines, when @PLN191 has extracted `core` and `front` (M4).

## Invariants

| # | invariant | held by |
|---|---|---|
| I1 | `opt` and `interp` name nothing in `native` | `crate_layers.py check` (zero budget once M1 ends); later cargo's dependency graph and a `cargo tree -p loft-run` test |
| I2 | an analysis both backends ask is MOVED, never copied — one home, re-exported from the old path | the step protocol (step 2) and review: a copy passes I3 until the two drift, so no later check would catch it |
| I3 | every move is behaviour-neutral: emitted Rust and bytecode byte-identical over the corpus and the suite | `scripts/introspect_diff.sh <before> <after>` → `IDENTICAL n/n`; `make rewrite-census` unchanged |
| I4 | no interpreter routine gets slower | `make interp-gap` per phase (a report; compared by hand) |
| I5 | an optimisation the interpreter can use lives in `opt` | a `@C` design decision (M1.14), cited by `crate_layers.py` and a guard |
| I6 | a minimal and a full release build of one commit adopt the same artifacts | M3's identity test |

## The step protocol

Every step below is ONE commit and follows the same recipe:

1. Build the BEFORE binary (`cargo build --release --bin loft`, copy it aside).
2. Move the item(s) to the new home; leave `pub use crate::opt::<m>::{…};` (or `pub(crate)
   use`) at the old path — the pattern @PLN191 P2 used: `src/fill.rs:1078-1079`,
   `src/codegen_runtime.rs:22-31`, `src/rpc.rs:25-28`.  Widen a visibility only as far as the
   new home needs (e.g. `non_sentinel::collect_escapes`, now `pub(super)`).
3. `cargo build` + both clippy variants; `scripts/introspect_diff.sh <before> <after>` must
   print `IDENTICAL`; `make rewrite-census` must not move; `python3 scripts/crate_layers.py
   check` with the budget lowered by what the step cut (`lower`).
4. The step's own targeted tests (named per step); `find_problems.sh --changed`.

A step that is not byte-identical is not a move: it is split, or it becomes its own change with
its own guard.

## M0 — contents and size target

| step | change | check |
|---|---|---|
| M0.1 | the owner decides the in/out list (below) | — |
| M0.2 | measure each optional piece by building today's binary with it off; record the table here | the sizes |
| M0.3 | the size target for M5a, and whether both binaries are stripped (4.7 MB of symbols today) | — |

- **In:** parser, scopes, `opt`, the bytecode compiler, the interpreter, the runtime, the
  compiled standard library, artifact adoption (`native-extensions`, `Cargo.toml:81` — `dlopen`),
  threads, mmap.
- **Owner's call:** `loft install` and the registry (`registry`, `Cargo.toml:90`: TLS, zip, tar,
  ed25519 — needed to fetch packages and their artifacts); `remote-store`; the REPL; `loft debug`.
- **Out:** `--native` (`main.rs:7835`), `--native-release` (7838), `--native-emit` (7915),
  `--native-wasm` (7924), `--html` (7945), `--native-android` (~7934), `build-native` (8922,
  except `--identity`, 8932), `introspect`'s Rust section (`introspect.rs:504`), the LSP and DAP
  binaries (`Cargo.toml:44-53`), `gendoc`, `create`.

## M1 — the `opt` layer: the interpreter's optimisations without the emitter

Smallest first; each step lowers a budget.  Closures and lines from the table above.

| step | change | check (beyond the protocol) |
|---|---|---|
| **M1.0** | **Fix first:** `non_sentinel`'s memo `RETURNS` (`non_sentinel.rs:123-133`) is cleared only by `Output::new` (`generation/mod.rs:2403`), but `leaf_inline::rewrite_program` asks `non_sentinel_vars` first (`leaf_inline.rs:1214`) and clears only the range memo (`:1157`) — a process that compiles several programs on one thread (the test harness, the REPL, the LSP) can answer from an earlier program's definition numbers, the failure the comment at `:123` describes.  Write the two-program reproduction first; then one `opt::begin_program()` that clears every analysis memo, called at `leaf_inline.rs:1157` and `mod.rs:2403-2404` | the reproduction, failing before, passing after; both backends |
| M1.1 | `crate_layers.py`: split `gen` into `opt` (the five IR passes) and `native` (`generation`, `create`); add `interp` between them with `runtime`'s modules (`compile`, `compiled_stdlib`, `fill`, `state`, `native`, `native_gate`, `lib_ids`, `extensions`, `parallel*`, `debugger`, `engine_host`, `live_*`, `profiler`, `stack`, `c_call`, `codegen_runtime`, `wasm_gl`); `native_lib` and `introspect` to `native` until M2/M1.13.  Register today's back-edges in `BUDGET` (the ratchet @PLN191 P2 used) | `check` passes with the new budgets; no code change |
| M1.2 | `state/codegen.rs:4989`: call `crate::text_calls::refvar_text_stack_variant` directly (the parser already does, `parser/objects.rs:3675`) | protocol |
| M1.3 | `container_element_base` (`mod.rs:2116`) → `src/opt/projection.rs`; `state/codegen.rs:3348/3589` and `dispatch.rs:415` ask it there | protocol |
| M1.4 | `ret_buffer_attr` (`hoist.rs:15655`) → `src/opt/attrs.rs` (23 emitter mentions keep the re-export) | protocol |
| M1.5 | `char_walks`, `CharWalk`, `text_written` (`hoist.rs:4451-4494+`) → `src/opt/char_walk.rs`; `Output`'s `CharWalk` field and `range_vars`' signature (`range.rs:503`) name it there | protocol |
| M1.6 | the iteration helpers `VectorIteration` 4291, `iteration_head` 4307, `element_read` 4329, `iteration_step` 4369, `bounded_iteration_index` 6063 → `src/opt/iteration.rs` | protocol |
| M1.7 | `range.rs` and `non_sentinel.rs` whole → `src/opt/range.rs`, `src/opt/non_sentinel.rs` (`collect_escapes` to `pub(crate)`); `ranged_call.rs` and `hoist.rs:19309` follow the re-export | protocol; `make interp-gap` |
| M1.8 | `loop_records`, `LoopRecords`, `LoopRecord` (`hoist.rs:15259-15415+`) and their 11 helpers (`all_scalar_record` 8425, `is_scalar` 9023, `group_covers_type` 10843, `refillable_plain` 11077, `mint_type_of` 11231, `is_var` 11496, `vector_field_offsets` 15280, `vector_field_numbers` 15300, `loop_record_zeros` 15323, `SCALAR_GETTERS` 806, `IN_PLACE_SET_OPS` 157) → `src/opt/loop_records.rs`.  `mint_type_of` is also an `impl Output` user: the free function moves, the method calls it | protocol |
| M1.9 | the WRITER analysis — `may_write_store` 8385 → `writes_store` 8389 → `blocks_header_hoist` 8635 → `call_writes_store` 8786 and its subtree (`in_place_only_writer` 8849, `retbuf_only_writer` 8908, `native_op_is_store_free` 9005, `frees_a_record` 8492, `null_buffer_alloc` 8444, `fresh_write` 8604, `fresh_text_set` 8541, the mint/path family 7699–7839, `HoistOwned` 354, `HoistTiers` 401, `PathKey`/`vector_path` 300/306, `view_field` 859, `sub_record` 925, `in_place_copy` 188, the op tables 49/91/157/8419/8846/8961, three `OnceLock` switches 290/7839/8597) → `src/opt/writes.rs`, in two commits (the tables and paths, then the writer functions) | protocol; the emitter's users (`mod`, `emit`, `apart`, `append_twin`, `keyed_entry`, `keyed_place`, `ops/vector_ops`) build through the re-exports |
| M1.10 | `borrowed_text_walks` (`hoist.rs:4700`), `TextBorrow` 4441, `SplitTable` 9306, `text_walk_head` 4408, `as_emitted_by_tables` 4652, `text_escapes(_with)` 5949/5957 → `src/opt/text_borrow.rs` | protocol |
| M1.11 | the naming helpers `reachable_functions` (`mod.rs:402`), `duplicate_fn_names` (:463), `disambiguated_fn_ident` (:539) → `src/opt/names.rs`; `extensions.rs:845`, `lib_placement/dispatch.rs:287/357/388/415`, `compiled_stdlib.rs:123-124` ask them there | protocol |
| M1.12 | `value_records` (`hoist.rs:14247`), `ValueRecords` 13285 + impl 13505, `ValueTuple` 13347, `is_nullable_layout` 13420 and the rest of the 177-item closure → `src/opt/value_records/` by sub-subject, bottom-up in ~5 commits: (a) body writes `body_writes` 2405, `element_target` 2221, `setter_target` 2129, `callee_writes` 2625, `rebound_vars` 692, `link_targets` 718; (b) leaves and view fields 15821–16979; (c) shapes and presence 17140–18493; (d) sites and reads 18565–19156; (e) the candidates, the fixpoint and the fn-ref groups 13285–15742.  `super::fnref::…` (`hoist.rs:14371/14398/14682`) becomes `crate::fnref::…` | protocol at every commit; `make interp-gap` at the last |
| M1.13 | `introspect::emit_rust` (`introspect.rs:504`) → `generation/introspect_rust.rs`; `introspect` keeps the IR, bytecode, slots and ownership sections and asks the Rust section through one function `main.rs` passes in (`main.rs:12632`) — so `introspect` joins `interp` | `tests/introspect*.rs` |
| M1.14 | record the placement rule (I5) as a design decision (`./scripts/idx next-decision`), cited in `crate_layers.py`, `src/opt/mod.rs` and a guard | `make decisions` |
| M1.15 | `crate_layers.py lower`: `opt→native` and `interp→native` at 0 except `native_lib`'s (M2) | `check` |

## M2 — `native_lib`: adopting an artifact apart from building one

`src/native_lib.rs` is 3,675 lines; its adopting and keying side names only the three helpers
M1.11 moved.  Classes by item:

| class | items (native_lib.rs line) |
|---|---|
| **ADOPT** | `mark_native_exports` 409, `mark_exports` 424, `BridgeProbe` 439, `probe_and_mark_exports` 481, `init_one` 535/543, `mark_library_native` 630, `LibArg` 732, `HostTable` 772, `HOST_ABI` 805, `HOST_TABLE` 808, `HOST_COROUTINES` 821, `interpreter_for` and `host_coroutine_*` 835–903, `IdTable` 1215, `LAYOUT_FP_SYMBOL` 1330, `layout_fp_of_relocated` 2578/2592, `LayoutProbe` 2613, `artifact_matches_layout` 2725, `layout_fp_off_path` 2768, `relocate_for_probe` 2810, `report_unprobeable` 2830 |
| **KEY** | `library_export_set` 554, `library_candidates` 572, `is_synthetic_name` 640, `hidden_dest_type_name` 1702 (also `state/io.rs:997`), `platform_cdylib_name` 2119, `auto_cdylib_stem` 2528, `mix_fp` 2546, `artifact_sources` 2851, `contributing_fingerprint` 2908, `type_layout_fingerprint` 2930, `fn_numbering_fingerprint` 3124, `source_content_hash` 3147, `source_newer_than` 3219 |
| **TOOLCHAIN** | `rustc_available` 2420, `rustc_version` 2427, `NO_RUSTC` 2434, `toolchain_failure_hint` 2448 |
| **BUILD** | everything else: `generate_interface` 54, the rlib pickers 119–375, `HOST_INIT_SRC` 956, `IN_HOST_TABLE_SRC` 992, `emit_program` 1013, `linked_packages` 1061, `library_code_rs` 1107, `id_tables_src` 1120, the `*_cdylib_lib_rs`/`shared_cdylib_source` 1255–1293, `export_wrapper`/`shared_bridge_wrapper` 1359/1408, the bridge readers 1720–1806, `find_loft_rlib` 1932 and the link helpers to 2150, `build_shared_cdylib` 2210, `build_keyed_cdylib` 2230, `prune_artifacts` 2662 |
| **MIXED** | `cached_or_build_shared_cdylib` 2971–3117: the key 2978–3037, the fresh check 3053–3077 (ADOPT), the build branches 3079–3116 (BUILD) |

| step | change | check |
|---|---|---|
| M2.1 | KEY items → `src/native_key.rs` (`interp`), re-exported from `native_lib` | protocol; `tests/n2_cdylib.rs`, `n3_use_native.rs` |
| M2.2 | TOOLCHAIN items → `src/toolchain.rs` (`interp`: `main.rs:3228/10061` and `test_runner.rs:2419` report it) | protocol |
| M2.3 | ADOPT items → `src/native_adopt.rs` (`interp`).  `LibArg` and `IdTable` are named by EMITTED Rust (`generation/emit.rs:2100-2118`): the `native_lib::` re-exports keep those paths | protocol; the emitted text unchanged |
| M2.4 | split `cached_or_build_shared_cdylib` into `adopt_shared_cdylib(…) -> Adopt { Fresh(so), Stale(key), Missing(key) }` (the key and fresh check, `interp`) and `build_shared_artifact(key, …)` (`native`); the old function becomes the composition, so `main.rs:9963` and `:9025` are unchanged | protocol; `registry_e2e::a_released_library_artifact_runs_compiled_without_rustc`; `n3_parity` |
| M2.5 | `native_lib` (BUILD only) moves to `native` in `crate_layers.py`; `interp→native` budget 0 | `check` |

## M3 — one artifact identity for the full and the minimal build

What a release bundle's artifact is keyed on (`src/cache.rs`): `exe_identity_of` (:231) =
`BUILD_ID` (:108) + `LOFT_VERSION` (:103) + `host_triple()` (:427) + `feature_signature()` (:119)
+ `build_identity_tag()` (:171, the source stamp of `src/`, `default/`, `loft-ffi/src`,
`Cargo.toml`/`.lock` — `build.rs:319` — plus the rustflags and `debug_assertions`).
`feature_signature` lists `threading`, `wasm`, `mmap`, `png`, `native-extensions`, `random`;
`strict-stores` changes no layout (`keys.rs:2836`: a `cfg!` test only).  A minimal build that
differs only by `native-codegen`, `registry` and `remote-store` therefore ALREADY answers the
same identity — M3 makes that a rule, not a coincidence.

| step | change | check |
|---|---|---|
| M3.1 | document `feature_signature` as THE list of features that change what an artifact or a cached image sees, and list beside it the features deliberately absent (`native-codegen`, `registry`, `remote-store`, `strict-stores`, and the tooling ones) | a unit test that every `[features]` name of `Cargo.toml:55-115` is in exactly one of the two lists — a new feature must be classified |
| M3.2 | the identity printer moves out of `build-native --identity` (`main.rs:8932`) into `native_key::identity_lines()`, printed by both binaries (the minimal one under `--artifact-identity`) | — |
| M3.3 | the guard: CI builds both binaries of one commit as release bundles and compares their identity lines | the CI step (M5a.4) |

## M5a — `make minimal` from the single crate

Possible as soon as M1 and M2 hold `interp→native` at 0: one feature leaves the generator out.

| step | change | check |
|---|---|---|
| M5a.1 | feature `native-codegen` (in `default`) gates `pub mod generation;` (`lib.rs:399`), the BUILD module, `create`, `introspect_rust`, and the dispatch arms of M0's out-list in `main.rs` | the full build unchanged (protocol); the minimal build compiles |
| M5a.2 | without the feature, each out-list flag answers one line — the binary cannot emit native code, use the full `loft` — and exits 2; a library without an adoptable artifact interprets, as with no `rustc` today | `tests/minimal.rs` (runs only when the minimal binary exists) |
| M5a.3 | `make minimal`: `scripts/lean_build.sh` gains `--minimal` (`--no-default-features --features mmap,random,threading,native-extensions` + M0's choices, into `target/minimal`) | the binary exists, its size reported |
| M5a.4 | CI job: build both; `scripts/minimal_diff.sh` runs the corpus and the suite (`scripts/corpus_files.sh`, as `introspect_diff.sh:52`) under the minimal binary and the full `--interpret`, comparing stdout and exit code; the identity lines (M3.3); clippy over the minimal feature set | the job |
| M5a.5 | a `registry_e2e` case that installs and runs with the minimal binary, adopting artifacts the full one built | the test |

## M4 — the crates

After @PLN191 has extracted `core` and `front` (its P3 onward; no local document — the issue is
the plan), and in its manner: one crate per step, `pub use` re-exports keep every path, emitted
Rust names unchanged.  The root `Cargo.toml` has no `[workspace]` yet (sibling crates `loft-ffi`,
`plugin-api`, `fuzz` are path dependencies); `fuzz/Cargo.toml:14-17` and
`tools/zt-c-web-staging/web/wasm/Cargo.toml:21` depend on `loft` and keep doing so through the
facade.

| step | change | check |
|---|---|---|
| M4.1 | `crates/loft-opt` from `src/opt/` and the five IR passes | full gate; I3 |
| M4.2 | `crates/loft-interp` from the `interp` layer | full gate; I3 |
| M4.3 | `crates/loft-native` from the `native` layer; the `native-codegen` feature becomes the optional dependency on it | full gate; I3 |

## M5b — `loft-run` on the crates

| step | change | check |
|---|---|---|
| M5b.1 | a `loft-run` binary (or `loft` itself, per M0) depending on `loft-interp` only; `make minimal` builds it | M5a's guards |
| M5b.2 | `cargo tree -p loft-run -e normal` names no `loft-native` | a test |

## M6 — shipping it (owner)

A runner bundle per released triple beside the full one, or the minimal binary as the default
with the full one opt-in.  Linux waits on loft#2127 (the static musl bundle cannot `dlopen`).

## Order

M0 → M1.0 → M1.1–M1.15 → M2 → M3 → M5a → M4 → M5b; M6 is the owner's.  M1 and M2 are useful on
their own; M5a gives the minimal binary before any crate exists.

## Risks

- **M1.12 is the long one** (5.6k lines, ~5 commits).  Its sub-steps are cut along the closure's
  own grouping; a sub-step that would need a type from a later one moves both.
- **The writer analysis (M1.9) is shared with ~25 emitter-only hoist functions**; they keep
  working through the re-exports, and are not moved.
- **Thread-local memos and `OnceLock` switches move with their module** — never duplicated, or
  two caches answer one question (M1.0's class of defect).
- **`lib_placement` is in `facade`** (`crate_layers.py`) but its run-time marking
  (`dispatch.rs:286`, `:377`) is needed by the minimal binary: it stays in the single crate for
  M5a and moves to `interp` before M4.2.

## Open questions for the owner

1. M0's in/out list: is `loft install` (registry, TLS) part of the minimal binary?
2. One name or two: `loft-run`, or `loft` built without `native-codegen`?
3. M6: ship the minimal binary, and which way round?
