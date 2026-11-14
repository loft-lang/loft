<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# What the loft interpreter still takes from Rust

The interpreter written in loft is the bytecode runner (`selfhost/src/fill_run.loft`,
`state.loft`, the generated `fill.loft`; [1b-fill-port.md](1b-fill-port.md)).  The rule: it
functions without any Rust code, and work on it is loft or C — never a `.rs` file.  This
page lists every place where it does not meet that rule yet, what each one costs to remove,
and the order to build the next version in.  The plan's whole path to a tree without Rust is
[README.md § The path](README.md); this page is the interpreter's part of it, by dependency.

## Contents
- [What needs no Rust](#what-needs-no-rust)
- [The dependencies](#the-dependencies)
- [Rust code written for the runner](#rust-code-written-for-the-runner)
- [The order to build in](#the-order-to-build-in)
- [How each step is checked](#how-each-step-is-checked)

## What needs no Rust

| Part | Where |
|---|---|
| the operator table: every operator's body | `#loft` templates in `default/*.loft`, written out by `selfhost/src/create.loft` |
| the machine: stack, frames, calls, generators, calls through a reference | `selfhost/src/state.loft`, `state_ops.loft` |
| the dispatch loop, `par`, the natives | `fill_run.loft`, `natives.loft` |
| rendering, files, parse tables, type facts | `show.loft`, `files.loft`, `ptypes.loft`, `types.loft` |
| the store layer and everything that reaches the system | the C runtime (`src/c/*.h`) through `selfhost/crt` |
| reading a program's source into IR | the loft front end (`lex.loft`, `syntax.loft`, `el*.loft`, `frontend.loft`) — not yet the IR the runner is fed |

The C runtime's headers live under `src/c/` beside the Rust C writer; they are C and stay.

## The dependencies

Each row is something the runner cannot do, or be built, without Rust today.

| # | What comes from Rust | Rust code behind it | What the runner uses it for |
|---|---|---|---|
| 1 | **The bytecode.**  The runner executes a run image `loft --emit-run-image` writes. | the lexer, parser, scopes and slot assignment (`src/lexer.rs`, `src/parser/`, `src/scopes/`, `src/variables/`), the IR passes (`src/compile.rs`), the bytecode writer (`src/state/codegen.rs`) | every program it runs |
| 2 | **The image's tables**: the operators' numbers and names, the natives' numbers, the type schema, the constants' stores, the fn-ref shapes, the generators' frame facts, the `par` workers, the persistence sidecars, the store-text instances, the long texts. | `src/ir_store.rs` (`save_run_image` and its `run_image_*` functions), over `Data` and `Stores` | binding operators and natives, records, constants, calls through a reference, generators, `par`, persistence |
| 3 | **The IR the loft bytecode writer reads** (`selfhost/src/bgen.loft`): a program image `loft --emit-image` writes. | the same front end as row 1, and `src/ir_store.rs::save_bundle` | the bytecode writer's input until the loft front end's own IR replaces it |
| 4 | **Building the runner.**  Its loft sources are compiled to C and linked by `loft --native-c --ship`. | the Rust front end again, the C writer (`src/c/*.rs`, `src/opt/`), the C driver (`src/c_driver.rs`, `src/c_shim.rs`) | the runner's own binary, and the `crt` shim's build |
| 5 | **The compiled standard-library bodies** the shim includes (`loft_bodies.c`). | `loft --emit-c-bodies` (`src/c/bodies.rs`) | speed only: without them every library function runs as bytecode |
| 6 | **Debug data**: the line of each bytecode position, a definition's name and file as a trace shows them, a function's variable table. | known to the bytecode writer and the front end (`State::line_numbers`, `Data`) and written nowhere the runner reads | `stack_trace()`; not carried — the programs that call it stop |
| 7 | **Reflection tables** (`type_named`, `reflect_type`). | built per program by the C writer (`src/c/reflect.rs`) or by `src/native.rs` | not carried — a program that calls `type_named` stops |
| 8 | **The oracle and the harness**: `loft --interpret` is what every answer is compared with; `tests/selfhost.rs` is a cargo test; the surveys call the `loft` binary to write images. | the Rust interpreter, cargo | checking only — the runner does not need them to run |

Rows 1, 2 and 6 are one dependency seen three times: whoever writes the bytecode knows the
tables and the debug data.  They end together when the bytecode is written in loft.

## Rust code written for the runner

Four things in `src/` exist because of the runner.  They are to be removed with the rows
above, not extended:

| Rust | Why it is there | Replaced by |
|---|---|---|
| `src/ir_store.rs`: the run image's writer, its long-text entry and its store-text instance table | the runner's input (rows 1–2) | the loft bytecode writer producing the image in memory (steps 1–2) |
| `src/parser/definitions.rs`: `#loft"…"` skipped by the Rust parser | the templates sit beside `#rust` in `default/*.loft` | nothing to do: the line goes when the Rust parser does |
| `#c_twin` and the float-bits operators in the Rust compiler | the runner's stack and code accessors | their handling in the loft front end and the loft C generator (steps 3 and 6) |
| `src/c/bodies.rs`, `src/c_shim.rs`: the compiled bodies fragment | row 5 | the loft C generator (step 6) |

## The order to build in

Each step removes dependencies and can fail on its own; none needs a `.rs` change.

| Step | What is built | Ends | Gate |
|---|---|---|---|
| **1. The bytecode writer in loft** | `selfhost/src/bgen.loft`: a port of `src/state/codegen.rs` from the stored IR to bytes.  Function by function: a function it does not write yet is counted, never written wrongly. | the writer half of row 1 | every function's bytes equal the Rust image's (`selfhost/bench/bgen_check.loft`), over the corpus |
| **2. The image in memory** | The tables of row 2 computed in loft from the IR and the schema — operator numbers by `Data::op_code`'s rule, the natives' table, constants, fn-ref shapes, generator facts, workers, sidecars, instances — and with them the debug data of row 6.  The runner takes this image; `stack_trace()` is answered from it. | rows 2 and 6 | each table equal to the Rust image's; the wide survey unchanged when the runner is fed the loft-written image |
| **3. The loft front end's own IR** | The bytecode writer is fed the IR `frontend.loft` elaborates, not `--emit-image`'s: the lexer, parser, scopes, slot assignment and IR passes complete in loft ([README.md](README.md) strand 1a). | rows 1 and 3 | the IR equal to Rust's (`selfhost/compare/`), then the bytes of step 1 equal again from that IR |
| **4. One program: source in, run** | `selfhost/front/loft.loft` runs a source file through the front end, the bytecode writer and the runner.  The older IR interpreter (`run.loft`, `lower.loft`) is removed once the corpus runs this way. | the last use of an image file | the wide survey, started from sources |
| **5. What the runner does not carry** | Reflection (row 7) from the schema in loft; a program's own `#c` function called by its symbol; the paged load by a key holding a NUL byte. | row 7 | the programs that stop or fault today agree |
| **6. The C generator in loft** | [README.md](README.md) strand 12: the runner's own sources compiled to C by loft, byte-identical to the Rust writer's C, the compiled bodies fragment included; then the driver that calls the C compiler. | rows 4 and 5 | the C equal for every program; stage 2 of the bootstrap equal to stage 1 |
| **7. The harness and the oracle** | `tests/selfhost.rs` and the surveys as loft programs; the Rust interpreter stays the oracle until the gates above have held over the corpus, then answers are pinned. | row 8 | the same counts from the loft harness |

Speed is its own line of work beside these: the runner's time against the Rust
interpreter's on the integer kernels (`selfhost/bench/fill_speed.sh`), cured in the emitted
C and in the generator of `fill.loft`, never by a Rust change.

## How each step is checked

| Check | Command |
|---|---|
| the bytecode writer, one program | `loft --interpret --lib selfhost/src --lib selfhost/crt/src selfhost/bench/bgen_check.loft <program image> <run image>` — functions identical, differing (a defect, named), not written |
| the runner over the corpus | `python3 selfhost/bench/run_image_survey.py` |
| the runner's tests | `cargo test --release --test selfhost` |
| the front end's IR against Rust's | `selfhost/compare/` ([1b-compare.md](1b-compare.md)) |
| speed | `selfhost/bench/fill_speed.sh` |
