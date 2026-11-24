<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 225 — Where the rustc writer is used

Every place, grouped by what removes it.  Each group names the command that measures it, so a
phase reads its own progress rather than this page's numbers.

## Writer-only source — deleted in R7

| File(s) | Lines |
|---|---|
| `src/generation/` (`mod.rs`, `emit.rs`, `calls.rs`, `dispatch.rs`, `coroutine.rs`, `kept.rs`, `pre_eval.rs`, `ranged_call.rs`, `stream_walk_emit.rs`, `text.rs`, `fault_flag.rs`, `apart_emit.rs`, `apart_records_emit.rs`, `decode_structure_emit.rs`, `decode_view_emit.rs`, `ops/` × 13) | ~40 100 |
| `src/codegen_runtime.rs` — the runtime the generated Rust links | 7 301 |
| `src/stdlib_codegen.rs`, `src/compiled_stdlib.rs`, `src/compiled_stdlib_gen.rs` (@PLN181, 33 functions) | ~2 600 |
| `src/native_build.rs`, `src/native_lib.rs` (rustc builds, the `@C71` shared cdylib), `src/native_release_args.txt` | ~3 000 |

`wc -l src/generation/*.rs src/generation/*/*.rs src/codegen_runtime.rs src/stdlib_codegen.rs src/compiled_stdlib*.rs src/native_build.rs src/native_lib.rs`

## Writer parts in shared source — cut in R2 and R7

`git grep -c -iE "rustc|native-rust|rust_backend_selected|LOFT_NATIVE_BACKEND|generation::|codegen_runtime" -- 'src/*.rs' 'src/**/*.rs'`
minus the files above.  Heaviest: `src/main.rs` (159: the CLI, the target routing, the
`LOFT_NO_RUSTC` gate), `src/native_utils.rs` (43: the `--html` rustc link, cargo layouts),
`src/cache.rs` (28), `src/platform.rs` (27), `src/toolchain.rs` (21: rustc detection),
`src/cargo_layout.rs` (18), `src/test_runner.rs` (15), `src/c_driver.rs` (15:
`rust_backend_selected`, the android route), `src/data.rs`, `src/keys.rs`, `src/parser/objects.rs`,
`src/parser/expressions.rs`, `src/parser/control.rs` (the `#native`/`#rust` callee rules),
`src/android.rs`, `src/use_analysis.rs`, `src/c/body.rs`.  The C headers `loft_rt.h`,
`loft_io.h`, `loft_keyed.h`, `loft_ffi_host.h` name `codegen_runtime` only in comments ("the twin
of"): those comments are reworded, not the code.

## The standard library — kept

The `#rust "…"` bodies in `default/*.loft` (`grep -c '^#rust' default/*.loft`) are not the
writer's: `create.rs` generates the interpreter's `src/fill.rs` from the operators' bodies, a
non-operator body marks a primitive `src/native.rs` registers, and about twenty IR-pass sites read
"has a body" as "a primitive the runtime implements" (`git grep -n "rust().is_empty()" -- src`).
Deleting them would change the IR, not remove text.  The parser accepts the annotation only in the
standard library.  Only the writer-only reader went: `create::generate_lib`.

## User-facing surface — R2

- Flags: `--native-rust`; `--native-emit` keeps its C meaning only.
- Environment: `LOFT_NATIVE_BACKEND=rust` (refused, then removed), `LOFT_NO_RUSTC` (moot after
  R7), the writer's `LOFT_NO_*` bisect switches (`NATIVE_SWITCHES.md`), `FALSIFY_NATIVE=rust`.
- Targets: `--html` / `--native-wasm` fall back to the writer when no WASI clang links
  (`src/main.rs`, after "A page or a wasm module builds through the C by default"), and
  `--native-android` takes it with `--native-rust`.
- Execution model: `@C71` — an interpreted program's used libraries run as writer-built cdylibs
  sharing its `Stores` (`native_lib::build_shared_artifact`), and the compiled stdlib.

## Libraries — R3

The bench checkouts' `loft.toml` files show no Rust-only native part (`server`, `web` and `ssh`
declare `[native] component = "net"`, the C component).  Published releases are the real
census: `revalidate-libs` under `LOFT_NO_RUSTC=1` over every release in the index.  Precedent:
`crypto` 0.3.x, a Rust-only native part, already deprecated for a release with a C part.

## Tests — R4

165 test files drive the writer or name its leg:
`git grep -lE "native-rust|LOFT_NATIVE_BACKEND|FALSIFY_NATIVE=rust|generation::Output|--native-emit|rust_backend_selected" -- tests`.
118 of them build through `generation::Output` or `--native-emit` (`tests/native.rs`,
`tests/wrap.rs`, `tests/testing.rs` and the cross-mode `code!` users among them);
`tests/layout_golden.rs` checks `generation::rust_type`; `tests/issues.rs` calls
`generation::reachable_functions`.

**Patch receipts:** 125 of 863 patch the writer —
`grep -lE '^\+\+\+ [ab]/src/(generation/|codegen_runtime\.rs|stdlib_codegen\.rs)' tests/falsified/*.patch`.

**Language-suite cells armed for the rust leg** (`git -C ../loft-suite grep -l "FALSIFY_NATIVE=rust" -- rules`):
`binding/B-Scope/2389-…`, `binding/B-Scope/a-name-bound-in-sibling-loops-…`,
`calls/F-OneBody/2318-…`, `coroutines/G-Yield/a-generator-keeps-its-links-…`,
`coroutines/G-Yield/a-generator-yields-a-tuple-…`, `dispatch/Disp-Dynamic/a-runtime-selection-…`,
`matching/M-Exhaust/2390-…`, `ownership/O-Override/2319-…`,
`rewrites/R-Base/a-byte-read-through-a-held-header-…`, `types/C-Never/a-diverging-arm-…`,
`types/N-Shape/2316-…`.  The suite runner's daily rustc-native leg goes with them.

## Tooling — R6

**Scripts** (`grep -liE "native-rust|LOFT_NATIVE_BACKEND|FALSIFY_NATIVE|--native-emit|generation/|codegen_runtime" scripts/* bench/portal/*`):
`after.py`, `check_doc_drift.sh`, `ci_failure_digest.py`, `crate_layers.py`, `decline_census.py`,
`emission_audit.py`, `falsify.sh`, `file-sizes.py`, `find_problems.sh`, `free_licence_audit.py`,
`gap_report.py`, `interp_gap.py`, `ir_walker_audit.py`, `native_attrib.py`,
`native_call_census.py`, `native_ratio.sh`, `p09_fast_gate.sh`, `profile.sh`, `receipts_ci.py`,
`rewrite_census.py` (the rust lane), `rule_phases.py` (the native column), `runtime_surface.py`,
`slot_matrix.py` and `slot_matrix.rust.baseline`, `slot_reach.tsv`, `test_subjects.sh`,
`wide_ledger.py`, `bench/portal/hand_price.sh`.  Some only name the writer in a list of paths;
each is read before it is cut.

**Workflows:** `ci.yml` (the `LOFT_NATIVE_BACKEND` pins and the rewrite census's two-writer
step), `miri.yml` (the rust slot matrix against `slot_matrix.rust.baseline`, the toolchain
matrix), `browser-threads.yml` (`LOFT_NATIVE_BACKEND: rust`), `derive.yml`, `codeql.yml`.

**Release:** `scripts/make-release.sh` ships the runtime rlib a `--native-rust` build links
and pins rustc's version for it; both go.  `make check-rlib`, `make compiled-stdlib`.

## Documentation — R8

**Working docs, formal chapters and skills** (61; closed plans, `-history.md` and `releases/`
stay as history):
`git grep -lE "native-rust|rustc writer|LOFT_NATIVE_BACKEND|--native-emit|codegen_runtime|src/generation" -- 'doc/claude/*.md' 'doc/claude/formal/*.md' CLAUDE.md '.claude/skills/*/SKILL.md' | grep -vE -- "-history.md|/plans/|/lib_plans/|/releases/|/presentations/"`.

**Plans:** every plan directory that names the writer
(`git grep -lE "native-rust|rustc writer|LOFT_NATIVE_BACKEND|codegen_runtime|src/generation" -- doc/claude/plans | grep -v /finished/`)
is read against its issue's state: a closed plan stays as history; an open one is reconciled —
@PLN219 (rust slot links) is declined, @PLN181 and @PLN211 are amended, @PLN91's
`rust-free-build.md` is checked off.
