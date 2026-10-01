<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN28 — Better error messages

**Friend-readiness gate (added 2026-05-13):** phases 5
(suggestions polish) + 6 (per-site type-mismatch wording) + 7
(closeout doc) are flagged as Priority-1 in
[ROADMAP § Near-term focus](../../ROADMAP-history.md#near-term-focus--friend-readiness-added-2026-05-13).
Friends' first WTF moment is a type error; tighter messages
compound into trust.  Phase 6.1 (exhaustive `Type::name`)
shipped 2026-05-13 in commit `406cd9e3`; phase 5 partial (1-char
suggestion guard + anti-tests) shipped 2026-05-13 in commit
`a3550dd3`.

## Goal

Make every loft error reach the user as **`file:line:col — concrete
message — source line with caret — optional suggestion`**, whether it
originates in the parser, the type system, the bytecode runtime, or a
native-codegen crash.

The bar is: a user can read the message and know which token in their
own source file is wrong, what was expected, and (when possible) what
to try instead — without opening a dump file or re-running with
`LOFT_LOG`.

## Why

Today's diagnostic surface is uneven:

- **Parser errors** carry `Position { file, line, pos }` and render as
  `Error: Expect token > at foo.loft:42:17` (compact, single line, no
  source context).  See `src/lexer.rs:443::diagnostic` and
  `src/diagnostics.rs:DiagEntry::to_string_compact`.
- **Type errors raised during the second pass** (e.g.
  `src/parser/expressions.rs:215`) use the same channel, but the
  message often names a variable without saying *which type was
  expected vs. found* — the user has to grep the source for
  `tp(v_nr).is_unknown()` to find out.
- **IR-walk errors** (scope analysis, slot assignment) sometimes fall
  through to `Level::Fatal` strings without a `Position` attached
  (`add` instead of `add_at` in `src/diagnostics.rs:82`).
- **Runtime errors** are the worst case: a divide-by-zero, null
  dereference, or out-of-bounds index reaches `fill.rs` and either
  panics with no source context (`fill.rs:1677` is the only intentional
  one — every other panic is an interpreter bug or a user bug
  indistinguishable from one) or returns a sentinel that produces a
  silent wrong answer downstream.  `src/crash_report.rs` recovers the
  bytecode `pc` + op-name on SIGSEGV but **not** the originating
  loft source line — that mapping does not exist yet.
- **`Value` IR nodes carry `Value::Line(u32)` markers but not
  full positions.**  `src/data.rs:291` stores line only; column and
  file are inferred from the surrounding `Definition.position`
  (`src/data.rs:1376`).  An expression mid-block has no per-node
  span, so when a runtime opcode fails the interpreter cannot point
  at the exact sub-expression.

The cumulative effect: when I (or a user) write a loft program and
something breaks, the failure is "panic at line 1677 of fill.rs"
rather than "tried to divide by zero at game.loft:88:14 in
`damage / armour`".  Debugging then requires reading
`tests/dumps/*.txt` and reasoning backwards from the bytecode trace.

## Architectural anchor — "every error knows where it came from"

Three things must travel together to every error site:

1. **A source `Position`** (file + line + column) — the parser already
   produces these for tokens; we need to keep them attached to the IR
   nodes the parser builds and to the bytecode positions the codegen
   emits.
2. **A typed kind** (`ParseError`, `TypeError`, `RuntimeError {
   DivByZero | OutOfBounds | NullDeref | TypeMismatch | … }`) so that
   the renderer can group, suggest, and (for runtime errors) decide
   whether to abort or continue.
3. **A renderer** that reads the source file, prints the offending
   line, draws a caret under the column, and optionally appends a
   `did you mean …?` line driven by `suggest_similar` (already in
   `src/diagnostics.rs:181`).

Spans on Value IR + a pc→span side table for bytecode is the
load-bearing change.  Everything else is rendering polish on top.

## Phases

Each phase preserves every currently-green test.  Each phase ships its
own message-fixture corpus under `tests/error_messages/` (new) so that
*the rendered output itself* is a regression target — not just the
fact that the error fires.

| Phase | File | Status | Effort | Summary |
|---|---|---|---|---|
| 0 | [00-survey-and-baseline.md](00-survey-and-baseline.md) | **shipped 2026-04-28** | S | Catalogued 815 error sites across 52 files (`0a-sites.md`).  40-case `.loft` corpus + `.expect` goldens + `tests/error_messages.rs` runner landed.  `0e-rubric.md` shows 13 cases silently produce wrong output today — phase-4 (RuntimeError) closes 8, phase-6 (cascade + checks) closes 5.  Bench baseline in `0d-bench.txt`. |
| 1 | [01-spans-on-ir.md](01-spans-on-ir.md) | **done 2026-07-07** (5 leftover wraps verified unnecessary — see `01 § Resolution 2026-07-07`) | M | Attach a `Position` to every `Value` IR node that can fail at runtime via `Value::Span(Box<(Position, Value)>)` wrapper.  Wraps shipped for binary `+ - * / % << >>`, `[`, `.`, `Call` / `CallRef`, plus pc → Position table populated at codegen.  Walker discipline (every `match Value` adds a `Span(b)` recurse arm) maintained across `parser/expressions`, `parser/operators`, `parser/collections`, `scopes`, `variables/slots`, `state/codegen`, `formatter`, `state/debug`, `parser/mod::substitute_type_in_value`. |
| 2 | [02-renderer.md](02-renderer.md) | **shipped 2026-05-02** | S | `render_entry_pretty(&entry, &dyn SourceLoader, ColorMode)` prints `error: <msg>` + `--> file:line:col` + source line + caret.  `LOFT_ERRORS=pretty\|compact` env var + `--errors=...` CLI flag.  `FileSourceLoader` reads each file once; multi-byte UTF-8 + tab handling.  All 40 phase-0 baseline cases regenerated.  Compact format kept for the test harness. |
| 3 | [03-bytecode-pc-to-source-map.md](03-bytecode-pc-to-source-map.md) | **shipped 2026-05-03** | M | `State::source_spans: BTreeMap<u32, Position>` populated at codegen for every `Value::Span` walked.  Panic hook + SIGSEGV / SIGABRT handler look up the offending pc and print `at file:line:col` before the bytecode-level context.  `crash_report::set_source_spans` publishes the snapshot to a thread-local for crash printers.  Lookup is `range(..=pc).next_back()` (sparse map). |
| 4 | [04-runtime-error-kinds.md](04-runtime-error-kinds.md) | in-progress (reframed 2026-05-11 per [DESIGN_DECISIONS.md § C66](../../DESIGN_DECISIONS_FAILURE.md#c66--production-loft-programs-never-abort-on-user-attributable-edge-cases-development-may-halt)) | M | Typed `RuntimeError` for div / mod by zero, vector / text OOB, null-DbRef deref, narrow cast, `panic("…")`, stack overflow.  Mode-dependent behaviour: **production** (Logger.config.production = true) logs typed event + continues with silent sentinel; **development** halts + renders via phase-2's pretty renderer.  Three-way defense contract picks the opcode at compile time: `??` → Nullable peer (silent); `if x != null` after assignment → Nullable peer via flow-analysis; neither → raising peer.  Compile-time warning at undefended sites.  Split into 4a-4g sub-arcs below. |
| 5 | [05-suggestions.md](05-suggestions.md) | **delivered 2026-07-07** | S | Audit every `name not found` / `unknown method` / `unknown field` / `unknown type` site (parser + type-check) and wire `suggest_similar` (`src/diagnostics.rs:181`) with the right candidate set: in-scope variables, visible global fns, struct fields, methods of `T`, enum variants, type names.  Cap distance at min(2, 25 % of name length).  New `Level::Note` entries render as `= note: did you mean …?` under the parent error.  `LOFT_NO_SUGGESTIONS=1` for the harness. |
| 6 | [06-type-mismatch-detail.md](06-type-mismatch-detail.md) | **delivered 2026-07-07** | M | Rewrite type-mismatch messages to name both sides and the operation: `cannot assign vector<i32> to variable of type text` not `type mismatch`; `argument 2 of fn 'fight': expected reference<Enemy>, got integer`.  Add `Type::render_user` (loft-surface syntax) and switch errors / formatter / gendoc to it.  Pure rendering change once spans + kinds are in place; landed in 4 batches (assignment + return / calls + structs / operators + iterators / match + format). |
| 7 | [07-cleanup-and-doc.md](07-cleanup-and-doc.md) | **done 2026-07-07** | XS | New `doc/claude/COMPILER.md` § Diagnostics; retire resolved entries from `doc/claude/PROBLEMS.md` / `CAVEATS.md`; update `LOFT_LOG` quick reference in `CLAUDE.md`; user-facing `CHANGELOG.md` entry; per-phase `CHANGELOG_TECHNICAL.md` entries; dead-helper sweep; 5-case end-to-end smoke test. |

### Phase 4 sub-arcs

Phase 4 is the largest arc; broken into landable sub-phases.  Each
row's "Related" column lists P-issues (or other concrete bugs /
regressions) the sub-phase closes, opens, or otherwise touches.

| Sub-phase | Status | Effort | Summary | Related |
|---|---|---|---|---|
| 4a — Foundation + dev-mode halt | **shipped 2026-05-11** | M | `RuntimeError { kind, position, op_pc, message }` + `RuntimeErrorKind` in `src/runtime_error.rs`.  `Stores::runtime_error: Option<Box<RuntimeError>>` field for State/native boundary.  `State::raise(kind)` helper.  Dispatch loop short-circuits via `code_pos = u32::MAX` when `runtime_error.is_some()`.  `main.rs` renders captured error through phase-2's `render_entry_pretty`.  Site conversions: `n_panic` / `n_assert` (commit 21c0450c), integer `/` and `%` (commit 129d6ee5), vector / text OOB + `OpGetVectorNullable` / `OpVectorRefNullable` / `OpTextCharacterNullable` peers for loop iteration (commit 6016655e).  10 cells in `tests/runtime_errors.rs`. | **Closes (dev-mode rendering)** — 8 of phase-0's 13 silently-wrong cases (rubric in `0e-rubric.md`): `25_runtime_panic_builtin`, failed-`assert`, `17_runtime_div_by_zero_int`, `19_runtime_mod_by_zero`, `20_runtime_index_oob_vector`, `21_runtime_index_oob_text`, `22_runtime_index_negative`, `28_runtime_unwrap_none`.  **Opens (regressions to be addressed by 4c)** — `s.raise(...)` annotations broke native codegen for any program reaching a div/mod/OOB site under `--native`: re-broke @P204 (closed by @PLAN11; commit `129d6ee5` re-broke); broke `tests/codegen_emitter::p200_binary_compiles_under_native`, `p204_tail_expression_return_passes_under_native`; broke `tests/native::native_tuple_script`, `native_tuple_return_script`, `native_binary_script`.  All native consumers of touched annotations regress until 4c lands. |
| 4b — Production log-and-continue (per C66) | **shipped 2026-05-11** | S | `Logger::log_runtime_kind(&kind, position)` in `src/logger.rs` — maps each kind to severity per C66 table (UserPanic/StackOverflow → Fatal, AssertionFailed → Error, Div / OOB / null-deref / narrow-cast → Warn), formats message as `[<kind_label>] <detail>` for stable grep keys, routes through existing `Logger::log` for rate-limit + filtering + rotation.  Production branch in `State::raise`: when `database.logger.config.production == true`, log + had_fatal + return without populating `runtime_error` — execution continues, sentinel takes over.  `n_panic` / `n_assert` production paths aligned to use `log_runtime_kind` for consistent shape.  6 cells in `tests/runtime_logging.rs` (commit 16ce480b).  Production deployments now log warnings + keep running — programs (games / servers / browser embeds) never halt on user-attributable edge cases. | **Codifies** — first concrete implementation of `DESIGN_DECISIONS.md § C66`.  **Closes (production behaviour)** — same 8 silently-wrong cases as 4a, but in production mode the rendered diagnostic flows to the logger rather than halting; programs continue past the fault site with the existing sentinel.  No P-issue regression (production-branch is interpreter-only; doesn't touch native codegen). |
| 4c — Native codegen template fixup | **shipped 2026-05-11** | S | Added Stores-side counterparts of the four State helpers (`Stores::raise_runtime`, `vec_get_or_raise_runtime`, `vec_ref_or_raise_runtime`, `text_char_or_raise_runtime`) — same body, no position info (phase 4g threads positions later).  Native template rewriter in `src/generation/calls.rs` now translates `s.X(` → `stores.X_runtime(` for these four helpers.  The `_runtime` suffix is mandatory: substring-collision with the `String::replace` substitutor (`s.X(` is a substring of `stores.X(`) caused destructive accumulation (`storestorestores.raise(` after 3 passes); the suffix breaks the relationship.  Production-vs-development split mirrored on Stores side per C66.  Position info dropped on native path today; phase 4g threads codegen-time positions through. | **Closed** — @P204 (`p204_tail_expression_return_passes_under_native`, originally closed by @PLAN11, re-broken by 4a's commit 129d6ee5; re-closed here).  **Closed** — `p200_binary_compiles_under_native`, `native_tuple_script`, `native_tuple_return_script`, `native_binary_script` (4 native suite regressions opened by 4a).  **Codifies** — every per-site annotation added by 4a / 4b / 4f works in BOTH interpreter and native after 4c; future fault sites won't need a per-PR native fix. |
| 4d — Defense dispatch (`??` extension + flow-analysis) | **shipped 2026-05-11** | M | Two arms.  4d.1 — `parser/operators.rs::rewrite_outer_arith_to_nullable` extended with `GetVector` / `VectorRef` / `TextCharacter` arms, plus a recurse-one-level case for the `OpGetInt(OpGetVector(...), 0)` shape integer-vector indexing produces.  4d.2 — `parser/control.rs::rewrite_defended_fault_sites` runs at end of `parse_block`, walks the operator list looking for `Set(x, fault_op); If(test_using_x, …)` adjacent pairs, swaps the source op to its Nullable peer (mode-independent — neither log nor halt fires in production OR development because the Nullable peer never calls `s.raise`).  `value_mentions_var` walker recognises the variable in the `if` test for `if x != null`, `if x` (truthy), `if !x`, `if x > 10`, etc. — any mention counts.  3 new production-mode test cells in `tests/runtime_logging.rs` (`?? null` rescue, bare `if x != null`, bare `if x`). | **Closes** — defensive idiom log-noise concerns from chat 2026-05-11 (both `v[i] ?? null` AND bare `if x != null` after `x = v[i]` are now silent in BOTH dev and prod).  **Codifies** the three-way contract from C66's table — all three defense patterns recognised at compile time; only undefended sites trigger the runtime path. |
| 4e.1 — Format-string suppression | **shipped 2026-05-11** | S | The parser already maintains an `in_format_expr` flag while parsing `{...}` interpolations (`parser/objects.rs:773`).  Inside that scope every fault-prone op is auto-swapped to its Nullable peer via the new recursive helper `Parser::rewrite_subtree_to_nullable` in `src/parser/operators.rs` (mirrors phase 4d's outer-arith swap, but walks the whole subtree because format-string interpolations can have arbitrarily nested fault sites — `"{a + v[i] / b}"`).  Format strings are the developer's debugging surface and must NEVER halt, log, or warn (per C66 + chat 2026-05-11).  Two new test cells in `tests/runtime_logging.rs` (commit 8e74aa16). | **Codifies** the "format strings are observability, never raise" rule into a parser-driven mechanism.  **Closes** — the debugging-via-print failure mode where `println("{x}")` would otherwise halt or log on null fields.  **Pairs with** 4d — same swap table; different trigger (format-string scope vs. `??` / flow-analysis). |
| 4e.2 — Compile-time warning at undefended sites | **shipped 2026-05-11** | M | `Level::Warning` at every undefended, non-format-string fault site naming the three defense patterns (length check, `??`, `if x != null`).  Silenceable via `LOFT_NO_WARN_RUNTIME=1` env var.  Defaults ON.  Defended sites + format-string sites stay quiet at compile time AND at runtime — all paths agree.  Easy-proof skip list shipped (per 2026-05-11 workflow evaluation): (1) constant non-zero literal divisor for `OpDivInt` / `OpRemInt`; (2) constant non-negative literal index for `OpGetVector` / `OpVectorRef` / `OpTextCharacter`; (3) index is the iteration variable of an enclosing for-loop (recognised via the `Loop { Set(loop_var, Block { name "Iter …" \| contains break }); body }` shape — robust to parser-name changes via the semantic break-check fallback).  4d.1 / 4d.2 / 4e.1 swap-pass results are seen as Nullable peers in the IR (the raising form is gone there), so defended sites are implicitly skipped without a separate skip pattern.  Walker in `src/parser/operators.rs::warn_undefended_fault_sites` called from `parse_function` after `parse_code` + `vars.test_used` + `warn_upper_case_locals`.  Stdlib (`default/*.loft`) is exempt — `self.default == true` short-circuits the walker (its functions implement the very fault-handling primitives the warning is meant to nudge users toward).  In-process test harness filters the warning at `tests/testing.rs::assert_diagnostics`; binary-level coverage in `tests/runtime_warnings.rs` (11 cells: 4 `undefended_*`, 3 `skip_*`, 3 `defended_*`, 1 `silenced_by_env`). | **Adds** — first user-facing nudge for the C66 defense contract; turns the runtime-log strategy into a self-improving system (each warning closure removes both compile-time noise AND production runtime-log noise).  **Pairs with** 4d — the warning fires exactly where defense dispatch did NOT swap to the Nullable peer. |
| 4e.3 — Distinct null tokens in format-string output | **shipped 2026-05-11 (slice 1)** | M | Format-string suppression (4e.1) made every fault collapse to bare `null`, losing the *why*.  4e.3 slice 1 carries the outermost fault kind from 4e.1's swap point through to the format renderer via a runtime tag (`Stores::format_fault_tag` + `OpTagFault(kind)` op + `Stores::set_format_fault` / `take_format_fault` helpers).  4e.1's `parse_format` emits `OpTagFault(kind)` as a sibling statement BEFORE the `OpAppend*` / `OpFormat*` op when the interpolated outer expression is a fault-prone op.  The format-conversion ops (interpreter `State::format_int` / `format_stack_int`, native `ops::format_long_with_tag` via `src/generation/text.rs`) read + clear the tag — when value is `i64::MIN` AND tag is `Some(label)`, render `null(<label>)` instead of bare `null`.  Kind id mapping: `1 → /0`, `2 → %0`, `3 → oob` (covers vector OOB, vector ref OOB, text char OOB, negative index).  Genuine null values render bare `null` (no tag set).  `LOFT_FORMAT_BARE_NULL=1` env var (read once via `OnceLock`) suppresses the suffix entirely for production deployments that surface format strings to end users.  Works under both `--interpret` and `--native`.  Five new test cells in `tests/runtime_warnings.rs` (`fmt43_div_by_zero_renders_null_div`, `fmt43_mod_by_zero_renders_null_mod`, `fmt43_vec_oob_renders_null_oob`, `fmt43_genuine_null_renders_bare_null`, `fmt43_loft_format_bare_null_env_silences_suffix`).  **Slice 2 (deferred)**: distinguish negative index from OOB (today both → "oob"); extend to text/char/long/single/float format ops (today only `OpFormatInt` is wired); width-truncation polish (`{x:>5}` truncates suffix to `null(...)`); `{:j}` JSON skip; null-record-deref `null(.field)` token. | **Adds** — distinguishable fault tokens for debugging; closes the gap from 4e.1 where suppression killed the *why*.  **Pairs with** 4e.1 — same trigger (format-string scope), additional metadata threaded through via the new tag op.  **Codifies** the "format strings are observability AND debugging" rule from chat 2026-05-11 (per the user follow-up that distinguishing fault-produced null from value-null is the cheapest debugging win available given that the print is already alive). |
| 4f — Remaining site conversions | **shipped 2026-05-11 (slices 1+2: float div/mod + stack overflow)** | M | Float / single `/` and `%` by zero (was IEEE Inf / NaN), null-DbRef field / method access (was SIGSEGV in some paths), narrow-cast overflow (was wrap or sentinel), stack-overflow trap (was `panic!`).  **Slice 1 (shipped)**: float / single `/` and `%` by zero raise `RuntimeError::DivideByZero` like the integer peers; `OpDivFloatNullable` / `OpRemFloatNullable` / `OpDivSingleNullable` / `OpRemSingleNullable` peers added (silent IEEE).  4d.1 / 4d.2 / 4e.1 / 4e.2 / 4e.3 dispatch tables extended.  **Slice 2 (shipped)**: 4.12 stack-overflow trap — `State::fn_call`'s recursion-depth check now raises `RuntimeError::StackOverflow` (typed error, file:line:col, full call chain) instead of an opaque `assert!` panic.  Production mode logs + continues; dev mode halts + renders.  One new test cell in `tests/runtime_warnings.rs` (`f4f_stack_overflow_raises_typed_error`).  **Slice 3 (deferred)**: 4.9 null-DbRef field/method access (most cases already covered by the sentinel-fix from earlier work; remaining sites are fragmented edge cases — file specific reproducers as P-issues before adding raises); 4.10 narrow-cast overflow (`as u8` / `as u16` with values out of range — needs new cast-site machinery in the parser since `u8` is currently `integer limit(0,255) size(1)` which is checked at storage time, not at the explicit `as` cast).  **Recovery path for stack overflow** is intentionally NOT in this plan — recovery belongs to the async substrate (per user constraint chat 2026-05-11: library-driven, integrated with the event-loop pump, no urgency).  Design lives in [`plans/32-event-loop/README.md § Stack-overflow recovery via the async substrate (deferred)`](../32-event-loop/README.md#stack-overflow-recovery-via-the-async-substrate-deferred); the typed `StackOverflow` from this slice is its prerequisite. | **Closes (slices 1-2)** — phase-0 cases `18_runtime_div_by_zero_float` and `26_runtime_recursion_overflow` / `27_runtime_stack_overflow`.  **Slice 3 closes** the last 2 of phase-0's 13 silently-wrong cases (`23_runtime_null_deref`, `24_runtime_narrow_cast_overflow`).  **Codifies** — phase 0's "phase-4 closes 8, phase-6 closes 5" estimate becomes "phase-4 closes 11; phase-6 closes 2" after slices 1-2; "phase-4 closes 13" once slice 3 lands. |
| 4g — Backtrace + state snapshot + soft-halt + bench + docs | **shipped 2026-05-11 (slice 1: soft-halt + call-chain)** | L | Five arms.  **Slice 1 (shipped)** — 4g.1/4g.2 (function call-chain rendered after typed error) + 4g.3 (`--dev-soft-halt`).  4g.1+4g.2 slice 1: capture function chain (innermost first) into `RuntimeError.call_chain: Vec<String>` at raise time; render in main.rs as `  in fn inner() ← called from / fn outer() / …` after the typed-error block (top 5 frames + "(N more)" summary).  4g.3: `--dev-soft-halt` CLI flag (or `LOFT_DEV_SOFT_HALT=1` env) demotes dev-mode raises to log-and-continue (matches production semantics) so a single run surfaces every fault site to stderr as `soft-halt: <kind> at file:line:col`; sets `had_fatal` so the process still exits non-zero.  Three new test cells in `tests/runtime_warnings.rs` (`g4g_soft_halt_continues_past_faults`, `g4g_default_halts_on_first_fault`, `g4g_call_chain_rendered`).  **Slice 2 (deferred)**: 4g.1 source-position resolution per frame (today only function names captured; resolving the call-site source via `Data::source_at_pc` lets the renderer show `at file:42:14 in fn divide_safely` for each frame); 4g.2 named-arg value snapshot (capture `damage = [10, 20, 30] (len=3), idx = 5` from named locals at fault — needs per-variable type-aware reads via the function's variable table); 4g.4 `make bench` ≤ 3 % regression gate; 4g.5 `doc/claude/LOGGER.md § Runtime event logging` + § Production setup; phase-4 close-out. | **Closes (slice 1)** — the "fault rendered without context" failure mode the 2026-05-11 workflow evaluation flagged; developers now see WHICH function the fault fired in (and which called it), not just the file:line:col.  **Slice 2 closes** — the bench-regression acceptance gate + the named-arg-value snapshot (the highest-leverage remaining workflow win per the evaluation: "I have to add a print statement to find what `i` was").  **Codifies** — `STACKTRACE.md` shape extended to runtime-error backtraces. |
| 4h — `not null` field reminder hint | **shipped 2026-05-11 (slice 1)** | S | Per the 2026-05-11 workflow evaluation: when a struct field is read 10+ times across the codebase and never defended with `??`, the developer probably meant to mark it `not null` at construction.  Marking it eliminates ~all field-deref fault sites for that field — strictly better than defending each read site individually.  4h emits a `Level::Warning` after the second pass at the struct declaration position for each non-`not-null` field whose read count >= `HINT_NOT_NULL_THRESHOLD` (10) AND that has no defensive `?? default` site.  Hint names the field, the read count, and suggests `not null`.  Implementation: per-(struct_d_nr, attr_idx) read counter on `Parser` (`field_read_counts`) incremented in `Parser::field()` on second-pass nullable field reads; defended-set (`defended_field_reads`) populated by `handle_null_coalesce` via `last_field_read_site` (set by `field()` after each read, taken by `??` parser).  Walk + emit happens in `Parser::emit_not_null_hints()` called from `parse()` after second pass.  Stdlib (`self.default == true`) is exempt.  Silenceable via `LOFT_NO_HINT_NOT_NULL=1`.  Four new test cells in `tests/runtime_warnings.rs` (`hint_4h_high_read_count_suggests_not_null`, `hint_4h_already_not_null_quiet`, `hint_4h_defended_with_nullable_quiet`, `hint_4h_env_silences`).  Test harness filter in `tests/testing.rs::assert_diagnostics` skips `Warning: field …` lines.  **Slice 2 (deferred)**: detect `if p.field != null` flow analysis as defended (today only `??` defenses are recognised); detect defended reads in nested expressions (`(p.field + 1) ?? 0` doesn't currently mark p.field defended); per-file vs cross-file scoping; threshold tuning based on real-world usage data. | **Adds** — first proactive nudge to use the `not null` modifier instead of pervasive defensive reads.  **Pairs with** 4e.2 — 4e.2 says "defend this read"; 4h says "eliminate the need to defend by changing the constructor's contract."  **Closes** — the long-term "everyone writes `?? null` everywhere because the warning fired" failure mode the 2026-05-11 evaluation flagged. |

## Ground rules

Inherits the global plans rule from
[doc/claude/plans/README.md](../README.md):

> A plan's job is to split work into manageable chunks that each land
> cleanly without introducing new problems.

Specific to this plan:

1. **No degraded message ships.**  Every phase is a strict improvement
   over the phase-0 baseline.  If a fixture regresses (loses
   information, loses a span, gets longer-without-being-clearer), the
   phase pauses.
2. **The compact format stays available.**  `to_string_compact`
   remains the test-harness rendering — pretty rendering is opt-in via
   `--pretty-errors` (default on for `cargo run --bin loft`, default
   off for `cargo test`).  This keeps `tests/issues.rs` and friends
   stable while phase 6 churns the messages.
3. **No new error category without a fixture.**  Every new
   `RuntimeError` kind (phase 4) and every rewritten message (phase 6)
   ships with a `tests/error_messages/<name>.loft` and its
   `<name>.expect` golden output.  Goldens live under
   `tests/error_messages/` so they're easy to bulk-regen with
   `UPDATE_GOLDEN=1 cargo test error_messages` when the format
   changes intentionally.
4. **Spans on IR are additive.**  Phase 1 adds positions; it does not
   change semantics, slot assignment, codegen output, or bytecode
   layout.  If sizing `Value` enum grows the runtime hot path, store
   spans in a side table keyed by node identity instead.  Measured in
   phase 0's `make bench` baseline.
5. **Runtime panics that survive phase 4 are interpreter bugs.**
   After phase 4, any panic reaching the user means the interpreter
   itself is wrong (or a `#rust ""` annotation in the stdlib has a
   bug).  That's the contract: user code produces `RuntimeError`,
   never `panic!`.

## Risks (all addressable, none plan-blocking)

| Risk | Mitigation |
|---|---|
| `Value` enum size grows when every node carries `Position` | Side-table indexed by IR-node identity; phase 1's decision-A picks the cheaper representation after measuring on the bench corpus. |
| pc→span table inflates `.loft` startup cost | Lazy-build only when an error fires; the table is just `Vec<(u32, Position)>` per fn — measured size in phase 3. |
| Pretty rendering breaks dump comparisons | `to_string_compact` stays as the dumps' format; pretty is a separate code path gated on a flag. |
| Suggestions become noisy ("did you mean … ?" on obviously-wrong names) | Distance cap from `suggest_similar`; cap at min(2, 25 % of name length); per-site candidate sets are scoped (in-scope vars only, not all global names). |
| Source-line lookup at crash time touches the filesystem inside a signal handler | `crash_report.rs` is async-signal-safe; phase 3's enrichment lives in the *normal* panic path, not the signal handler.  The signal handler keeps printing `pc + op_name` only.  Pretty source-line resolution happens when we have a normal panic (still inside Rust's panic hook, where allocation is fine). |

## Out of scope

These are not addressed by @PLN28 even though they're tempting:

- **A REPL.**  Tracked separately under future ROADMAP work; pretty
  errors do not require a REPL and a REPL does not require pretty
  errors.
- **Stack-trace introspection from loft code.**  That's
  [STACKTRACE.md](../../STACKTRACE.md) — different design surface
  (`stack_trace()` API for user code).  Plan-07 only improves what
  the *interpreter itself* prints when something goes wrong.
- **Compile-time exhaustive type checking.**  Plan-07 improves the
  messages of the existing type checker; it does not add new checks.
- **IDE / LSP diagnostics.**  Tracked under
  [LSP.md](../../lib_plans/63-lsp/README.md).  The diagnostic types this plan introduces
  will feed the LSP later; the wire format is not in scope here.
- **Localisation.**  Messages stay English-only.

## Cross-references

- [src/diagnostics.rs](../../../../src/diagnostics.rs) — current
  `DiagEntry` / `Diagnostics` / `suggest_similar`.
- [src/lexer.rs:443](../../../../src/lexer.rs) — `Position` source +
  `diagnostic` / `pos_diagnostic` entry points.
- [src/crash_report.rs](../../../../src/crash_report.rs) — SIGSEGV /
  SIGABRT context publisher; phase 3 extends its output.
- [src/data.rs:291](../../../../src/data.rs) — `Value::Line(u32)`
  marker (replace or augment in phase 1).
- [src/fill.rs:1675](../../../../src/fill.rs) — the one intentional
  `panic!` (the `panic(text)` builtin); every other panic in
  `fill.rs` is a phase-4 candidate.
- [doc/claude/COMPILER.md](../../COMPILER.md) — diagnostic flow
  through the parser; phase 7 adds a Diagnostics section.
- [doc/claude/CAVEATS.md](../../CAVEATS.md) — entries about poor
  error messages get retired in phase 7.
- [doc/claude/plans/README.md](../README.md) — global plan ground
  rules.
