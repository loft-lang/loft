<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 00 — Baseline survey

**Status: done.**

## Summary

The introspection tool is a **packaging job**: every dump primitive
it needs already exists and is well-tested.  The REPL, in contrast,
hits **three architectural blockers** that need their own phases.

## Existing dump primitives (introspection-ready)

### Bytecode disassembly

| Entry | Location | Notes |
|-------|----------|-------|
| `state.dump_bytecode(log, config, data)` | `src/state/mod.rs:2370` | Top-level entry — iterates definitions and emits per-fn dump. |
| `compile::show_code()` | `src/compile.rs:225` | Dispatches per-fn; respects `LogConfig`'s function-filter. |
| `state.dump_code(writer, d_nr, data, annotate_slots)` | `src/state/debug.rs:846` | Per-function disassembler.  Output: bytecode offset + opcode name + const operands + source line numbers + variable annotations. |

The disassembler is exercised by every `LOFT_LOG=full` /
`LOFT_LOG=static` test case — the resulting `tests/dumps/*.txt`
files are the regression corpus.  Output format is locked in
practice; the introspection tool just plumbs it to a different
sink (stdout / per-fn file instead of `tests/dumps/`).

### Variable slot tables

| Entry | Location | Notes |
|-------|----------|-------|
| `dump_variables(writer, &function.variables, data)` | `src/variables/validate.rs:695` | Tabular dump per function: `index`, `arg-flag`, `name`, `type`, `scope`, `slot range [pos, pos+size)`, `live interval [first_def, last_use]`. |
| `Variable` struct | `src/variables/mod.rs:93-133` | Underlying record fields (`stack_pos`, `scope`, `type_def`, `first_def`, `last_use`). |

Triggered today by `LOFT_LOG=variables`; same plumbing target as
above.

### Generated Rust source

| Entry | Location | Notes |
|-------|----------|-------|
| `generation::Output::output_native(&mut f, 0, end_def)` | `src/generation/mod.rs::output_native` | Full-program native emitter — emits init() + every reachable user fn. |
| `generation::Output::output_native_reachable()` | same | Optimised emit (`--ship` mode) — only reachable fns. |
| CLI handler | `src/main.rs:1973-2080` | The `--native-emit [out.rs]` argv branch wraps the Output emitter and writes to a file. |

The generated Rust is the EXACT shape `cargo build` compiles for
`--native` mode.  Introspection wraps the same emitter but writes
to stdout (or a chosen file).

### LOFT_LOG variants

`src/log_config.rs:13-23, 146-420`.  The relevant presets:

| Variant | Effect |
|---------|--------|
| `full` | IR + bytecode + execution trace + slot annotations.  Default for tests. |
| `static` | IR + bytecode only — fastest for codegen debugging. |
| `minimal` | Execution trace for `test()` only — cleanest runtime-debug shape. |
| `variables` | Variable table per function (the slot-table dump). |
| `all_fns` | Bytecode of every function including `default/` built-ins. |
| `fn:<name>` | Filter to a single function. |
| `crash_tail:N` | Last N execution lines, flushed on panic. |
| `ref_debug` | Full + stack snapshots after every Ref/CreateStack op. |
| `bridging` | Execution + bridging-invariant warnings. |
| `scope_debug` | Scope-analysis diagnostics to stderr. |

All of these can become `--introspect=<variant>` sub-flags or `:cmd`
arguments in the REPL.

### Existing CLI

`src/main.rs:120, 125, 1236-1238, 1244-1250`:

- `--dump`: compiles + dumps bytecode to stderr via
  `state.dump_bytecode()`, exits.  Respects `LOFT_LOG` env var.
- `--native-emit [out.rs]`: emits Rust to file, exits.  Default
  output `.loft/<script>.rs`.
- `--interpret`: runs in interpreter mode (native is default).
- `--native`, `--ship`: rustc-compile + run.

The introspection tool's CLI is essentially `--dump` + `--native-emit`
+ a `--dump-slots` (new) folded into one orthogonal `--introspect`
surface, with optional sub-filters.

## REPL architectural blockers

### Blocker 1: Parser is file-level only

`src/parser/mod.rs:354-386`.  `Parser::parse(filename, default)`:

1. Opens the file via `lexer.switch(filename)`.
2. Runs `parse_file()` for **pass 1** (definitions: `parse_struct`
   / `parse_enum` / `parse_function` / `parse_typedef` /
   `parse_constant` loop).
3. If no fatal errors: `data.reset()` (clears use_names only),
   `lexer.switch(filename)` again, runs pass 2 with
   `first_pass = false` (function bodies + codegen).
4. Recursive `use` imports go through `todo_files` queue.

There is **no `parse_statement(&str)` entry**.  Adding one needs:

- A way to feed a string instead of a file (already exists —
  lexer takes any source).
- Splitting `parse_file()`'s top-level loop so a single statement
  can be parsed without re-running pass 1 over already-parsed
  defs.
- Two-pass model adapted: an interactive statement either adds a
  new top-level def (struct/fn/etc.) — needs both passes — or runs
  an expression / assignment in an existing scope — only needs
  pass 2.

### Blocker 2: Bytecode is monolithic

`src/state/mod.rs:91`.  `state.bytecode: Arc<Vec<u8>>` is built
once via `compile::byte_code()` and is immutable thereafter.
`state.code_pos: u32` (`src/state/mod.rs:64, 94`) tracks the
current position within that single block.

There's **no append API**.  Adding one requires either:

- (a) Per-statement bytecode segments managed by a registry
  (`HashMap<entry_point_d_nr, Vec<u8>>`), with `code_pos`
  becoming a `(segment_id, offset)` tuple.  Touches every
  jump / call / opcode emitter.
- (b) A growable `bytecode: Vec<u8>` (no Arc) where each new
  statement appends bytes; existing references stay valid because
  appends don't move earlier bytes.  Less invasive but loses
  `Arc` sharing across worker threads (currently used for par
  workers — they clone the Arc to share bytecode read-only).

Approach (b) is simpler if we keep the REPL single-threaded.  Par
workers in REPL mode would need separate handling.

### Blocker 3: State has no reset API

`src/state/mod.rs::execute_argv` and friends drive the runtime
end-to-end per call.  Between REPL inputs we'd want to:

- Clear `stack_pos`, `code_pos`, `def_pos`, `call_stack`.
- Preserve `database` (the user's defined values).
- Preserve `bytecode` (the per-statement segments).
- Preserve `const_refs` and `string_from_const_store` (literal
  strings interned across all statements).

There's **no public `reset_for_repl()` method**.  The closest is
`State::new(db)` (`src/state/mod.rs:165-204`) but it builds a
fresh State from scratch, losing bytecode + const-refs.

## What this baseline tells us

- **Introspection tool**: phase 01 wires existing primitives to a
  CLI surface.  No new internal APIs.  S effort.
- **REPL**: needs phases 02 (parser entry), 03 (bytecode +
  state reset), 04 (shell + execution).  M-MH effort each.

## See also

- `src/log_config.rs` — `LOFT_LOG` preset definitions.
- `src/state/debug.rs` — bytecode disassembler.
- `src/variables/validate.rs` — variable slot dumper.
- `src/generation/mod.rs` — native-Rust emitter.
- `src/parser/mod.rs` — file-level parser entry.
- `src/state/mod.rs` — runtime state + execution entry.
