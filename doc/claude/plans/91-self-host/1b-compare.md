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
| `REFUSED <reason>` | the loft interpreter refused the program; its RUN layers (out, alloc, leak, binds) are not compared — the compile-time layers still are, on every program the front end accepts |

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
| **ir** | `loft introspect`: each function's IR, as `Value`'s display writes it | the IR the loft side read from the image, printed by a port of that display | trailing empty lines of the whole report, on both sides; nothing else |
| **types** | `introspect`'s `=== types ===` table: each variable's type with its dependencies | the same from the image's variable table | trailing empty lines, on both sides |
| **ownership** | `introspect --show-ownership`, its verdict lines: the `#` header, each function's `fn … -> verdict:` (`return_ownership`), the table, each variable's `Owned` / `Borrowed(base)` / `Join(base)` / scalar (`ownership_of`, `render_own`) | the ported oracle (1b-s3.md B-2) | the overlay lines (below) are the next layer's |
| **ownership-overlays** | the same section's `!!`, `⚠` and `delivery:` lines: `per_iteration_frees`, `text_return_orphan_risk`, `free_before_dependent_read`, `return_source_freed`, `ref_param_publish_freed`, `heap_return_delivery` | their ports, after the oracle | none |
| **resolution** | `introspect --show-resolution`: per source, the names it defines and reaches, every import alias | the loft front end (strand 1a) | none |
| **schema** | `LOFT_DUMP_TYPES=1 loft --check` (nothing run): every store type (`Stores::show_type`) — size and alignment, the derived parents, the parts: fields with positions, variants, keys | the image's schema (`Bundle.types`) in the same text, the parents derived as the Rust side derives them | the closing `ok` line `--check` prints after the dump; nothing else |
| **ctable** | the C type table the C generator writes for the program (`--emit-c`, read as `tests/selfhost.rs`'s `c_type_table` reads it) | `types::render` of the loft-derived table (P5) | the rows of the types the C describes (it writes a row only for a type the program uses); and where a row's key pointer is `0` — the C describes a type's keys only when the program's code reads them (`keys_of`) — the loft row's `keys=` part is not compared.  Keys the C does describe are |
| **slots** | `introspect`'s variable table: `slot [a, b)` per variable | the slot and width every access addresses (from F-1 the image's `stack_pos`) | none |
| **lifetimes** | `introspect`'s variable table: `live [a, b]` per variable (`compute_intervals`) | a port of `compute_intervals` over the image's IR | none |
| **out** | `loft --interpret` (`--tests FILE::ENTRY` for a test) | the runner on the emitted image, in its comparison mode (no census lines) | stdout byte for byte; the exit code; stderr byte for byte after removing the front end's diagnostics, which the tool takes from `--emit-image`'s stderr (the same front end, nothing run) and requires as an exact PREFIX of the run's stderr — PC4 measured it so; `--check` is not that source, it also prints toolchain noise.  For a test entry, the harness's block (from the `ok`/`FAIL` line of the file on) is cut from the Rust stdout, and its verdict and failure message are compared with the runner's |
| **alloc** | `LOFT_ALLOC_TRACE` from the `op-census` build (`store_census::alloc_event`: `N`, `F`, `C`, `D`, `R`, `M`) | the same lines from the `crt` shim's runtime, compiled with the trace on | the events after `M` on stores made after it, each store named by the order it first appears — exactly `c_alloc_diff.py`'s `program_events` (the Rust value stack and constant stores are setup) |
| **leak** | the stores live at exit (the alloc layer's `N` without `F`) | the same | none: the two sets, by name, must be equal (a store both sides leak is equal; one side only is a difference) |
| **binds** | `LOFT_TRACE_BIND` from `generate_set` (1b-s3.md § The comparison): one row per assignment the bytecode writer lowers, `function⇥variable⇥first\|re⇥arm`, a reassignment's row carrying B2's facts (`b2=stash\|pre\|none`, `owned_ref`, `s1`, `reads_v`, `witnessed`, `entry_w`, `buffer_w`) before its arm | the lowering's bind table, written to the same variable in the same form | the functions the loft side lowered, matched by name, each one's rows in order — the Rust writer lowers every function of the image (the stdlib's too), the loft lowering only those the run reaches; a function the loft side lowered and the Rust side did not is a difference |
| **bytecode** | `loft introspect`'s bytecode listing | the lowering's output in the same form | none, byte for byte |

Until the loft interpreter builds a layer's producer, that layer is `MISSING loft`: ir,
types and schema from C-1 (the image is already read; the displays are ported), ctable from C-1
(P5's table already exists), binds from 1b-s3.md's B-1,
slots from 1b-frame.md's F-1, lifetimes with the port of `compute_intervals`, bytecode from F-2,
ownership from B-2, resolution from strand 1a.  The compile-time reports cover the program's own
functions, as `introspect` lists them (no operator, nothing from `default/`); `--all-fns` on
both sides is a later option.  The tool is complete from the
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

## C-1 — BUILT 2026-10-30 (out, alloc, leak)

`selfhost/compare/compare.py`; the runner's `--compare` mode and entry argument; the shim's own
trace (`LOFT_CRT_ALLOC_TRACE`, `LOFT_ALLOC_TRACE_ENV` in `loft_store.h`, the C backend
unchanged) and `crt::trace_mark` for `M`; `program_events` shared with `c_alloc_diff.py`.
PC2 holds (a test entry's trace starts at its own `M`), PC4 holds with `--emit-image`'s stderr.

**First measurement (PC1, PC3):** of eight entries, two are equal in every built layer (a hash
walk, a test function); `06-structs.loft` is refused (`OpAppendCopy`); every interpreter kernel
(`structs`, `vectors`, `enums`, `hashes`, `shown`) prints the same output and leaks the same,
and makes and frees its stores differently.  The first differences fall in three groups:

- **the displaced store's free comes late** (`structs`, `vectors`, `hashes`): the Rust side
  frees it before the new store is made (`generate_set`'s B2 pre-free), the loft side after —
  1b-s3.md § Binds and frees, B-1;
- **a struct-enum variant is claimed at its own size** (`enums`, `shown`): the Rust `OpDatabase`
  claims the ENUM's size (`enum_parent_size`, `io.rs:892`), the loft `Database` the variant's —
  the same values in a different layout, an operator semantics difference to fix in B-1;
- **a returned vector is built in a new store** (`sw1`): the Rust side fills the caller's buffer
  store in place (`alloc_record_at`), the loft side makes another — B-1's `Database` in place.

None of them shows in a printed answer or in the census.

## C-1b — BUILT 2026-10-30 (ir, types, schema, ctable)

`selfhost/src/report.loft` ports the Rust reports, each beside its home: the schema dump
(`show_type`, the parents `rebuild_parents` derives), the IR (`Data::show_code` with its block,
insert and loop forms, `Definition::header`, `Type::show`, `Type::argument`, the derived `Debug`
and `Display` of a type), the types table (`emit_types`) and the C type table (P5's
`types::row`); `selfhost/compare/reports.loft KIND IMAGE` prints one.  The compile-time layers
are compared on every program the front end accepts, refused by the interpreter or not.

**Measured:** 60 corpus programs (`tests/scripts`, a seeded sample), every compile-time layer
EQUAL, after the sweep found and the port fixed: introspect's function selection (a SUBSTRING
include-list of the program's `n_` names, so methods drop out); the `Debug` of function and
iterator types; an `i64` literal at its minimum (loft's null); and in P5's `types::row`, a view
member's `view`, which the comment said the image could not give — the wrapper flag is the
type's NAME (`__nullable<…>` with a `…::Some` variant), so it is derived now.  Probe PC5 holds:
an IR literal printed `i64` for `i32` is a DIFFERENT row at the right line.  Building the
report program through the C backend found loft#2255 (a text tail's `panic` arm), fixed first.

## B-2's layer — BUILT 2026-10-30 (ownership)

`selfhost/src/own.loft` ports the oracle (`use_analysis.rs`: the `Own` lattice, `Ownership`'s
classifier and its interprocedural `return_ownership` fixpoint, `ownership_of`,
`function_defs`, and the `scopes/phase.rs` collections they read), `report.loft`'s sibling for
`introspect --show-ownership`'s verdict lines (`reports ownership IMAGE`).  An `Own` is one
integer (Owned -1, Unknown -2, `Borrowed{b}` = b, `Join{b}` = 65536 + b).  **Measured:** EQUAL
on the 13 kernels, the 60-program sample, 30 programs picked for function values, `??`, `match`,
early returns and recursion, 497 of 513 `tests/scripts` programs and 1969 of 2647 suite
programs — the rest are programs the front end refuses by design; none DIFFERENT.  Its answers
cover 453 `Unknown` and 1253 `Join` verdicts (223 `Join(base=?)`), so mutual recursion and
unresolved closure calls are exercised.

