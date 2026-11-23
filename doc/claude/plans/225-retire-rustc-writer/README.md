<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 225 — Retire the rustc writer

## Status

Open, `status:next` on [loft-lang/plans#225](https://github.com/loft-lang/plans/issues/225).  Design
and inventory written; no phase started; R1's library half done in @PLN206.  Resume:
[handoff-2026-11-23.md](handoff-2026-11-23.md).  Every count below was measured on
`tuxedo-pr2606-next`, and each list in [inventory.md](inventory.md) carries the command that
measures it again.

## Goal

loft has two backends, the interpreter and the C backend.  The rustc writer (`--native-rust`,
`LOFT_NATIVE_BACKEND=rust`, `src/generation/`, the `#rust` bodies, the runtime rlib and every
build path that runs `rustc` on loft's output) is removed, together with every rewrite only it
applies.

## The ruling this plan carries out

**A backend stays only while it is semantically equal to the others.  When keeping it would
need a choice between equality and the backend, the backend retires** (owner, 2026-11-21).  The
trigger is security #61: a null text becomes an identity rather than the content `"\0"`, which the
writer's owned `String` cannot spell without converting every text operation it emits.  The
ruling is recorded as design decision `@C170` in R0, with its sites and a guard.

## Size

| What | Count |
|---|---|
| Writer-only source (`src/generation/`, `codegen_runtime.rs`, `stdlib_codegen.rs`, `compiled_stdlib*.rs`, `native_build.rs`, `native_lib.rs`) | ~53 000 lines |
| Other `src/` files with a writer part (`main.rs` 159 mentions, `native_utils.rs`, `cache.rs`, `platform.rs`, `toolchain.rs`, `cargo_layout.rs`, …) | 16 files |
| `#rust` bodies in `default/*.loft` | 369 |
| Rewrites applied only by the writer | 22 ([rewrite-audit.md](rewrite-audit.md)) |
| Test files that drive the writer or name its leg | 165 |
| Patch receipts that patch the writer | 125 of 863 |
| Suite cells armed for its leg (`FALSIFY_NATIVE=rust`) | 11 |
| Workflows with a rustc lane / scripts with one | 5 / ~27 |
| Working docs, formal chapters and skills that describe it | ~60 (closed plans stay as history) |

## What depends on it besides `--native-rust`

Three routes run `rustc` today without anyone asking for the writer by name; each needs its own
answer before the code can go:

1. **Interpreted programs call their libraries compiled** (`@C71`, "native libraries compile,
   scripts interpret"): `native_lib::build_shared_artifact` writes a used library through the
   writer into a cdylib that shares the interpreter's `Stores`.  The standard library's 33
   compiled functions (@PLN181, `src/compiled_stdlib_gen.rs`) are the same writer output, built
   into the binary.  Without the writer both run their loft bodies interpreted, or need another
   compiled route (open question O1).
2. **`--html` and `--native-wasm` fall back to the writer** when no WASI clang links
   (`src/main.rs`, PR #2516: the C default never refuses what built before).  Removing the
   fallback breaks that promise on purpose (O2).
3. **`--native-android`** keeps the writer as its bisect path.

## Phases

Each phase can go red on its own and is compared against something exact.  The census from R0
is the work list every later phase shrinks; R7 may start only when it is empty.

| Phase | What | Validated by | Effort |
|---|---|---|---|
| **R0** | Record `@C170`.  Run the whole test suite, the language suite, `revalidate-libs` and the corpus with `LOFT_NO_RUSTC=1` (nothing builds through rustc; a request for it is refused) on Linux and macOS, and file the result as the rust-free census: every test, cell, library and target that fails or refuses. | The census is a list that can be non-empty: it is the plan's measured work list. | S |
| **R1** | The library route (`@C71`): **done in @PLN206** (closed) on `tuxedo-pr2606-next` — a used library runs as its hosted C unit, shipped per release, no rustc.  **Open: the compiled stdlib** (@PLN181, `src/compiled_stdlib_gen.rs`), still writer output built into the binary, through the C backend too (O1). | Parallel run on the library test corpus: every interpreted program answers the same with its libraries built by C and by rustc (value, length, leak), and the time ratio C / rustc is recorded. | L+ (in @PLN206) |
| **R2** | Targets: `--html` / `--native-wasm` without the writer fallback; `--native-android` without its rust path; `--native-rust` and `LOFT_NATIVE_BACKEND=rust` refused with a message that names `--native`. | The html, wasm and android test binaries pass under `LOFT_NO_RUSTC=1`; the refusals are tests. | M |
| **R3** | Libraries: a registry census of published releases that need rustc (a Rust native part, a `#rust` body in library source).  Each gets a C successor or is deprecated in the registry, as `crypto` 0.3.x was.  Then the hand-written Rust native part goes (@PLN206 P6's route 3, [p6-design.md](../206-one-store-model/p6-design.md) § Route 3): a `[library] native` cdylib cargo builds and the interpreter loads, `install::fetch_prebuilt` and a library version's `binaries` — a library that declares one is refused, naming a `[c] shim`. | `revalidate-libs` green under `LOFT_NO_RUSTC=1`; the census list empty. | S–M |
| **R4** | Tests and guards: each of the 165 test files keeps its interpreter and C halves and loses its rust leg; tests that exist only for the writer go; the 125 receipts are classified (writer-only → retire with that half; also guarding C or the interpreter → re-derived against that code); the 11 suite cells lose `ARMED: FALSIFY_NATIVE=rust` and are re-falsified on C or the interpreter, or lose their native half. | `make rule-coverage`: no rule loses its last guard except the rewrites R5 removes; the R0 census has no test left. | L |
| **R5** | The 22 writer-only rewrites: their formal definitions, decision code in the IR pass, the interpreter and the C backend, census names, `REWRITE_REASONS.md` entries and guard cells ([rewrite-audit.md](rewrite-audit.md)).  The shared rewrites lose only their rust half (with R7). | The C backend's emission over the whole corpus is byte-identical before and after (`scripts/c_sweep.py` emission diff), and the rewrite census of every kept rule is unchanged. | M |
| **R6** | Tooling and CI: the rustc lanes of 5 workflows, ~27 scripts (rewrite census, `rule_phases` native column, `slot_matrix.rust.baseline`, `falsify.sh` `FALSIFY_NATIVE`, `emission_audit.py`, `native_ratio.sh`, the perf portal's native columns), release bundles without the runtime rlib, `make check-rlib` and `make compiled-stdlib`, `src/toolchain.rs`'s rustc checks. | A CI job with no `rustc` on `PATH` beyond building loft itself runs the full gate green. | M |
| **R7** | Delete the writer: the writer-only source, the 369 `#rust` bodies and the parser's `#rust` support (an annotation left in a library becomes an error naming the cure), the writer parts of the 16 shared files, `LOFT_NO_RUSTC` (moot). | Full CI green; the rust-free census still empty; binary size and build time measured before and after. | M |
| **R8** | Documentation, phase by phase (see Documentation below), and security #61 unblocked. | `make docs-lint`, `scripts/check_doc_drift.sh`, `make features-check`. | M |

**Order:** R0 → R1, R2, R3 (independent) → R4 → R5 → R6 → R7.  R8 lands with each phase, never
as a trailing docs pass.

## Open design questions

- **O1 — the compiled route for libraries and the stdlib (`@C71`): DECIDED (owner, 2026-11-21).**
  Every library stays compiled, through the C backend instead of rustc: a used library is built
  into a shared object by the C backend and called from the interpreter over the shared store,
  as the writer's cdylib is today.  `@C71`'s model holds; its mechanism (the rustc fingerprint,
  `libloft.rlib`) is amended in R1.  **That route is @PLN206** ("one store model", active): its
  P3 makes one store layout, P4 the hosted C unit (a C body working on the interpreter's
  stores — the bridge), P5 has libraries build their C artifact and the interpreter adopt it,
  P6 removes the rustc artifact path.  R1 is @PLN206 P3–P6, designed there, not here.  The
  compiled stdlib takes the same route.
- **O2 — `--html` / `--native-wasm` without a WASI clang.**  Refuse with install instructions
  (cheapest), fetch the toolchain on first use, or ship it in the bundle (size).  Any of the
  three breaks PR #2516's promise; the change carries `freeze:broken`.
- **O3 — retired diagnostics.**  Codes only the writer emits are a frozen surface
  (`DIAGNOSTICS.md`); they retire by its procedure, never by deletion.

## Cross-arc dependencies

- **Unblocks** security #61 (a null text is an identity on every representation).
- **Depends on** @PLN206 P3–P6 for R1 (libraries compiled through C, run by the interpreter).
- **Reconciles** @PLN181 (compiled stdlib, reverted or replaced by O1), @PLN219 (rust slot
  links: moot), @PLN211 (rule phases: the native column goes), @PLN91 (`rust-free-build.md`),
  @PLN199 / @PLN203 (C carries every program; the C default).
- **Decision** `@C71` is reopened by O1; `@C170` is new.
- **Supersedes** `ROADMAP.md`'s paragraph that keeps the writer as the reference the self-hosted
  ports are compared with, until @PLN91's stage 6.  No @PLN91 gate reads the writer's output:
  the harness calls `--interpret`, `--emit-image`, `--emit-run-image` and `--emit-c`
  (`rust-free-build.md`, the table of what is shared); its end state only listed
  `src/generation/` among what moves to the reference repository.  The paragraph and that list
  are amended in R0.

## Documentation

- `CLAUDE.md`: the opening (two backends), the debugging policy's step 6, the environment switches.
- `doc/claude/NATIVE.md`, `NATIVE_SWITCHES.md`, `CODEGEN_METHOD.md`, `COMPILER.md`,
  `C_BACKEND.md`, `INTERMEDIATE.md`, `REWRITE_REASONS.md`, `KERNELS.md`, `PERFORMANCE.md`,
  `PROFILING.md`, `TESTING.md`, `SLOT_MATRIX.md`, `CI_BUDGET.md`, `HTML_EXPORT.md`, `WASM.md`,
  `PACKAGES_TARGETS.md`: the writer's parts removed or moved to their `-history.md`.
- `doc/claude/formal/rewrites.md` (the 22 rules and every rust half), `VERIFICATION.md`,
  `facts.md`, `heap.md`, `layout.md`, `operational.md`.
- `DESIGN_DECISIONS_PLATFORM.md` (`@C71` amended), the `@C170` entry.
- `DIAGNOSTICS.md` (O3), `CHANGELOG.md` (user-visible: `--native-rust` gone; O2's requirement),
  `CHANGELOG_TECHNICAL.md`.
- Skills: `loft-codegen` (three generators → two), `loft-debug`, `loft-optimize`, `loft-test`,
  `engineering-rigor`.
- The feature catalogue issues for `--native-rust` and the compiled stdlib.

## See also

- [inventory.md](inventory.md) — every place the writer is used, with the commands that count them.
- [rewrite-audit.md](rewrite-audit.md) — the rewrites only the writer applies, rule by rule.
- [C_BACKEND.md](../../C_BACKEND.md), [NATIVE.md](../../NATIVE.md).
