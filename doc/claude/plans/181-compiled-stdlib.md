<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 181 — The standard library compiled like any other library

Tracker: [@PLN181](https://github.com/loft-lang/plans/issues/181).

## Status

Active on `laptop-superinstructions`.  **P0 (the probe) and P1 (the artifact inside the
binary) are built.**  An interpreted program dispatches the stdlib's looping loft functions
(11 today) to their compiled bodies, built into the loft binary, with no rustc at run time.
**Next:** P2, the kernels retire.

## Goal

An interpreted program calls the standard library's COMPILED loft bodies, exactly as it
already calls a `use`d library's (C71, "libraries compile, scripts interpret"), without
rustc on the user's machine.  Then the hand-written Rust kernels
([KERNELS.md](../KERNELS.md)) are replaced by their loft bodies, as the owner directed: every
kernel is a stop-gap, and a Rust routine is replaced by loft as soon as that is viable.

## Why

A kernel fixes one name with hand-written Rust.  Loft compiles through rustc, so its own code
can be as fast; the only gap is the interpreter, and C71 already closes that gap for every
library except the standard library.  Measured in P0: `split`'s own loft body, compiled and
called from the interpreter, runs within 1.4× of the `split` kernel (the retirement bar is 2×);
interpreted, it is 21× slower.

## What already exists (the reuse)

The whole dispatch path, unchanged:

- `native_gate::shared_store_dispatchable` — which functions can cross (text parameters,
  vector and record results; not generics, closures or concurrency).
- `native_lib::cached_or_build_shared_cdylib` / `generate_shared_cdylib_lib_rs` — the
  artifact: the native backend's own output for those functions plus one shared-store bridge
  each (`loft_shared_<fn>`, the `LibArg` ABI).
- `native_lib::probe_and_mark_exports` sets `def.native`, so the calls compile to
  `OpStaticCall`; `extensions::wire_shared_native_fns` resolves each bridge and
  `shared_store_dispatch` calls it, sharing the interpreter's heap by pointer (no copying).

## What differs for the standard library

1. **No rustc at run time.**  `--interpret` never runs one, and a user may have no toolchain.
   The artifact is generated with loft itself, once per loft version, and built INTO the loft
   binary: the bridge symbols come from an in-binary table rather than `dlsym`.  That also
   serves the browser interpreter, which cannot `dlopen`.
2. **One artifact for every program.**  A library artifact is keyed on the caller's WHOLE
   type table (`type_layout_fingerprint`), because it hard-codes type indices, so every
   program with its own types gets its own build.  The standard library's types register
   before any program's (P0, below), so the check becomes the stdlib's type PREFIX, verified at
   start-up; on a mismatch the program runs the loft bodies interpreted — slower, never wrong.
3. **Generics** are monomorphised per program and cannot be precompiled.  The compiled set is
   the non-generic, shared-store-dispatchable loft functions; `sum<T>` stays interpreted (its
   `vector_sum_int` kernel retires with the code generator's reduction instead).

## Phases

| phase | what | validated by |
|---|---|---|
| **P0** probe — DONE | the existing path pointed at the stdlib, behind a probe-only switch | same answers, strict stores clean, the speed, the type prefix across programs |
| **P1** in-binary artifact — BUILT | generate the stdlib's compiled source with loft (a derived file, drift-guarded like `src/fill.rs`), build it into the binary, an in-binary bridge table, the start-up prefix check and its fall-back | every stdlib guard on both backends with the compiled set on and off (`LOFT_NO_NATIVE_LIBS` A/B); a planted prefix mismatch interprets, byte-identical |
| **P2** kernels retire | `split` and `lines` back to their loft bodies; `make kernel-ratio` and the kernel guards decide | the ratio at or under 2× from the interpreter; the guards' oracles |

## P0 — measured

The probe was a temporary block in `src/main.rs` (removed): `LOFT_PROBE_COMPILED_STDLIB=<fns>`
handed the named stdlib functions to `cached_or_build_shared_cdylib`, `probe_and_mark_exports`
and the existing load-and-wire.  Nothing else changed.

| probe | interpreted loft body | compiled loft body, called from the interpreter | kernel |
|---|--:|--:|--:|
| `split_text` (60,001 parts, ×5) | 299 ms | 23.9 ms | — |
| `split`'s loft body (60,001 parts, best of 5) | 48.8 ms | 3.3 ms | 2.3 ms |

Same answers in every cell (`300005 f1 b 0`; the `split` cells asserted equal to the kernel,
edges included), no leak and no use-after-free under `LOFT_STRICT_STORES=1`, and the second run
reused the cached artifact.

**The type prefix.**  Seven programs' type tables were dumped: an empty `main`, one with
structs, an enum and a keyed collection, one with generic collections and `files()`, one with
JSON, one of the leak probes, one `use`ing `regex` and one `use`ing `cbor` (which declares 16
types of its own).  In every one the standard library's 91 types are an identical prefix; the
program's and the libraries' types follow them.  That is what makes one artifact possible —
and seven programs are evidence, not proof, which is why P1 checks the prefix at start-up.

## P1 — built

- `src/compiled_stdlib.rs`: `export_set` (dispatchable, a loft body, and the body LOOPS — a
  one-operation body gains nothing from the bridge, and output and file primitives stay out),
  `generate` (the library artifact adapted to the crate: an alias for `extern crate loft`, no
  exported symbols, a name → bridge table, the type prefix and the stdlib source hash it was
  compiled from), `mark` (an interpreted program only; declines on a prefix or source mismatch)
  and `bridge` (the lookup `extensions::wire_shared_native_fns` tries before `dlsym`).
- `src/compiled_stdlib_gen.rs`, `@generated` by `make compiled-stdlib`, kept current by
  `tests/compiled_stdlib.rs::compiled_stdlib_up_to_date` and exempt from `rustfmt` so it stays
  the generator's bytes.  Regenerate after a stdlib change or a code-generator change that alters
  what these functions emit.
- `LOFT_NO_COMPILED_STDLIB=1` is the A/B; its home is PERFORMANCE.md beside
  `LOFT_NO_NATIVE_LIBS`.

Validated: all 1,929 `tests/scripts` programs answer identically interpreted with it on and off
(stdout and exit status; 30 of them call a compiled function).  The harness can fail: a planted
defect in the compiled `split_text` turned two of them red and failed the drift test.  An edited
stdlib and a wrong type prefix each declined all 11 and ran correctly; a warm program-cache
start replays the marks; `mark` costs about 0.3 ms at start-up (within noise of a `hello`).

Not yet covered, each simply interpreting today: `loft test`'s in-process runner, the REPL, the
debugger, a host embedding and the browser (which has no bridge dispatcher).

## Open questions

- **Regeneration churn.**  The generated file follows the code generator, so a codegen change
  that alters these functions' emitted Rust asks for `make compiled-stdlib` in the same commit
  (the drift test says so).  If that proves frequent, generating it in the build is the fix.

## See also

- [KERNELS.md](../KERNELS.md) — the kernels this retires, and the bar.
- [C71](../DESIGN_DECISIONS_PLATFORM.md#c71--native-libraries-compile-scripts-interpret--the-steady-state-execution-model) — libraries compile, scripts interpret.
- [180-value-records-in-ir.md](180-value-records-in-ir.md) — interpreter speed; [179-scripts-in-loft/](179-scripts-in-loft/README.md) — the script ports that measure it.
