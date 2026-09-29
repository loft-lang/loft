<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 4 — Typed runtime errors

**REFRAMED 2026-05-11 — see [DESIGN_DECISIONS.md § C66](../../DESIGN_DECISIONS_FAILURE.md#c66--production-loft-programs-never-abort-on-user-attributable-edge-cases-development-may-halt).**
The original phase-4 spec called for HALTING the loft program on
any typed runtime fault (`State::raise` short-circuiting the
dispatch loop via `code_pos = u32::MAX`).  This violates loft's
fundamental "programs must not abort on user-attributable edge
cases" rule (C66) — loft's primary deployment is interactive
games / scripted scenes / multiplayer servers where halt is
strictly worse than a wrong-pixel edge case.

The rendering half (typed `RuntimeErrorKind` variants +
`--> file:line:col` + caret + source-line gutter via phase-2's
`render_entry_pretty`) is **kept** — it's exactly the right
diagnostic shape.  What changes is the halt half: `State::raise`
becomes "log + continue with sentinel" instead of "short-circuit
the dispatch loop."

After the reframe lands the model is:

1. **Per-site detection** continues to fire — `vec_get_or_raise`,
   `vec_ref_or_raise`, `text_char_or_raise`, the integer div/mod
   `if v2 == 0` annotation guard, `n_panic`/`n_assert` — they all
   still recognise the fault.
2. **`State::raise(kind)` logs + continues**: writes a
   `RuntimeError`-shaped entry to `Stores::logger` (when attached),
   stores the entry in `Stores::runtime_error` for the renderer at
   exit, sets `had_fatal = true`, and **returns**.  The calling op
   completes normally and produces its sentinel (null DbRef, char 0,
   `i64::MIN`, etc.).  User code keeps running; defensive idioms
   like `if v[i] != null { use(v[i]); }` and `x = a / b ?? 0` keep
   working because the sentinel is what the user already expects.
3. **Dispatch loop** drops the `runtime_error.is_some()` check —
   no more short-circuit via `code_pos = u32::MAX`.  Execution
   continues past every fault site.
4. **`main.rs`** renders the LAST captured `runtime_error` at exit
   for CLI / test feedback.  In a long-running game the renderer
   fires only at shutdown; the operator-visible signal during the
   run is the logger entry.
5. **Loop-iteration peers** (`OpGetVectorNullable`,
   `OpVectorRefNullable`, `OpTextCharacterNullable`) stay — they
   skip the diagnostic emission entirely so the hot loop-iter
   path doesn't pay for it (end-of-iteration is expected
   behaviour, not a fault).  The user-facing peers
   (`OpGetVector` / `OpVectorRef` / `OpTextCharacter`) log + return
   the same sentinel.
6. **Optional `--strict-runtime` / `LOFT_STRICT=1`** for
   development: restores halt-on-fault as a debug-mode opt-in for
   catching bugs.  Default and production are always log + continue.

Per-site rename in the next iteration: `vec_get_or_raise` →
`vec_get_logged`, `vec_ref_or_raise` → `vec_ref_logged`,
`text_char_or_raise` → `text_char_logged`, `State::raise` →
`State::log_runtime_event` to make the contract obvious.

---

## Status (2026-05-11 reframe)

Phase 4 ships the typed `RuntimeError` infrastructure + per-site
detection in BOTH modes; the **behaviour at the fault site is
mode-dependent** (production logs + continues, development halts +
renders).  See [DESIGN_DECISIONS.md § C66](../../DESIGN_DECISIONS_FAILURE.md#c66--production-loft-programs-never-abort-on-user-attributable-edge-cases-development-may-halt)
for the rule.

### Shipped (2026-05-11) — dev-mode halt path

- Foundation: `RuntimeError { kind, position, op_pc, message }` +
  `RuntimeErrorKind` in `src/runtime_error.rs`;
  `Stores::runtime_error: Option<Box<RuntimeError>>` field;
  `State::raise(kind)` helper; dispatch-loop short-circuit in
  `State::execute_argv` and `State::resume`; renderer integration
  in `main.rs` via `render_entry_pretty`.
- Site conversions (steps 4.3/4.4/4.6/4.7/4.8/4.11/4.13):
  - `n_panic` / `n_assert` (production check already in place;
    dev-mode now routes through typed error)
  - `OpDivInt` / `OpRemInt` (annotation guard `if @v2 == 0 { s.raise(...); 0_i64 } else { ops::op_div_int(...) }`)
  - `OpGetVector` / `OpVectorRef` / `OpTextCharacter` (raising
    annotations route through `s.vec_get_or_raise` /
    `s.vec_ref_or_raise` / `s.text_char_or_raise`)
- Loop-iteration peers: new opcodes `OpGetVectorNullable` /
  `OpVectorRefNullable` / `OpTextCharacterNullable` keep the
  legacy "return null on OOB" behaviour for the loop driver's
  end-of-iteration check (`parser/collections.rs:1492-1499`).
  Six parser emit sites rerouted; user-facing `v[i]` / `s[i]`
  keep emitting the raising peers.
- In-process test harness (`tests/testing.rs`) re-raises
  `runtime_error` as a Rust panic so `#[should_panic]` fixtures
  keep firing.
- Pre-existing tests that relied on silent `i64::MIN`
  propagation (`expr_zero_divide`,
  `inc29_bang_integer_null_is_caught`) converted to use the
  explicit `?? null` form.

End-to-end dev-mode output:
```
error: panic: boom!
  --> /tmp/p_panic.loft:2:1
  |
2 |   panic("boom!");
  | ^
```

### Open — production log-and-continue path + defense dispatch + warning (next slice)

Sub-arc tracked in `~/.claude/plans/async-toasting-nest.md`
(10-commit landing sequence).  Headline pieces:

1. `Logger::log_runtime_kind(&mut self, kind, position)` in
   `src/logger.rs` — formats via the rendered diagnostic shape,
   routes through `self.log(severity, file, line, msg)`, applies
   rate-limit keyed by `(file, line, kind.label())`.
2. Production branch in `State::raise`: when
   `database.logger.config.production == true`, log via
   `log_runtime_kind` + `had_fatal` + return without populating
   `runtime_error`.  Dev path unchanged.
3. Startup check in `main.rs` (and any embedding entry point):
   refuse to boot when production mode is requested without a
   logger attached, with a clear actionable error message naming
   the fix path (see C66 + LOGGER.md).
4. `tests/runtime_logging.rs` — production-mode test harness
   attaches a capturing Logger, asserts continue-and-log shape
   for each kind.
5. `n_panic` / `n_assert` cosmetic alignment to `log_runtime_kind`
   so all production-mode log entries share the rendered shape.
6. **Defense dispatch — `??`**: extend
   `parser/operators.rs::rewrite_outer_arith_to_nullable` to also
   handle OpGetVector / OpVectorRef / OpTextCharacter when followed
   by `??`.  `x = v[i] ?? null` then routes through the Nullable
   peer (no log / no halt) — matches today's arithmetic-only
   behaviour.
7. **Defense dispatch — `if x != null` flow-analysis**: in
   `parse_assign`, when the source is a fault-prone op AND the
   immediately-following sibling is `Value::If(Var(x) != null, ...)`
   (or `!Var(x)`), swap the source to its Nullable peer.  Single-
   block scope (next sibling only).  Cross-function defenses fall
   through to the raising peer + log path.
8. **Compile-time warning** at every undefended fault site with
   `note:` lines naming the three defense patterns.  Silenceable
   via `LOFT_NO_WARN_RUNTIME=1` / `--no-warn-runtime`.  Defaults
   ON.  Defended sites stay quiet at compile-time AND at runtime
   — the two paths agree.  Skip-list logic walks the IR after
   `rewrite_defended_fault_sites` (4d.2) and
   `rewrite_subtree_to_nullable` (4e.1) so the warning fires
   exactly where neither swap fired.  (Tracked as **4e.2** in
   the README sub-phase table.)
9. **Distinct null tokens in format-string output** — 4e.1
   suppression collapses every fault to bare `null` (or empty
   for text/char), losing the *why*.  4e.3 carries the fault
   kind from the swapped Nullable peer through to the format
   renderer so each fault-produced sentinel renders with a
   `(reason)` suffix in format strings only.  Examples:
   `null(/0)` (divide-by-zero), `null(oob 999/3)` (vector OOB
   with index/length), `null(neg -5)` (negative index),
   `null(.field)` (chain stops at null record).  Genuine null
   values stay as bare `null`.  Text/char nulls render as `null`
   (never empty — empty looks like a missing variable, not a
   fault).  **Out-of-scope contexts**: `{:j}` JSON format keeps
   `null` (must produce valid JSON); explicit `?? "fallback"` in
   a format string respects the fallback.  **Width handling**:
   when the format spec has a hard width (`{x:>5}`) the suffix
   truncates to `null(...)` so columns stay stable.
   `LOFT_FORMAT_BARE_NULL=1` env var suppresses the suffix for
   production deployments that surface format strings to end
   users.  Implementation: side table keyed by IR-node identity
   carrying `RuntimeErrorKind` from the swap point through to
   the renderer; no `Value` enum bloat.  (Tracked as **4e.3**
   in the README sub-phase table; queued AFTER 4e.2.)
10. `doc/claude/LOGGER.md` § Runtime event logging + § Production
    setup; document the three-way defense contract + warning.
11. Status block update.

### Severity per kind (production mode)

| Kind | Severity | Rationale |
|---|---|---|
| `UserPanic` | `Fatal` | Already Fatal in n_panic production path; intentional terminal call |
| `AssertionFailed` | `Error` | Already Error in n_assert production path; intentional invariant violation |
| `DivideByZero` | `Warning` | Recoverable via `??`; sentinel takes over silently |
| `IndexOutOfBounds` | `Warning` | Recoverable via defensive `if x != null`; sentinel takes over |
| `NegativeIndex` | `Warning` | Same as IOB |
| `NullDereference` | `Warning` | Same; sentinel propagates; `if x != null` rescues |
| `NarrowCastOverflow` | `Warning` | Clamped value or sentinel; rare |
| `StackOverflow` | `Fatal` | System-level; not recoverable; logged then host's frame loop decides whether to continue or restart |

### Easy-proof skip list — REQUIRED for 4e.2 (not optional)

Per the 2026-05-11 workflow evaluation, the 4e.2 warning is
worse than nothing if it fires at every `v[i]` in code that
the compiler could trivially prove safe — developers will
silence it within a session and the safety net evaporates.
4e.2 MUST recognise the four canonical safe patterns BEFORE
landing.

| Pattern | Skip when | Sketch |
|---|---|---|
| Bound loop variable | `for i in 0..len(v) { v[i] }` — `i` is bound to the exact `0..len(v)` range from the enclosing `Iter` | Track `var → CollectionLen(c)` on entry to `Value::Iter(var, init, …)` when `init` matches the `OpRange(0, OpLengthVector(c))` shape; clear on exit.  At fault-site walk, if the index var matches and the indexed collection is `c`, skip. |
| Bound by `if i < len(v)` | The fault site is inside an `if` whose test is `Var(i) < OpLengthVector(c)` (or `<=` against `len-1`, or `i >= 0 && i < len(v)`); the indexed collection is `c` | Track `var → CollectionLen(c)` on entering the `then` branch of `Value::If`; clear on `else` and on exit.  Same lookup as the loop case. |
| Constant-literal divisor | `Value::Int(n)` with `n != 0` (or `n.abs() > 0`) directly as the second operand of `OpDivInt` / `OpRemInt` | Pure pattern match — no context table needed. |
| Constant-literal index | `Value::Int(n)` with `n >= 0` AND we can prove `n < known_len(c)` (only for vector literals constructed in the same block) | Track `var → KnownLen(N)` on `Set(var, OpVectorLiteral(N elements))`; same thing for `for` comprehensions with a constant range bound.  At fault-site walk, if index is a literal in `[0, N)` of the indexed `var`'s known length, skip. |

The skip list grows as we observe real-world false-positive
patterns; **each new skip case ships with a regression test
in `tests/parse_errors.rs::warning_skip_<pattern>`**.  The bar
for adding a skip case is low ("would a competent loft
programmer reasonably expect this to be safe?"); the bar for
removing one is high (silently changes from no-warning to
warning, breaks downstream test goldens).

Cases NOT covered by 4e.2 (intentionally — fall through to the
runtime path + log entry):

- Cross-function defenses: `helper_that_checks(x)` after `x =
  v[i]`.  4e.2 has no inter-procedural analysis.
- Indexed by a variable bound by an unrelated computation:
  `j = compute(); v[j]`.  No way to prove `j < len(v)` without
  CSP / range analysis.
- Indexed by `find` / `position`: today these return either
  `null` or an in-range index, but 4e.2's pattern matcher
  doesn't yet recognise `Value::Set(j, OpFind(c, …))` as a
  safe-by-source pattern.  Add as a skip case when the cost
  of doing so is paid by a real false-positive complaint —
  not pre-emptively.

### Remaining site conversions (steps 4.5 / 4.9 / 4.10 / 4.12)

Inherit the same production-vs-dev shape via `State::raise`.
Each adds a per-site fault check (e.g., null-DbRef field read,
narrow-cast overflow check) that calls `s.raise(KIND)` and
returns the existing sentinel.  ALL must also be wired through:

- The 4d.1 / 4d.2 / 4e.1 swap tables — adding a new fault
  kind WITHOUT a Nullable peer means `??` / `if x != null` /
  format-string suppression silently fail to defend the new
  site.  Each new fault kind ships with its Nullable peer in
  the same commit.
- The 4e.2 warning's per-kind diagnostic message.

Sites:

- 4.5 — float / single div by zero (sentinel: `f64::NAN` or
  IEEE behaviour kept on nullable)
- 4.9 — null DbRef field / method access (sentinel: null-
  propagating null)
- 4.10 — narrow-cast overflow (sentinel: clamped or null)
- 4.12 — stack-overflow trap (no sentinel; production logs and
  host frame loop handles continuation)

### Renderer / backtrace polish (steps 4.14, 4.15, 4.16, 4.17, 4.18)

- 4.14 — capture `State::call_stack` into `RuntimeError.backtrace`
  at `raise` time, resolve each frame's source via phase-3's
  `Data::source_at_pc`.
- 4.15 — render the backtrace through the pretty renderer (top-3
  frames + "(use `LOFT_BT=full` for more)" if more).  Same shape
  for both dev-mode rendering and production-mode log entries.
- 4.16 — bench delta ≤ 3 % vs phase 3 baseline; outline the
  fault-check branches with `#[cold]` if hot.
- **4.17 — State snapshot at fault site.**  Per the 2026-05-11
  workflow evaluation: today the dev-mode halt tells the
  developer *where* `v[i]` was OOB but not *what* `v` and
  `i` were at that moment.  The pieces are already on the
  bytecode stack at fault time; surface them.  Capture the
  named-arg values at `raise` time into a
  `RuntimeError.snapshot: Vec<(String, ValueRepr)>` field
  using the same `Display` formatter the format-string
  renderer uses (so the snapshot prints exactly as the
  developer would see them in `println("{x}")`).  Keep the
  snapshot to direct argument values + the indexed
  collection's `len`; do NOT walk arbitrary heap structures
  (cost + cycles).  Renders inline with the diagnostic:

  ```
  error: index 5 out of bounds for length 3
    --> game.loft:42:14
     |
  42 |     dmg = damage[idx];
     |                 ^
     = state: damage = [10, 20, 30] (len=3), idx = 5
  ```

  Production-mode log entries get the same line.  Closes the
  "I have to add a print statement to find what i was"
  failure mode.
- **4.18 — `--dev-soft-halt` flag** (and `LOFT_DEV_SOFT_HALT=1`
  env var equivalent).  Per the 2026-05-11 evaluation: dev-mode
  halts at the first fault, which is right for tight diagnose-
  fix loops but wrong for "show me the full pattern of breakage
  in this newly-ported file."  The flag demotes development-mode
  raises to log-and-continue (matches production semantics) so
  the developer sees every fault site in a single run.  No new
  machinery needed — the production branch in `State::raise`
  already implements log-and-continue; the flag just toggles
  which branch fires regardless of the logger's `production`
  flag.  Renders the same per-kind diagnostic to stderr at each
  fault site and exits non-zero at end if any fault fired.

## Goal

Make every user-attributable runtime fault visible with a typed
kind + a `--> file:line:col` + caret diagnostic — through the
**rendered output** in development (halt-and-display so the
developer sees it loudly) and through the **logger** in production
(silent recovery via the existing sentinel, but an operator-
visible log entry so issues are still triaged).  Anything *not*
attributable to user code stays a hard panic and is treated as an
interpreter bug.

## Decision 04.A — sentinel, panic, or RuntimeError per site

Every fault site picks one of three behaviours:

| Behaviour | When | Example |
|---|---|---|
| **Sentinel (`null`)** | Op feeds the LHS of `??` (the C54.G-hybrid nullable opcode is emitted) | `attack / armour ?? 0` |
| **`RuntimeError`** | Op stands alone or feeds something other than `??` | `let dmg = attack / armour` (armour=0) |
| **Panic** | Interpreter invariant broken, never user-induced | bytecode pc out of range |

Codegen already chooses between non-nullable and nullable opcodes
based on whether `??` is the next operation (see `OpDivIntNullable`
at `src/fill.rs:517`).  Phase 4 keeps that decision but changes the
non-nullable failure mode from "sentinel + downstream silent
wrong-answer" to "raise `RuntimeError`".

## Steps

### 4a — `RuntimeError` type + propagation

```rust
#[derive(Debug)]
pub struct RuntimeError {
    pub kind: RuntimeErrorKind,
    pub position: Option<Position>,    // resolved via Data::source_at_pc
    pub op_pc: u32,
    pub fn_d_nr: u32,
    pub message: String,
}

#[derive(Debug, Clone)]
pub enum RuntimeErrorKind {
    DivideByZero,                       // `/`, `%`
    IndexOutOfBounds { idx: i64, len: u32 },
    NegativeIndex { idx: i64 },
    NullDereference,                    // null DbRef field/method access
    NarrowCastOverflow { value: i64, target: &'static str },
    StackOverflow,                      // recursion depth
    UserPanic { message: String },      // the `panic("…")` builtin
    AssertionFailed { message: String },
}
```

Propagation strategy: the interpreter's main loop (`src/state/mod.rs:1549`)
gets a per-thread `Cell<Option<RuntimeError>>` field on `State`.  A
fault-site opcode sets it and writes `u32::MAX` to `self.code_pos`,
which the existing loop terminator (`if self.code_pos == u32::MAX
{ break; }` at line 1607) already honours.

After the loop exits, `execute()` checks the cell.  If non-empty,
it bubbles up.  Top-level (`main.rs`) prints via the phase-2
renderer.  Inner `execute_at_*` callers propagate the error — most
are already infallible by signature; phase 4 widens them to
`Result<…, RuntimeError>` only where they're called from native
context (`#rust ""` annotations).

### 4b — Site list (every fault becomes typed)

Audit from phase 0's `0a-sites.md`.  Concrete site set:

| Site | File | Today | After phase 4 |
|---|---|---|---|
| Integer `/` | `src/ops.rs:305` | returns `i64::MIN` on v2=0 | `RuntimeError::DivideByZero` (non-nullable opcode); sentinel kept for nullable variant |
| Integer `%` | `src/ops.rs:315` | as above | as above |
| Float `/` | `src/fill.rs:886` | returns `f64::INFINITY`/`NaN` (Rust default) | `RuntimeError::DivideByZero` if v2=0 (non-nullable); IEEE behaviour kept on nullable |
| Vector index `v[i]` | `src/fill.rs::vec_get_*` | unchecked read or panic | `RuntimeError::IndexOutOfBounds` / `NegativeIndex` |
| Text index `t[i]` | `src/fill.rs::text_at` | as above | as above |
| Field access on null | `src/fill.rs::field_read_*` | unchecked deref → SIGSEGV | `RuntimeError::NullDereference` |
| Narrow cast | `src/native_utils.rs::narrow_int_cast` | wrap or sentinel | `RuntimeError::NarrowCastOverflow` (non-nullable variant) |
| `panic("msg")` builtin | `src/fill.rs:1675` | `panic!` | `RuntimeError::UserPanic` |
| Stack-overflow trap | `src/state/mod.rs` recursion guard | `panic!("Too many operations")` | `RuntimeError::StackOverflow` |
| Assertion (test stdlib) | `default/01_code.loft::assert` | `panic("…")` | `RuntimeError::AssertionFailed` (subkind of UserPanic) |

Out of scope (stays a panic — interpreter bug if it ever fires):
- Bytecode `pc` out of range
- Stack underflow
- Type-tag mismatch on a `Value` that codegen should have resolved
- `unreachable!` in second-pass walkers

### 4c — Per-site refactor pattern (REFRAMED — must always produce a sentinel)

Each site converts to call `s.raise(KIND)` AND continue producing
a sentinel value.  `State::raise` decides per-mode whether to halt
or log + continue (see `~/.claude/plans/async-toasting-nest.md` for
the production branch detail); the per-site code is identical for
both modes — it always produces the sentinel after the raise call:

```rust
// Today (lenient sentinel — silent wrong answer, no diagnostic):
fn div_int(s: &mut State) {
    let v_v2 = *s.get_stack::<i64>();
    let v_v1 = *s.get_stack::<i64>();
    let new_value = ops::op_div_int(v_v1, v_v2);    // returns i64::MIN on v2=0 silently
    s.put_stack(new_value);
}

// After phase 4 (sentinel kept, but raise emits the diagnostic):
fn div_int(s: &mut State) {
    let v_v2 = *s.get_stack::<i64>();
    let v_v1 = *s.get_stack::<i64>();
    let new_value = if v_v2 == 0 {
        s.raise(RuntimeErrorKind::DivideByZero);
        // Production: raise logged + returned; we now produce the
        //   sentinel so downstream `?? 0` / `if x != null` defenses
        //   keep working.  Execution continues.
        // Development: raise populated runtime_error; the dispatch
        //   loop will halt at this op's exit.  The sentinel we
        //   produce here is harmless — the dispatch loop fires
        //   before any consumer sees it.
        i64::MIN
    } else {
        ops::op_div_int(v_v1, v_v2)
    };
    s.put_stack(new_value);
}
```

The nullable opcode (`OpDivIntNullable`) and the loop-iteration
nullable peers (`OpGetVectorNullable` / `OpVectorRefNullable` /
`OpTextCharacterNullable`) skip the `s.raise(...)` call entirely
— they're the path codegen selects when `??` follows OR when
the IR site is a loop-iter step where end-of-iteration is
expected behaviour, not a fault.

### 4d — Stack-trace capture

`RuntimeError::position` covers the originating site.  But for a
user debugging a deep call chain, "divide by zero at
math.loft:12:8" is less useful than "divide by zero at
math.loft:12:8, called from game.loft:88:14, called from
main.loft:5:3".

Phase 4d snapshots `State::call_stack` into the error and exposes
it through the renderer:

```rust
pub struct RuntimeError {
    …
    pub backtrace: Vec<StackFrame>,    // most-recent-first
}

pub struct StackFrame {
    pub fn_d_nr: u32,
    pub fn_name: String,
    pub op_pc: u32,
    pub source: Option<Position>,      // resolved at capture time
}
```

The renderer prints up to 8 frames; user can set `LOFT_BT=full` to
get all of them.  The data is the same shape as
[STACKTRACE.md](../../STACKTRACE.md)'s user-callable
`stack_trace()` — phase 4d intentionally aligns with that design so
both sources reuse one type.

### 4e — Tests

- The 9 runtime cases from phase 0 (cases 17-25) are converted to
  `RuntimeError`.  The `.expect` files swap from "panic at …" to
  "error: divide by zero …".  Goldens regenerated.
- New `tests/runtime_errors.rs` for each kind: assert the error
  fires, the kind matches, the position matches, the backtrace
  contains the expected frames.
- `tests/issues.rs` and other suites that check for specific
  panic messages: re-baseline.  This is the only set of test
  changes outside `tests/error_messages/` — record in 4e's PR.
- `make bench` re-run.  The fault-check branch is added to every
  div / index / cast — needs measurement.  Bound: ≤ 3 %
  regression.  If breached, the check moves to a cold-out-of-line
  path (`#[cold]` + outlined fault-fn).

## Atomic landing sequence

Each row converts one fault site (or one infrastructure piece).
The site conversions can land in any order — they don't depend on
each other beyond the infrastructure (4.1).

| # | Step | Test |
|---|---|---|
| 4.1 | Add `RuntimeError`, `RuntimeErrorKind`; add `Cell<Option<RuntimeError>>` field to `State`; add `State::raise(kind)` helper that populates the cell and sets `code_pos = u32::MAX` | Unit test: `state.raise(DivideByZero)`; assert `state.error.is_some()`, kind matches, `code_pos == u32::MAX` |
| 4.2 | `execute()` checks the error cell after the dispatch loop and returns it; widen the few `execute_at_*` callers that propagate to native | Unit test: hand-build a tiny bytecode that calls `raise`, assert `execute()` returns the error |
| 4.3 | Convert `div_int` (`fill.rs:482`) to raise on `v2 == 0`; the nullable variant unchanged | Fixture case 17 (`runtime_div_by_zero_int.loft`): assert `kind == DivideByZero`, position = expected line/col |
| 4.4 | Convert `rem_int` (`fill.rs:489`) | Fixture case 19 (`runtime_mod_by_zero.loft`) |
| 4.5 | Convert `div_float` and `div_single` | Fixture case 18 (`runtime_div_by_zero_float.loft`) |
| 4.6 | Convert vector-index ops (positive OOB) | Fixture case 20: `kind == IndexOutOfBounds { idx, len }`, both fields assert |
| 4.7 | Convert vector-index ops (negative idx) | Fixture case 22: `kind == NegativeIndex { idx }` |
| 4.8 | Convert text-index ops | Fixture case 21 |
| 4.9 | Convert null-DbRef field/method access | Fixture case 23: `kind == NullDereference`, position points at the `.` token |
| 4.10 | Convert `narrow_int_cast` overflow (non-nullable variant) | Fixture case 24: `kind == NarrowCastOverflow { value, target }` |
| 4.11 | Convert `panic("…")` builtin (`fill.rs:1675`) → `UserPanic { message }` | Fixture case 25: kind matches, message is the user's literal |
| 4.12 | Convert recursion / stack-overflow trap (`state/mod.rs:1605`) → `StackOverflow` | Fixture cases 26, 27 |
| 4.13 | Convert `assert(...)` stdlib (`default/01_code.loft`) → `AssertionFailed` | New fixture: failed assertion produces typed kind |
| 4.14 | Capture `State::call_stack` into `RuntimeError.backtrace` at `raise` time (resolve each frame's source via phase 3's `Data::source_at_pc`) | Multi-frame fixture: `main → battle → divide_by_zero`, assert backtrace has 3 frames in order with correct positions |
| 4.15 | Render `RuntimeError` through phase-2's pretty renderer (kind label + position + caret + top-3 frames + "(use `LOFT_BT=full` for more)" if more) | Golden output for cases 17-27; `LOFT_BT=full` golden for case 26 |
| 4.16 | Re-run `make bench`; outline fault-check branches with `#[cold]` if hot | Bench delta ≤ 3 % vs phase 3 gates merge |

## Acceptance

- All 9 phase-0 runtime cases now emit `RuntimeError`-rendered
  output, not panics.
- `tests/runtime_errors.rs` covers every kind in
  `RuntimeErrorKind`.
- The only `panic!` reachable from valid loft code is the
  `panic("…")` builtin (which becomes `RuntimeError::UserPanic`).
- `make bench` ≤ 3 % regression vs phase 3.
- `make ci` green.
- `doc/claude/PROBLEMS.md` entries about silent div-by-zero
  sentinels are closed (or moved to "design — null sentinel kept
  for `??` only").

## Risks

| Risk | Mitigation |
|---|---|
| Per-op fault check slows hot loops | Use `#[cold]` outlined fault paths and `#[inline(always)]` happy-path checks.  Measure on `bench/01_classic`. |
| Existing tests that depended on `i64::MIN` propagating silently break | These tests were relying on undocumented behaviour; convert them to `??`-explicit.  Audit done in phase 0 already (case 17-25 in baseline).  Any *other* test that breaks here means a real semantic change — investigate per-test, do not paper over. |
| `RuntimeError` propagation requires touching every native fn | Most natives never raise; only the documented site-list does.  Other natives keep their `fn (&mut State)` signature.  The error cell is on `State` so propagation is implicit through control flow. |
| Backtrace capture allocates per error | Errors are rare; allocation is acceptable.  Cap at the most recent 32 frames to bound memory if recursion is deep. |
