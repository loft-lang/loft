<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN200 — a minimal interpreter: its own crates, the native generator apart

**Issue:** [loft-lang/plans#200](https://github.com/loft-lang/plans/issues/200).  **Builds on:**
[@PLN191](https://github.com/loft-lang/plans/issues/191) (the crate DAG, whose layer order this
plan amends) and [@PLN199](199-library-compiles-whole.md) (library artifacts, whose key this plan
moves from the executable to the runtime interface).

## Goal

A `loft` binary that parses, optimises, compiles to bytecode, interprets, and adopts compiled
library artifacts — built from crates that do not contain the native code generator, and
produced by one command (`make minimal`).

- **Every optimisation the interpreter can use stays available to it.**  The analyses both
  backends ask and the IR-phase rewrites live in a crate the interpreter depends on (`loft-opt`).
  Only native emission and the rewrites that exist for emitted Rust live in the generator's crate
  (`loft-native`).  This is also the direction [INTERPRETER_PERFORMANCE.md](../INTERPRETER_PERFORMANCE.md)
  already sets — move a native rule into the IR — given a structural home.
- **The best routines still run compiled.**  The standard library's loft bodies are compiled into
  the binary (`compiled_stdlib_gen.rs`, @PLN181), and a published library's artifact (@PLN199 G2c)
  is adopted, both with no `rustc`.  The program's own code is interpreted.

Not a goal: replacing the full `loft` (compiler, `--native`, tools), which stays and is built on
the same crates.  Shipping the minimal binary as a release asset is M6, the owner's call.

## Where it stands (measured 2026-10-23, macOS arm64, lean release build)

| | size |
|---|---|
| `loft` as `make-release.sh` builds it | 25.2 MB (21.1 MB stripped) |
| … of which machine code / symbol table | 15.4 MB / 4.7 MB (the bundle is not stripped) |
| the native generator's share of the machine code, by symbol (inlining smears it) | ~1.8 MB (~12 %) |
| `--no-default-features` (no registry, TLS, threads, mmap, remote store, `dlopen`) | 20.5 MB (17.2 MB stripped) |

So the size win is real but modest — a minimal binary near 15–17 MB stripped against 21 MB.  The
case for the split is the BOUNDARY: a binary that cannot emit Rust, a crate the interpreter's
optimisations live in without the emitter, and builds that do not recompile the generator.

**The tangle.**  @PLN191's layers are `core < front < gen < runtime < facade`, and the interpreter
(`compile`, `state`, `fill`) sits in `runtime`, ABOVE `gen`; five IR-phase rewrites sit IN `gen`
because they ask the generator's analyses.  The references into `src/generation/` from outside it:

| from | sites | what it asks |
|---|---|---|
| `state` (the bytecode emitter, `state/codegen.rs`, and `state/mod.rs`) | 6 | `hoist::borrowed_text_walks`, `hoist::char_walks` / `CharWalk`, `container_element_base` ×2, `ops::refvar_text_stack_variant` |
| `value_record` | 2 | `hoist::value_records`, `hoist::is_nullable_layout` |
| `leaf_inline` | 4 | `range::*` (the call memo, `begin_generation`), `non_sentinel::non_sentinel_vars` |
| `ctor_literal` | 2 | `hoist::ret_buffer_attr`, `hoist::loop_records` |
| `compiled_stdlib` | 2 | `generate` — the build-time half; the run-time half (`bridge`, `mark`) asks nothing |
| `native_lib` | 20 | artifact BUILDING and artifact ADOPTION in one module |
| `lib_placement` | 4 | the placed-library dispatch |
| `extensions`, `introspect` | 1 each | |

`src/generation/` is ~69k lines; `hoist.rs` alone is 19.5k and mixes analyses with emission.

**The artifact key.**  A library artifact runs only in the binary that built it (`loft_exe_identity`):
a release bundle is named by its version, commit, target, FEATURES and source stamp.  A minimal
build has other features, so today it adopts nothing.

## Design

The crates, bottom up — @PLN191's order with `gen` split in two and the interpreter between them:

```
loft-runtime   what compiled programs link (@PLN191 R): stores, keys, database, codegen_runtime, …
loft-core      the IR types: data, variables, typedef, diagnostics, …
loft-front     lexer, parser, scopes, the front-end IR passes (in_range, forward_walk, …)
loft-opt       the analyses both backends ask + the IR-phase rewrites (value_record, leaf_inline,
               ctor_literal, self_replace, zero_count, …)
loft-interp    the bytecode compiler (compile, state/codegen), the interpreter (state, fill), the
               interpreter's natives, the compiled standard library's run-time half, ADOPTING an
               artifact (load, probe, keys, HostTable)
loft-native    generation/ (emission and the native-only rewrites), BUILDING an artifact,
               `--native`, generating the compiled standard library
loft           the facade: every path that exists today, the full binary and the tools
loft-run       the minimal binary: loft-interp and nothing above it
```

The rules, each held by a check:

1. **`loft-interp` never depends on `loft-native`.**  Before the crates exist,
   `scripts/crate_layers.py check` holds it as a layer rule with a zero budget; after, cargo does,
   and a test asserts `cargo tree -p loft-run` names no `loft-native`.
2. **An analysis both backends ask is MOVED, never copied**, into `loft-opt`; the generator asks
   it there.  A copy is two answers to one question, and they drift.
3. **Every move is behaviour-neutral:** the emitted Rust and the bytecode are byte-identical over
   the corpus and the suite (`scripts/introspect_diff.sh`), the rewrite census does not move, and
   no interpreter routine gets slower (`make interp-gap`).
4. **An optimisation the interpreter can use goes to `loft-opt`**, not to `loft-native` — the
   placement rule for new work, recorded as a design decision in M1.
5. **An artifact is keyed by the runtime interface it was built against**, not by the executable,
   so `loft-run` adopts what the full `loft` of the same commit builds (M3).

## Phases

### M0 — what the minimal binary contains (owner decisions, then a size target)

Decide, then build a feature-trimmed variant of today's binary to measure each:

- **In:** parser, scopes, `loft-opt`, bytecode compiler, interpreter, runtime, the compiled
  standard library, artifact adoption (`native-extensions` — needed for `dlopen`), threads, mmap.
- **Owner's call:** `loft install` and the registry (TLS, zip, tar, ed25519: needed to fetch
  packages and their artifacts; without it the minimal binary runs only what the full one
  installed); the remote store; the REPL; the debugger (`loft debug`).
- **Out:** `--native` / `--native-release` / `--html` / `--native-wasm`, the LSP, the DAP,
  `gendoc`, the live tier, `build-native`, introspection dumps of emitted Rust.
- The name: a separate `loft-run`, or `loft` built without the native feature (one name on a
  user's PATH, two builds).
- Stripping: the bundle ships 4.7 MB of symbols today; strip the shipped binary (both builds) or
  keep them for crash reports — decided here, applied to both.

Check: a table of the variant sizes in this file, and the target the M5 binary is held to.

### M1 — the layer amendment: `opt`, `interp`, `native`

In `scripts/crate_layers.py`: split `gen` into `opt` and `native`, and move the interpreter
modules (`compile`, `state`, `fill`, the interpreter halves of `extensions`, `compiled_stdlib`'s
run-time half) to a new `interp` layer between them.  Then cut every reference the table above
lists, smallest first:

1. the IR passes' analyses (`range`, `non_sentinel`, `ret_buffer_attr`, `loop_records`,
   `is_nullable_layout`) move to `loft-opt` modules, re-exported from their old paths;
2. the bytecode emitter's four walk analyses (`borrowed_text_walks`, `char_walks`,
   `container_element_base`, `refvar_text_stack_variant`) likewise;
3. `hoist::value_records` last: it reaches deep into `hoist.rs`, so its closure is measured first
   (`crate_layers.py graph`) and the analysis/emission line inside `hoist.rs` is drawn by what that
   closure needs — not by moving `hoist.rs` whole;
4. `compiled_stdlib::generate` moves to the native side; `bridge` and `mark` stay;
5. `lib_placement`, `extensions`, `introspect`: each site moved to the side that uses it.

Check: `crate_layers.py check` at zero upward references with the new order; rules 2 and 3
(byte-identical emission and bytecode, census unchanged); the placement rule (4) recorded as a
`@C` design decision, cited by `crate_layers.py` and one guard.

### M2 — `native_lib` split: adopting an artifact vs building one

`native_lib` holds both halves (20 references into the generator).  Adoption — `init_one`,
`probe_and_mark_exports`, the `HostTable` and its id tables' host side (`lib_ids::HostIds`), the
key check (`artifact_matches_layout`) — moves to `interp`; building — `emit_program`,
`shared_cdylib_source`, `build_keyed_cdylib`, `cached_or_build_shared_cdylib`'s build branch, the
`rustc` driver — to `native`.  The run path in `main.rs` asks "is there an artifact to adopt"
through the `interp` half, and "build one" only where the native half is linked.

Check: `registry_e2e::a_released_library_artifact_runs_compiled_without_rustc` and the
`n2_cdylib` / `n3_use_native` suites unchanged.

### M3 — the artifact key is the runtime interface

`loft_exe_identity` today names the executable.  What an artifact relies on is narrower: the
commit (the generator that wrote it and the parse that marks its exports), the layout of the
stores it shares by memory (`loft-runtime`'s source stamp and the features that change its
types), `HOST_ABI`, the target and `rustc`.  Name THAT (`runtime_interface_id`), and key
release-bundle artifacts on it; a development build keeps its file-time identity (loft#1776).

Check: a test builds the full and the minimal binary of one commit and asserts the same
`build-native --identity`; another flips a runtime layout feature and asserts it differs.

### M4 — the crates

`loft-opt`, `loft-interp` and `loft-native` become crates in the workspace, in @PLN191's manner:
one crate per step, re-exports keep every path, emitted Rust names do not change.  Sequenced
with @PLN191 P3 (the first crate): the layer order M1 settles is the one P3 must cut along, so
M1 lands before P3.

Check: `cargo build -p loft-interp` builds without `loft-native` in the graph; the full gate.

### M5 — `make minimal` and its guards

`loft-run` (or the `loft` build M0 names), built by `make minimal` with the shipped tier's flags
(`scripts/lean_build.sh`).

- `--native` and the other emitters are refused with one line that says the binary cannot emit
  them and names the full `loft` — never a crash.
- A library without an adoptable artifact runs interpreted, as with no `rustc` today.
- **Guards:** the corpus and the suite under `loft-run` answer exactly as the full `loft
  --interpret` does (stdout and exit code, `tests/common/corpus.rs`'s list); `cargo tree`
  names no `loft-native`; a `registry_e2e` case installs and runs with `loft-run`, adopting the
  artifacts the full binary built (after M3); the size against M0's target, reported (not gated)
  in CI_BUDGET.md.

### M6 — shipping it (owner)

A runner bundle per released triple beside the full one, or the minimal binary as the default
with the full one opt-in.  Linux waits on loft#2127 (the static musl bundle cannot `dlopen`).

## Order

M0 → M1 → M2 → M3 → M4 → M5; M6 is the owner's.  M1 and M2 are useful on their own — the
interpreter's optimisations get a home without the emitter — even if the crates (M4) wait on
@PLN191.

## Open questions for the owner

1. M0's in/out list: is `loft install` (registry, TLS) part of the minimal binary?
2. One name or two: `loft-run`, or `loft` built without the native feature?
3. M6: ship the minimal binary at all, and which way round?
