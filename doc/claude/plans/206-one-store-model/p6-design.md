<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN206 P6 — the release ships C units; no library is built through rustc

**Goal.**  A library's compiled form is its hosted C unit, built and shipped by the loft release;
a library never resolves to a binary rustc built for it.

## The routes a library had to a Rust-built binary

| Route | What | P6 |
|---|---|---|
| 1 | the generated rustc artifact: the library's loft code through the rustc writer (`native_lib::build_shared_artifact`), taken under `LOFT_LIB_BACKEND=rust`, for a `[native] crate` package, and with no C compiler | **removed** |
| 2 | a `[native] crate`'s rlib, linked only into route 1 | gone with route 1 (`--native-rust` still links it, @PLN225) |
| 3 | a hand-written Rust cdylib (`[library] native`), built by cargo and dlopened for the library's `#native` functions | still loads, its library not compiled (a C unit cannot call a Rust native); removed in @PLN225 R3 |
| 4 | prebuilt Rust cdylibs: `prebuild-native.yml` produced them, `install::fetch_prebuilt` fetches one a version's `binaries` names | producer **removed**; the consumer serves route 3 |

## What ships instead

Each loft release builds every published library's unit with the loft it ships
(`scripts/release-lib-artifacts.sh`, `release.yml`) and lists them in its registry row
(`gen-toolchain-entry.py --artifacts`); `loft install` fetches the ones built by the running loft.
With no C compiler and no shipped unit, a library interprets.

## Guards

- `tests/lib_rustc_free.rs` — a stub `cargo`/`rustc` logs every call; a plain library, the old
  opt-out, a run with no C compiler, and a library declaring a `[native] crate` each answer
  right with no call, the C unit built where a compiler is.  Red on the tree before P6 (the
  opt-out, the fallback and the crate each called `rustc`).
- `tests/lib_rustc_free.rs` — a C build of a library whose native part is Rust only refuses by
  the library's name.
- `tests/native.rs` `a_library_calling_its_rust_native_part_runs_interpreted_with_it` — route 3
  as it stands: the library answers interpreted with its cdylib; `LOFT_REQUIRE_NATIVE` refuses.

## Route 3

P6's red condition, "a library resolves to a rustc-built binary anywhere", also covers route 3:
a hand-written Rust native part still loads.  It goes with the rustc writer, in @PLN225 R3
(owner, 2026-11-23): `--native-rust` and `--html` link the same Rust native interface, so one
removal takes it out of every mode.  Nothing published uses one (the five library repositories,
the consumer applications and the registry were checked), and the C writer already calls such
a release deprecated.  P6's own guard is the narrower claim it can keep now: a library's loft
code never resolves to a rustc-built binary.
