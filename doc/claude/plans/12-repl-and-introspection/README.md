<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN12 — REPL + interpreter-introspection tool  ·  [loft-lang/plans#12](https://github.com/loft-lang/plans/issues/12)  ·  *(was `@PLAN08`)*

**Status: FINISHED (closed 2026-06-08).**  Phases 00–06 plus every follow-up
that carried weight shipped on the `repl` branch: `loft repl` (and a bare `loft`)
is an interactive prompt with line editing + history, result echo, multi-line
input, error recovery, `:`-commands, value-bearing `:vars`, auto-resume
(text-replay), and Tab completion of identifiers **and** members
(methods / struct fields / enum variants); `loft introspect` dumps a program's
intermediate forms.  The two heavy successors — the REPL.X re-run *cost* and
exact value-for-value resume — **moved to @PLN14**
([store-resident REPL session](../14-store-resident-repl-session/README.md));
the small residuals route to existing homes (see
[§ Deferred follow-ups](#deferred-follow-ups)).  This README is the **closure
record** of what shipped; the per-phase files keep the build narrative.

## Goal

Two related developer tools, sharing a planning doc because they
overlap heavily in the underlying machinery:

1. **Introspection tool** — a clean CLI surface that dumps any
   loft program's intermediate forms in a human-readable shape:
   - **Bytecode** disassembly per function.
   - **Generated Rust** source (the `--native-emit` shape).
   - **Variable slot tables** per function (name, type, slot
     offset + size, scope, live interval).

   Today these are reachable via `LOFT_LOG=…` env variants writing
   to `tests/dumps/*.txt`, scattered `--dump` / `--native-emit`
   flags, and ad-hoc setup.  The introspection tool gives users
   one CLI command that produces all three side-by-side without
   touching env vars or test harnesses.

2. **REPL** — an interactive `loft>` prompt where users type
   expressions / statements one at a time, each is parsed +
   compiled + executed, and **state persists across inputs**:
   variables stay bound, struct/enum/fn definitions remain in
   scope, and the next input can use them.  Special `:cmd`
   commands expose the introspection tool's outputs for the
   currently-defined functions.

## Why these together

The REPL needs the same machinery the introspection tool exposes:
both inspect the same intermediate forms (bytecode, generated
Rust, slot tables).  Building the introspection CLI first gives
the REPL its inspection commands (`:bytecode`, `:rust`, `:slots`)
for free — the REPL's `:cmd` handler dispatches to the same
underlying functions.

The introspection tool also ships value standalone (XS effort,
ships in days), so users get something concrete while the REPL's
deeper machinery (incremental parser, monolithic-bytecode
relaxation, State reset API) takes weeks.

## Architectural blockers (REPL only)

The introspection tool is ready to build today.  The REPL needs
three pieces of new infrastructure (each is its own phase):

| Blocker | What's missing today | Phase |
|---------|---------------------|-------|
| Parser is file-level only | `Parser::parse_file()` and `parse(filename)` work over a whole file; no `parse_statement(&str)` entry exists.  Two-pass parsing assumes the full source is available before pass 2 begins. | 02 |
| Bytecode is monolithic | `state.bytecode: Arc<Vec<u8>>` is built once via `compile::byte_code()`; no append / resume API.  `code_pos` is a position within that single block. | 03 |
| State has no reset API | Between REPL inputs, `stack_pos`, `code_pos`, `call_stack` need clearing without losing `database` (the user's defined values).  No `State::reset_for_repl()` exists. | 03 |

The introspection tool (phase 01) does NOT need any of these — it
operates on a fully-loaded program just like `--dump` does.

## Phases

| # | File | Status | Effort | Summary |
|---|------|--------|--------|---------|
| 0 | [00-baseline.md](00-baseline.md) | done | XS | Survey of existing dump APIs (`LOFT_LOG`, `--dump`, `--native-emit`, `dump_bytecode`, `dump_variables`).  Confirmed the introspection tool is a packaging job; the REPL needs three new pieces of architecture. |
| 1 | [01-introspection-cli.md](01-introspection-cli.md) | shipped | S | `loft introspect <file>` / `--introspect`: emits bytecode + generated Rust + slot tables + per-fn types to stdout (or per-flag files).  Wraps `state.dump_bytecode`, `dump_variables`, `Output::output_native`.  Sub-flags select one dimension or filter to a function.  (2026-06-07: bare `introspect` subcommand added; default-stdlib filter fixed for absolute paths + synthesized internals so all sections show user code only; `tests/introspect.rs` regression guard.) |
| 2 | [02-statement-parser.md](02-statement-parser.md) | shipped | M | `Parser::statement_incomplete` (read-more detector) + `parse_statement` → `Ready`/`NeedMore`/`Error`: top-level defs parse in place, bare expressions wrap in a synthetic fn, parse errors roll `data` back (`Data::rollback_to`). |
| 3 | [03-state-reset-and-append.md](03-state-reset-and-append.md) | shipped | S | The `Arc<Vec<u8>>`→`Vec<u8>` refactor proved unnecessary — `compile::byte_code_from` already appends. `ReplSession` (`src/repl.rs`) gives variable persistence for **any value type** (integer, text, struct, vector — verified in `tests/repl_session.rs`); error recovery fixed (fresh lexer per `parse_str`). See the doc's Revised design. The stack-resident model (run-once, no re-run of side effects) remains — REPL.X. |
| 4 | [04-repl-shell.md](04-repl-shell.md) | shipped | M | Interactive `loft>` prompt (`loft repl`, or a bare `loft`): result echo in loft's native form, multi-line input, parse-error + runtime-panic recovery, `:quit`/`:help`/`:reset`. `rustyline`/history deferred. |
| 5 | [05-repl-introspection.md](05-repl-introspection.md) | shipped | S | `:bytecode`/`:rust`/`:slots`/`:fns` wired to phase-01 introspection. `:type <expr>` + value-bearing `:vars` deferred (need a type-only parse / result capture). |
| 6 | [06-cleanup-and-doc.md](06-cleanup-and-doc.md) | shipped | XS | User docs: [REPL.md](../../REPL.md), CHANGELOG, CLAUDE.md key-commands + doc index. Deferred follow-ups recorded below. |

## Deferred follow-ups

Recorded here (not filed as GitHub issues — these are future enhancements, not
`main` regressions):

| ID | Description |
|----|-------------|
| **REPL.X** | ✅ **SHIPPED — fully typed (2026-06-08)** via **value-snapshot** (not the preserved-frame model).  A binding's RHS runs **once** at binding time; its value is captured and the binding is stored as a literal (`name = 42`), so re-running `body` on later observes is side-effect-free.  Capture builds `fn cap() -> <T> { <body> <rhs> }`, reads the return off the stack top (where `execute_at` reads its return — no new execution entry point), and dispatches on the value's exact `Type`.  **Every type:** scalars (`integer` 64-bit, `single`, `float`, `boolean`, `character`), `text`, simple enum (discriminant → `Enum.Variant`), and `DbRef`-backed heap values — struct / vector / struct-enum → `show_loft` on the returned `DbRef` (`P{a:7,b:9}`, `[10,20,30]`).  Built on the own-format serializer (steps 1–3) + parser alignment.  `ReplSession::capture_binding` + `render_capture`; tests `capture_typed_scalars` / `capture_enum_runs_once` / `capture_heap_types_run_once` / `side_effecting_binding_runs_once` / `…_text_…`.  **Remaining:** only the re-run *cost* (a long session still re-runs a literal body each observe, ~O(session²)) — that is now **@PLN14** ([store-resident REPL session](../14-store-resident-repl-session/README.md)). |
| **REPL.T** | ✅ **SHIPPED (2026-06-08)** — value-bearing `:vars` lists each bound variable with its current value in loft's native rendering.  Implemented via the **print path** (one `println("name = {name}")` per variable appended to the body, run once) rather than a result-capture API — `:vars` only needs to *display* values, so no in-process capture was required.  Reflects the latest value after a rebind; reports "no variables bound yet" when empty; realising values re-runs the body, so a side effect repeats (REPL.X).  `ReplSession::show_vars` + `bound_var_names`; tests in `tests/repl.rs`.  **Routed:** the in-process result-as-`String` return (for embedding/GUI) *does* need the capture API — **absorbed into @PLN14 sub-arc H** ([store-resident REPL session](../14-store-resident-repl-session/README.md)), where store-resident values make it nearly free. |
| **REPL.E** | ✅ **SHIPPED (2026-06-08)** — line editor + history via `rustyline` 14 (arrow-key recall, in-line editing).  Gated on an interactive TTY (`std::io::IsTerminal`): pipes, files, tests, and wasm use the unchanged plain reader (`run_piped`), so captured output stays byte-stable.  History persists to `~/.loft_history`; Ctrl-C cancels the open statement, Ctrl-D quits.  `src/repl.rs`.  **Coverage gap:** the interactive driver (`run_interactive`) is TTY-gated, so the test suite always exercises `run_piped` — the editor glue (history load/save, Ctrl-C/Ctrl-D handling, the `DefaultEditor::new()` → plain-reader fallback) runs in no CI test.  Both drivers funnel through the shared `process_line`, so the *evaluation semantics* are covered via pipes; only the editor glue is verified manually.  Closing it fully needs a pty-driven harness (e.g. `rexpect`/`portable-pty`) — deferred as low-value relative to the harness cost. |
| **REPL.W** | WASM browser playground — the parse + execute machinery already runs under wasm32; the playground is a separate web UI.  **Routed to @PLN13** ([beginner scripts + browser playground](https://github.com/loft-lang/plans/issues/13)), which owns the user-facing playground. |
| **REPL.S** | ✅ **SHIPPED (2026-06-08)** — **auto-resume via text-replay**.  Each state-changing input (def or binding) is appended to `~/.loft_session`; on the next *interactive* start the file is replayed (silently — bindings validate-only, defs emit nothing) to rebuild state, then new inputs keep appending.  Fault-tolerant (a stale/unparsable line is skipped, counted, never bricks resume), upgrade-proof (re-parsed source, no binary image to version), `--fresh` starts clean, `:reset` clears the file too.  Interactive-only — `run_piped` (pipes/tests/wasm) never touches it, so captured output stays deterministic.  `src/repl.rs` (`resume_from`/`enable_persistence`/`record_input`/`clear_session`); tests in `tests/repl_session.rs`.  **Successor:** exact value-for-value restore (non-deterministic RHS recomputes on resume — see C72) is the store-resident / mmap session image — now **@PLN14** ([store-resident REPL session](../14-store-resident-repl-session/README.md), design in [CONVERGENCE.md](../14-store-resident-repl-session/CONVERGENCE.md)). |
| **REPL.C** | ✅ **SHIPPED (2026-06-08)** — Tab completion of identifiers (global fns user + stdlib, struct/enum/base-type names, session variables) and `:`-commands, via a `rustyline` `Completer` fed from the live session (`ReplHelper`, refreshed after each input).  Operators, methods, and synthetic `<…>`-shaped defs are filtered out.  Pure matching in `complete_word` (unit-tested in `src/repl.rs`); candidate extraction in `ReplSession::completion_names` (covered in `tests/repl_session.rs`).  Interactive-only, like the rest of the line editor.  **Member completion SHIPPED (2026-06-08):** after a `.`, a value completes its **methods** (rendered `name(` so they read as callable — `"hi".` → `starts_with(`, `length(`), a struct variable also its **fields** (`p.` → `x`, `y`), and an enum type its **variants** (`Color.` → `Red`, …).  `ReplSession::completion_model` builds a `CompletionModel { names, members }`: a one-probe `infer_var_types` types all bound vars in a single rolled-back parse, then each var's `Type::name` resolves methods (`t_<len><name>_<method>` prefix scan, operator/`__` filtered) + struct fields (`Parts::Struct`); enum-type→variant from `Parts::Enum`.  Keying methods by the same `Type::name` they register under makes structs / text / vectors / base types resolve uniformly.  `complete_word` returns *only* a receiver's members after a `.`, never leaking globals.  Tests: 9 `complete_word` member cases (`src/repl.rs`) + `completion_model_resolves_members_from_schema` (`tests/repl_session.rs`).  **Residual:** a non-identifier receiver (`xs[0].`, `foo().`) and chained access (`a.b.`) need live cursor-context inference — currently yield nothing (documented, not wrong). |
| **INSP.J** | JSON output mode for `loft introspect` (machine-readable, for IDE integration).  **Routed to [`lib_plans/63-lsp/`](../../lib_plans/63-lsp/README.md)** — it is an IDE/LSP concern, tracked there as an open-work row. |

## Total effort

Single-developer estimate: **2–3 weeks** for the full sequence.  Phase
1 alone ships in **1 day** as a standalone tool — users get
introspection value before the REPL machinery starts.

Phases 2–5 are sequential (each depends on the prior).  Phase 6 is
documentation cleanup.

## Ordering rationale

**01 first.**  Standalone value, no architectural prerequisites.  The
existing dump APIs are battle-tested (every `LOFT_LOG` test case
exercises them); we're packaging them into a clean CLI surface.
Ships before REPL work begins.

**02–04 sequentially** (parser → state → shell).  Each is a hard
prerequisite for the next.  Reordering doesn't make sense: the shell
can't run without state-reset, state-reset can't be designed without
knowing how the parser feeds new statements, etc.

**05 after 04.**  REPL shell first, then layer the introspection
commands on top.

**06 last.**  Documentation and any deferred items (browser
playground, IDE integration).

## Out of scope

- **IDE integration** — language server protocol, diagnostic
  pushing, completion.  Tracked separately in `doc/claude/lib_plans/63-lsp/README.md`.
- **Debugger / breakpoints** — DAP-style stepping.  Tracked in
  `doc/claude/lib_plans/63-lsp/README.md` (LSP.3 / NDB.0).
- **Hot-reload** of `.loft` source files — the REPL reads from
  stdin only.  Editor-side hot-reload is an LSP feature, not a
  REPL feature.
- **Multi-user / networked REPL** — the prompt is single-user
  local-process only.  Browser playground (deferred to phase 6) is
  the sandboxed multi-user analogue.
- **Save / restore session** — not in this first cut; auto-resume is the
  deferred [REPL.S](#deferred-follow-ups) follow-up.  **Deterministic RNG
  continuation across resume is declined** (a saved generator state would let
  future `random()` be predicted/replayed) — see
  [DESIGN_DECISIONS.md § C72](../../DESIGN_DECISIONS.md#c72--repl-session-resume-does-not-persist-rng-generator-state);
  reproducible streams use explicit seeds.

## Cross-references

- `src/log_config.rs` — `LOFT_LOG` env-var presets; basis for
  introspection's bytecode-dump filtering.
- `src/state/debug.rs::dump_code` — the disassembler the
  introspection tool calls.
- `src/variables/validate.rs::dump_variables` — the slot-table
  dumper.
- `src/generation/Output::output_native` — generated Rust emitter.
- `src/parser/mod.rs::parse` — file-level parser, the entry that
  phase 02 splits up.
- `src/state/mod.rs::execute_argv` — runtime entry, the function
  phase 03 makes resumable.
- `doc/claude/RUNNING_TESTS.md` § LogConfig — user-facing reference for
  the LOFT_LOG presets that introspection's CLI sub-flags mirror.
- `doc/claude/DEBUG.md` — debug-output framework that the
  introspection tool re-uses.

## Ground rules

Inherits from [doc/claude/plans/README.md](../README.md):

- Every phase preserves all currently-green tests.
- No regression-now-fix-later trades.
- Each phase ships its own `make ci` run + at least one new test.

Specific to @PLN12:

1. **Introspection tool's output is byte-stable** across loft
   versions for the same input.  Tests pin the exact text shape
   so users can grep / diff outputs across releases.
2. **REPL never crashes on user input.**  Parse errors, type
   errors, runtime panics — all recover to the prompt with a
   clean diagnostic.  Crash-only behaviour is for fatal compiler
   bugs (and even those flush a useful trace before exiting).
3. **REPL output is reproducible.**  Same input sequence →
   same output (modulo time-dependent natives like `now()` which
   are flagged in their own way).  No iteration-order non-determinism.
4. **Introspection output stays valid loft IR.**  Bytecode
   disassembly + slot tables describe the EXACT shape the runtime
   sees; the generated Rust is the EXACT shape `cargo build`
   would compile.  No "approximate" or "for-display-only" forms.
