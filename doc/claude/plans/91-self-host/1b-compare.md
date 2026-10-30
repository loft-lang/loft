<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN91 strand 1b — the strict comparison: one program, every output, both implementations

A child document of [1b-interpreter.md](1b-interpreter.md).  Status: **design, nothing built.**

## Why (owner, 2026-10-30)

*"Start with a strict comparison framework that validates all the outputs.  It should not
integrate with the normal tests but can be used for every individual test to validate equal
implementations."*  Goal constraint 5 makes the loft interpreter a reimplementation of the Rust
one; this tool is how "equal" is checked, one program or one test function at a time, on every
output either side can produce — not only the printed answer the census compares.

## What it is

`selfhost/compare/compare.py FILE[::ENTRY] …` — a standalone script, like the census and
`scripts/c_alloc_diff.py`: no `tests/` hook, not in `make ci`, run by hand on any program, test
script or suite cell.  `FILE` runs `main`; `FILE::test_name` runs that one test function, as
`loft --tests FILE::test_name` runs it.  For each entry it reports one line per LAYER:

| verdict | meaning |
|---|---|
| `EQUAL` | both sides produced the layer and it is the same, under the layer's stated normalisation only |
| `DIFFERENT` | the first differing row, with three rows of context from each side |
| `MISSING <side>` | that side does not produce the layer yet (the loft interpreter before the step that builds it) |
| `REFUSED <reason>` | the loft interpreter refused the program; no layer is compared |

**Strict:** the exit code is 0 only when every selected layer of every entry is `EQUAL`.
`MISSING` is a failure, not a skip; `--layers out,alloc` selects layers for a run that wants
fewer, and the selection is printed with the verdict.  No normalisation exists but the ones
written in the table below; a new one is a design change here first.

## The layers

Every output either implementation produces, at compile time and at run time (owner: *"it also
includes all compile time reports: slots, IR, bytecode, lifetimes"*; *"also include the type
information"*).  The compile-time reports
are `loft introspect`'s sections, per function; the loft side writes the same sections in the
same text, so the comparison is of text, and a report the loft side reads or derives
differently shows as the row where it does.

| layer | Rust side | loft side | normalisation (all of it) |
|---|---|---|---|
| **ir** | `loft introspect`: each function's IR, as `Value`'s display writes it | the IR the loft side read from the image, printed by a port of that display | none, per function, byte for byte |
| **types** | `introspect`'s `=== types ===` table: each variable's type with its dependencies | the same from the image's variable table | none |
| **schema** | `LOFT_DUMP_TYPES`: every store type (`Stores::show_type`) — size and alignment, the derived parents, the parts: fields with positions, variants, keys | the image's schema (`Bundle.types`) in the same text, the parents derived as the Rust side derives them | none |
| **ctable** | the C type table the C generator writes for the program (`--emit-c`, read by `tests/selfhost.rs`'s `c_type_table`) | `types::render` of the loft-derived table (P5) | none, row by row |
| **slots** | `introspect`'s variable table: `slot [a, b)` per variable | the slot and width every access addresses (from F-1 the image's `stack_pos`) | none |
| **lifetimes** | `introspect`'s variable table: `live [a, b]` per variable (`compute_intervals`) | a port of `compute_intervals` over the image's IR | none |
| **out** | `loft --interpret` (`--tests FILE::ENTRY` for a test) | the runner on the emitted image, in its comparison mode (no census lines) | stdout byte for byte; the exit code; stderr byte for byte after removing the front end's diagnostics, which the tool takes from `--emit-image`'s stderr (the same front end, nothing run) and requires as an exact PREFIX of the run's stderr — PC4 measured it so; `--check` is not that source, it also prints toolchain noise.  For a test entry, the harness's block (from the `ok`/`FAIL` line of the file on) is cut from the Rust stdout, and its verdict and failure message are compared with the runner's |
| **alloc** | `LOFT_ALLOC_TRACE` from the `op-census` build (`store_census::alloc_event`: `N`, `F`, `C`, `D`, `R`, `M`) | the same lines from the `crt` shim's runtime, compiled with the trace on | the events after `M` on stores made after it, each store named by the order it first appears — exactly `c_alloc_diff.py`'s `program_events` (the Rust value stack and constant stores are setup) |
| **leak** | the stores live at exit (the alloc layer's `N` without `F`) | the same | none: the two sets, by name, must be equal (a store both sides leak is equal; one side only is a difference) |
| **binds** | `LOFT_TRACE_BIND` from `generate_set` (1b-s3.md § The comparison) | the lowering's bind table | none: function, variable, first/re, arm, B2's free, every fact and its value |
| **bytecode** | `loft introspect`'s bytecode listing | the lowering's output in the same form | none, byte for byte |

Until the loft interpreter builds a layer's producer, that layer is `MISSING loft`: ir,
types and schema from C-1 (the image is already read; the displays are ported), ctable from C-1
(P5's table already exists), binds from 1b-s3.md's B-1,
slots from 1b-frame.md's F-1, lifetimes with the port of `compute_intervals`, bytecode from F-2.  The tool is complete from the
start; the interpreter fills it in.

## What each side needs (the build)

- **C-1 — the tool, with out, alloc, leak, ir and types.**
  - The Rust side runs the `op-census` build (`cargo build --release --features op-census
    --target-dir target/op-census`, which `c_alloc_diff.py` already uses) for everything — the
    image, the check and the run — so both sides see one front end.
  - The runner (`selfhost/cover/run_program.loft`) takes the entry to run (`run_program IMAGE
    [ENTRY]`, `n_main` by default), writes `M` before it starts (a `crt::trace_mark` over the
    runtime's `LOFT_TRACE("M")`), and exits with what the Rust interpreter exits with for a
    failed assertion or a panic.
  - The shim's runtime writes the alloc trace under its OWN variable, `LOFT_CRT_ALLOC_TRACE`, so
    the interpreter's own stores (the runner's runtime copy) never mix into it: `loft_store.h`
    reads the variable's name from `LOFT_ALLOC_TRACE_ENV` (default `"LOFT_ALLOC_TRACE"`,
    values and the C backend unchanged), and `shim/crt.c` defines both before including the
    runtime.
  - The runner is built once per tool run into `target/selfhost-compare/`, rebuilt when a
    `selfhost/` source is newer.
  - The runner's `--dump-ir` and `--dump-types` print the image's IR and variable types in
    `introspect`'s text: a loft port of the IR display (`Value`'s `Display`, `src/data.rs`) and
    of the types table, in `selfhost/src/show_ir.loft`.
- **C-2 — binds** (with 1b-s3.md's B-0/B-1): `LOFT_TRACE_BIND` in `generate_set`, the lowering's
  table, `--dump-binds` on the runner.
- **C-3 — slots, lifetimes and bytecode** (with F-1, the `compute_intervals` port, F-2): the
  runner's `--dump-slots`, `--dump-lifetimes` and `--dump-code`, in the forms `introspect`
  prints.

## Probes before building (each can fail on its own)

- **PC1** — the tool can fail: one program whose stdout differs (a format the interpreter gets
  wrong on purpose under a probe switch), one whose alloc events differ (a bind lowered as a
  copy where the Rust side adopts), each reported as `DIFFERENT` at the right row.
- **PC2** — the `M` marker on the test path: `loft --tests FILE::test_x` writes `M` before the
  test function runs, or the alloc layer of a test entry compares the test runner's setup.
- **PC3** — the alloc layer's first measurement: three programs the census passes today
  (`tests/scripts/06-structs.loft`, a hash kernel, a vector kernel); the expectation is
  `DIFFERENT` under today's run-time ownership (the owner table makes fresh stores where the
  Rust side remakes in place), and the rows say where.
- **PC5** — the ir layer can fail: an image read with one node kind decoded wrong (a probe
  switch in the reader) shows as `DIFFERENT` at that node's function and line.
- **PC4** — the diagnostics prefix: a program with a warning, whose Rust stderr starts with
  exactly `loft --check`'s output.
