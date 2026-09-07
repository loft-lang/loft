<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Closed-by-Decision Register

Items evaluated for inclusion and **explicitly declined** after
review.  This file exists so the same questions don't resurface in
every session.  Before proposing one of these as a bite, plan item,
or PR, check the entry here — if the situation hasn't materially
changed, the decision stands.

**Workflow rule** (see [DEVELOPMENT.md § Using this
register](DEVELOPMENT.md#closed-by-decision-register)):

- Closed-by-decision items are not "backlog" and should not appear
  in ROADMAP.md's scheduled milestones, PLANNING.md's priorities,
  or QUALITY.md's active tables.  A short reference in the
  "Out of scope" sections of those docs is sufficient.
- Re-opening requires **new evidence**: a concrete use case,
  incident report, or performance measurement that wasn't available
  at the decision.  Bring that evidence to the top of the revived
  entry; don't silently flip the decision.
- Adding a new entry requires the same rigor: the question, the
  evaluation, the decision, and the conditions under which the
  decision would change.

---

## Format

Each entry has:

1. **Question** — the proposal as it was raised.
2. **Evaluation** — the trade-offs weighed.
3. **Decision** — closed / accepted / partial, with the date.
4. **Revisit when** — the concrete trigger that would warrant
   reconsideration.

An entry that bounds a catalogued feature also carries a **`Catalogue:`**
line right under its header, listing the `@F###`/`@I###` entries it limits or
shapes. This is the cost/bound side of the feature catalogue (@PLN92): `idx
tag:@F<n>` then surfaces a feature's design bounds alongside its code and doc
anchors, with no per-feature `## Cost` prose to maintain.

---

## C3 — WASM `par()` runs sequentially

**Catalogue:** @F33 (par), @F54 (WASM/browser target).

**Question.** Should browser WASM builds parallelise `par()` loops
across a Web Worker pool?

**Evaluation.** Web Workers in a loft-compiled WASM require:
- Bundle-size overhead for the pool shim (KB per worker at minimum).
- Startup latency (cold-starting a worker is ~50 ms, dominates short
  frame budgets).
- Shared-memory configuration (`SharedArrayBuffer` requires COOP/
  COEP HTTP headers, which most loft hosting targets — itch.io,
  plain GitHub Pages — don't set).

None of the shipping loft programs are CPU-bound on the browser.
Brick Buster (the headline game) runs at 60 fps on a single thread.

**Decision.** **Closed — accepted limitation.**  Native target
keeps `par()` parallelism; browser is sequential.  Dated 2026-04.

**Revisit when.** A concrete loft program demonstrates a CPU
bottleneck on browser that can't be solved by algorithmic work, AND
the target host supports COOP/COEP headers.  Bring the profiler
trace.

---

## C38 — Closure capture is copy-at-definition

**Catalogue:** @F22 (closures & lambdas).

**Question.** Should lambda captures be by reference (like Rust
borrows) instead of by value (like Rust `move`)?

**Evaluation.** Reference capture requires either:
- **Garbage collection** — fundamental departure from loft's
  store-based heap with explicit scopes; changes every allocation
  path.
- **Borrow tracking** — Rust-style lifetimes and borrow checker;
  crosscuts every function signature, every type declaration, and
  the `#rust"..."` FFI layer.

Neither fits the "simple, fast, no lifetime annotations" language
ethos.  The value-semantic capture is also the less-surprising
default for beginners — the captured value is exactly what was
visible at lambda definition time.

**Decision.** **Closed — accepted design choice.**  Dated 2026-04.
Regression guard: `tests/scripts/56-closures.loft::test_capture_timing`.

**Revisit when.** A critical loft program cannot be expressed
ergonomically with value capture AND the alternative has been
prototyped to show it doesn't destabilise the store-based heap.

**Plan-22 addendum (shipped 2026-05-13).**  Plan-22 (mutable
closures) ships implicit-by-body mutation classification on top
of C38.  The "copy-at-definition" framing now applies only to
truly-immutable scalar captures in pure read-only contexts:

- Captures of `Type::Reference` (struct, nested struct) always
  use 12B `Parts::DbRef` pointing at the live original (@P260 fix,
  `src/parser/vectors.rs::synthesize_closure_record`).  Mutations
  from either side are visible immediately.
- Captures of scalars whose bodies write to the capture are
  promoted to heap-owned cells via the phase-02d-iii.a type flip
  (`Type::Reference(__cell_<T>, vec![])` encoding).  The outer
  scope and the (single) mutating closure share the same cell —
  sharing the cell across SEVERAL closures is rejected at compile
  time since C74 (#314).
- Pure read-only scalar captures remain value-copy (Case A
  semantics — unchanged).

Case D ("aliased mutating") was decommissioned 2026-05-13: the
cell + auto-Reference machinery from phases 02-03 already gives
shared-state semantics, so no rejection was needed.  Design lives
in the closed plan README:
[plans/finished/22-mutable-closures/](plans/finished/22-mutable-closures)
— Case D's major finding sits in its phase file, and the
alternatives considered are in `DISCUSSION.md` alongside.

---

## C54.D — Rust-style numeric literal suffixes

**Catalogue:** @F4 (width integers), @F5 (type conversions — `as`).

**Question.** Should loft accept `34u8`, `4948u32`, `100i32` as
literal syntax for explicit-width integer constants?

**Evaluation.** Loft's context-driven type inference already
handles every common case:

- `x: u8 = 255;` — range-check at the binding site.
- `f(a: u8)` called as `f(34)` — literal constrained by parameter
  type.
- Ambiguous cases — `34 as u8` (one existing operator, no new
  syntax).

Adding suffix syntax would:
- Crosscut the lexer (ambiguity with identifiers: `1u8` vs `1_u8`).
- Conflict with loft's "prefer the type annotation over the literal
  annotation" ethos (the binding site documents intent, not the
  literal).
- Solve a 1 % problem that `as` already covers.

**Decision.** **Closed — declined.**  Dated 2026-04-13.  See
[QUALITY.md § C54](QUALITY.md#active-design--c54-integer-i64) — `C54.D` listed under sub-tickets.

**Revisit when.** A real loft program needs a literal-size
distinction that cannot be expressed as `as <T>` in reasonable
syntax.  "I wrote it in Rust that way" is not sufficient evidence.

---

## C62 — No type annotations in `|x|` shorthand lambdas

**Catalogue:** @F22 (closures & lambdas).

**Question.** Should loft accept type annotations on shorthand
`|x|` lambda parameters (e.g. `|x: integer, y: integer| { x + y }`)?

**Evaluation.** Loft already has two orthogonal function syntaxes:

- `|x| { body }` — the **inferred** shorthand, designed for use
  inside `map` / `filter` / `reduce` and other higher-order calls
  where the expected parameter types flow in from the call site's
  lambda hint.  Its whole reason to exist is visual compactness.
- `fn(x: T, y: T) -> R { body }` — the **explicit** form, with
  full type annotations and an optional return type (omit `->` for
  void returns).  Use this when the types can't be inferred — for
  example, when the lambda is stored in a local variable before it
  reaches a call site.

Adding types to the shorthand:
- Collapses the distinction — the two forms now mean exactly the
  same thing (one with `|` delimiters, one with `fn(` keyword) and
  each style becomes a coin-flip.
- Blurs the "where types flow from" mental model: users stop
  asking "is this inferrable?" and start writing `|x: T|` by
  habit, defeating the point.
- Complicates the parser (currently `|x|` is unambiguous with
  `|` as bitwise-or via lookahead on parameter shape; adding `: T`
  introduces more disambiguation branches).

Users who want types should use `fn(...)`.  There is no scenario
where `|x: T| { ... }` is the only viable syntax — if types are
wanted, `fn(x: T) { ... }` has every capability plus an explicit
return type when needed.

**Decision.** **Closed — declined.**  Dated 2026-04-17.  The
compiler rejects `|x: T| { ... }` with an error that points at
the `fn(x: <type>) { ... }` form (P169 updated the wording).

**Revisit when.** Never, barring a language-level change that
eliminates the inferred-shorthand / explicit-fn distinction
altogether (i.e. a fundamental rewrite of the lambda story).

---

## C63 — No nested `fn` definitions inside fn bodies

**Catalogue:** @F16 (functions & declarations).

**Question.** Should loft accept `fn` declarations inside another
function's body (e.g. `fn outer() { fn helper(x: integer) -> integer
{ x * 2 } helper(5) }`)?

**Evaluation.** Loft already has two orthogonal forms for the
"function-shaped local helper" use case:

- `let helper = |x| { x * 2 };` — closure shorthand, inferred types.
- `let helper = fn(x: integer) -> integer { x * 2 };` — typed
  closure form.

Both bind a function value to a local variable, callable inside
the parent fn (`helper(5)`), invisible outside, dropped at scope
exit.  Adding a third syntax via nested `fn` declarations forks
into one of two semantic choices, neither of which is good:

1. **Captures parent locals** — re-implements closures with
   different syntax.  Two forms now mean the same thing; users
   coin-flip between them with no clear heuristic.  Documentation
   surface grows for cosmetic-only gain.
2. **Doesn't capture** — contradicts every mainstream language
   (Python, JS, Rust, Swift all give nested fns access to outer
   locals).  Users write `fn helper() { use(outer_var) }` expecting
   it to work; the parser rejects with "can't find outer_var."
   That's a worse experience than the current "no nested fns"
   rule, which fails clearly with a single-line diagnostic.

Implementation cost is also non-trivial: parser path for local
fn decls; scope analysis for capture/no-capture; codegen path or
desugaring rules to existing closures; edge cases for forward
refs, recursive nested fns, mutual recursion.  Estimated 1-2
focused sessions for a feature whose runtime benefit is zero
(closures already cover every use case) and whose only gain is
cosmetic locality.

The "with working closures" framing is the giveaway: if inline
fn semantics ARE just "closure with implicit name binding," the
feature is pure sugar.  Sugar that's not load-bearing for any
concrete user need is exactly what pre-1.0 should refuse —
every additional surface is one more thing to validate,
document, and maintain.

**Decision.** **Closed — declined.**  Dated 2026-05-04.  The
parser continues to reject `fn` definitions inside function
bodies with the existing `'fn' definitions must be at file
scope, not inside a function or block` diagnostic.  The
loft-write skill's "Nested fn definitions are forbidden"
section documents the workaround (typed lambda).

**Revisit when.** A concrete user workflow surfaces where the
typed-lambda form (`let helper = fn(x: T) -> R { ... };`) is
genuinely awkward enough to cost more developer time than the
inline-fn implementation cost.  Today (2026-05-04): no such
workflow has surfaced; the loft-test cross_mode harness pulls
helper fns to file scope cleanly, and `lib/graphics/` plus the
stdlib never need local-helper recursion.

---

## C64 — Tuple struct-ref elements use MOVE semantics (not copy + null)

**Catalogue:** @F11 (tuples).

**Question.** When a tuple element is a struct reference
(`Type::Reference`), what semantics apply at scope exit and on
destructuring?  Two candidates:

- **Move semantics:** the source tuple's scope-exit emits no
  per-element `OpFreeRef`; each destructured destination variable
  owns its element.
- **Copy + null semantics:** the destructure copies the DbRef,
  then nulls out the source tuple's slot via a new
  `OpNullTupleElem(var, offset)` opcode so the source's scope-exit
  cleanup is a no-op for the moved-out element.

**Evaluation.** Move semantics is what the runtime already does:
`src/scopes.rs:1000-1009`'s tuple scope-exit arm is a `continue`
stub (no per-element `OpFreeRef` on the source tuple), and
`parser/expressions.rs::parse_assign`'s destructure path types
each destination variable as the source element's `Type::Reference`
via `change_var_type(v_nr, &rhs_elems[i])` so the destination
gets ordinary scope-exit cleanup.  The Plan-14 phase 04 cross-mode
harness exercised every single-iteration E5 shape (swap, arg,
return, mixed Ref+int, mixed Ref+text, plain local) on both
backends with byte-identical output and no panics — confirming
move semantics is correct in practice.

The "copy + null" alternative would require a new opcode at a
time when the opcode space is near-saturation (254/256 used per
CHANGELOG_TECHNICAL.md), would add a per-destructure runtime
write the move path doesn't need, and produces no observable
difference to user code — only the runtime cleanup ordering
changes.  No concrete program shape exists that move-semantics
gets wrong but copy+null would get right.

The loop-iteration aliasing bug (@P250 — `for { (q1, q2) =
make_pair(pa, pb); }` reads `null` for whichever destructured
variable picked up the FIRST argument on iterations >0) is a
SEPARATE dep-tracking issue, not a move-vs-copy semantics
question.  Both candidates would have the same loop-iter problem
without an additional fix; the bug lives in the destructure
path's dep propagation between source argument slot and
destination variable slot.

**Decision.** **Move semantics.**  Dated 2026-05-11.  The
runtime path is locked by 6 cross-mode E5 cells in
`tests/tuple_matrix.rs` (e5_d1_struct_ref_local + swap +
ref_int_local + ref_text_local; e5_d2_struct_ref_arg + return).
Plan-14 phase 04 records the rationale.

**Revisit when.** A concrete shape appears where move semantics
is observably wrong but copy + null would be correct — none
known as of 2026-05-11.  @P250's fix lives in dep-tracking, not
in the move/copy axis.

---

## C65 — Tuple "structure value" element type folded into reference (E5 = E6)

**Catalogue:** @F11 (tuples).

**Question.** Should the validation matrix carry a separate E6
element type for "structure value" (an inline by-value struct
copied into the tuple slot, distinct from `Type::Reference`)?

**Evaluation.** Loft has no inline by-value struct type distinct
from `Type::Reference`.  A `struct Foo { ... }` declaration
produces a record laid out in a store; the loft-level "value" you
pass around is a `Reference(struct_def, dep)` — a 12-byte
`DbRef`.  Tuple element E5 (Reference) already covers this shape
end-to-end.

Carving out a separate E6 row would either (a) duplicate every
E5 cell with no semantic difference, or (b) require introducing
a new `Type::StructValue` variant — a substantial language-design
change with no consumer.  Neither pays for itself.

**Decision.** **Folded into E5.**  Dated 2026-05-11.  Plan-14's
matrix in `00-matrix.md` marks every E6 cell as
`CLOSED:folded into E5`.  A future feature that introduces inline
value structs (none on the roadmap) would re-open the row.

---

## C66 — Production loft programs never abort on user-attributable edge cases (development may halt)

**Catalogue:** @F38 (arithmetic safety), @F44 (logging — panic/assert).

> **Revised by [C80](#c80--the-spreadsheet-fault-model-nothing-stops-a-running-calculation)
> (2026-06-24).** The "development *may halt*" half no longer applies to **calculation**
> faults (divide-by-zero, overflow, OOB, null-deref): those now yield **null and continue in
> every mode** (the spreadsheet model), silently by default. C66's split still holds for the
> *explicit* signals — `panic` / `assert` halt in dev/test, log + continue in production — and
> for startup ([C67](#c67--fail-at-startup-not-at-runtime)).

**Question.** Should runtime fault sites (divide-by-zero, vector /
text out-of-bounds, null DbRef dereference, narrow-cast overflow,
`panic("msg")` / failed `assert`) HALT the loft program with a typed
runtime error (the rustc-style "loud failure" model)?  Or should they
return a silent sentinel (null / `i64::MIN` / null DbRef / char 0)
and let execution continue?

**Evaluation.** Loft's primary deployment target is **interactive
programs that must not stop**: browser games shared via URL, native
games with continuous frame loops, scriptable scenes inside
applications, multiplayer servers driving live sessions.  In every
one of those contexts an abort is far worse than a wrong-pixel /
wrong-frame / wrong-record edge case.  A frozen game cannot be saved
by the user; a wrong pixel can.

But during **development** — running tests, debugging a script,
iterating on a feature — halting on a runtime fault is a feature,
not a bug.  Halt-on-fault is how you find the divide-by-zero you
didn't know about, the OOB the test missed, the assert that's
firing.  The dev tooling (CLI, test harness, REPL) wants the loud
failure mode; the production game / server / browser embed does
not.

This separation is not new.  Loft has always carried it via the
`production` flag on the logger:

- `default/01_code.loft::panic / assert` are documented as
  **logging in production** (`#impure(host_io)`); the
  production-mode path in `src/native.rs::n_panic` and `n_assert`
  checks `logger.config.production`.  When `production == true`
  it writes a fatal log entry, sets `Stores::had_fatal = true`, and
  **returns** — execution continues.  When `production == false`
  (or no logger attached) the same site halts via the typed
  `RuntimeError` path so the developer sees the failure.
- `main.rs` reads `had_fatal` after execute returns and exits 1
  ONLY when no frame loop is running.
- Integer `/` by zero, `%` by zero, vector / text OOB, null DbRef
  field reads, narrow-cast overflow today return null sentinels
  (`i64::MIN`, `char(0)`, `DbRef { rec: 0 }`) and let downstream
  code keep running — even in dev mode.  Plan-07 phase 4
  converts these to typed-error halt **in dev mode only**;
  production keeps the silent + log shape.
- The `??` operator is the user's tool for explicit handling of
  null sentinels: `x = a / b ?? 0` discharges the null with a
  fallback at the user's choice.
- The `RuntimeLogger` framework (`doc/claude/LOGGER.md`) is the
  surface for surfacing edge-case incidents to operators without
  halting the program.

**Decision.** **Loft programs in production MUST NOT abort on
user-attributable runtime edge cases.**  Dated 2026-05-11.
Development is unaffected; halt-on-fault is the right behaviour
for the test runner, the CLI, and interactive debugging.  The
gate is the existing `Stores::logger.config.production` flag.

For each fault site:

| Site | Production (`logger.config.production == true`) | Development (default) |
|---|---|---|
| Integer `/`, `%` by zero | log warning, return `i64::MIN` sentinel, continue | typed `RuntimeError::DivideByZero`, halt + render |
| Vector / text OOB (positive, negative past start) | log warning, return null DbRef / char 0, continue | typed `RuntimeError::IndexOutOfBounds` / `NegativeIndex`, halt + render |
| Null DbRef field / method access | log warning, return null result, continue | typed `RuntimeError::NullDereference`, halt + render |
| Narrow-cast overflow | log warning, return clamped or null, continue | typed `RuntimeError::NarrowCastOverflow`, halt + render |
| `panic("msg")` builtin | log fatal, set `had_fatal`, return | typed `RuntimeError::UserPanic`, halt + render |
| Failed `assert(test, msg)` | log error, set `had_fatal`, return | typed `RuntimeError::AssertionFailed`, halt + render |
| Stack-overflow trap | log fatal, attempt graceful unwind | typed `RuntimeError::StackOverflow`, halt + render |

**Three-way defense contract** — codegen picks the opcode based on
how the user's code handles the fault potential:

| Defense at compile-time | Emit | Runtime behaviour |
|---|---|---|
| `expr ?? fallback` | Nullable peer (existing `parser/operators.rs::rewrite_outer_arith_to_nullable` extended to vector/text) | No log, no halt; sentinel discharged by `??` |
| `if x != null { use(x); }` (or `if !x { … }`) immediately after `x = expr` (statically detectable defensive check) | Nullable peer (new flow-analysis arm in parse_assign) | No log, no halt; user's defense handles the null |
| Neither | Raising peer | Production: log warning + continue with sentinel.  Development: halt + render. |

The static-analysis fallback "raising peer + runtime warning" is
the safety net — it catches sites the developer forgot to defend
AND sites where the defense lives across a function boundary
(can't be statically detected).  In production those sites still
produce a sentinel and execution continues; the log entry surfaces
the issue to operators for investigation.

A compile-time warning fires at every undefended fault site that
the parser can statically recognise as fault-prone:

```
warning: `v[i]` may produce null on out-of-bounds with no defensive check
  --> game.loft:42:8
   = note: guard with `if i < len(v) { ... }` before indexing
   = note: or accept null with `v[i] ?? <fallback>`
   = note: or follow with `if x != null { ... }` to catch the null
```

Silenceable via `LOFT_NO_WARN_RUNTIME=1` for codebases where the
warning rate is too high (rarely needed once defensive idioms are
adopted).  The warning is BOTH a quality nudge for developers AND
a way to silence the production-mode runtime log: defending the
site (any of the three patterns) makes the warning go away AND
the log goes silent.

**Production mode REQUIRES a logger.**  A deployment configured
for production with no logger attached MUST refuse to start with a
clear, actionable error message — there is no "production but no
logging" middle state.  The reason: in production every runtime
event needs a destination, and silently swallowing them defeats
the entire point of choosing the production path.  The startup
check fires before user code runs:

```
Error: production mode requires a logger.
Attach one via `loft::logger::Logger::attach(...)` at startup,
or configure via the deployment's logger settings (see
doc/claude/LOGGER.md § Production setup).  To run in development
mode (halt-on-fault), unset the production flag.
```

The deployment is expected to attach a real sink (stderr, file,
syslog, host bridge, etc.) at startup.  If a deployment genuinely
wants "log + continue but discard the output", it attaches a
no-op sink — that's a deliberate choice, made visible in the
deployment config, not an accident from forgetting to attach a
logger.

**Implementation shape (Plan-07 phase 4 reframe 2026-05-11):**

The typed-error infrastructure (`RuntimeError` type +
`RuntimeErrorKind` variants + `--> file:line:col` + caret rendering
through phase-2's `render_entry_pretty`) is **kept** — it's the
right shape for both the dev-mode halt diagnostic and the
production-mode log entry.  Per-site helpers (`State::raise`,
`vec_get_or_raise`, `vec_ref_or_raise`, `text_char_or_raise`)
need a production-mode branch added:

```rust
// In State::raise (sketch):
let production = self.database.logger.as_ref()
    .and_then(|l| l.lock().ok())
    .is_some_and(|l| l.config.production);
if production {
    // Production: log + had_fatal, do NOT short-circuit code_pos.
    // The op returns its sentinel; execution continues.
    // The logger is GUARANTEED present here — startup refuses
    // production mode without one.
    if let Some(logger) = &self.database.logger {
        if let Ok(mut lg) = logger.lock() {
            lg.log_runtime_kind(&kind, position.as_ref());
        }
    }
    self.database.had_fatal = true;
    return;
}
// Development: keep the typed-error halt path that's already in place.
self.database.runtime_error = Some(Box::new(RuntimeError { ... }));
self.database.had_fatal = true;
// (dispatch loop sees runtime_error.is_some() and short-circuits)
```

The `OpGetVectorNullable` / `OpVectorRefNullable` /
`OpTextCharacterNullable` opcodes added 2026-05-11 stay as the
form for **loop iteration end** specifically — they return null
without logging in either mode (end-of-iteration is expected
behaviour, not a fault).  The raising peers (`OpGetVector`,
`OpVectorRef`, `OpTextCharacter`) log + return null in production
and halt + render in development for the user-facing `v[i]` /
`s[i]` paths.

**Revisit when.** A concrete deployment shape surfaces where the
production-mode silent + log path is wrong.  No such case is
expected; the existing `n_panic` / `n_assert` production-mode
behaviour has held for years without complaint.

### Workflow corollaries (added 2026-05-11)

The 2026-05-11 evaluation of the C66 framework against the
day-to-day loft-development workflow surfaced four corollaries
that bind the abstract C66 rule to the developer experience.
All four are tracked under @PLN28 phase 4:

1. **Format strings are observability, never raise** — every
   `{...}` interpolation auto-swaps to its Nullable peer at
   parse time.  The `println("{x}")` you reach for to inspect
   a bug must NEVER itself become the next bug.  Shipped as
   phase 4e.1 (commit 8e74aa16).  Reasoning: a halt or log
   inside the developer's diagnostic surface defeats the
   point of the diagnostic.

2. **Easy-proof skip list is REQUIRED for the warning** — the
   4e.2 compile-time warning at undefended fault sites must
   recognise the four canonical safe patterns (bound loop
   variable / explicit length check / constant-literal
   divisor / constant-literal index against known-length
   vector) BEFORE landing.  A noisy warning gets disabled
   within a session and the safety net evaporates; the skip
   list is a release blocker, not a follow-up.  See
   `plans/28-error-messages/04-runtime-error-kinds.md
   § Easy-proof skip list — REQUIRED for 4e.2`.

3. **State snapshot at fault site is the next-highest-leverage
   workflow win** — the dev-mode halt today says *where* the
   fault was but not *what* the values were.  Phase 4g.2
   captures the named-arg values + indexed-collection length
   into the rendered diagnostic so the developer sees
   `damage = [10, 20, 30] (len=3), idx = 5` without having
   to add a print statement to discover it.  The values are
   already on the bytecode stack at fault time; surface them.

4. **`not null` field reminder closes the long-term failure
   mode** — when a struct field is read 47× across the
   codebase and never compared to null, marking it `not null`
   at the constructor eliminates the entire class of fault
   sites for that field.  Phase 4h emits a `Level::Hint`
   pointing at the constructor when the read pattern says
   "this is morally not-null, mark it so."  Strictly better
   than defending each read site individually with `?? null`.

The corollaries together prevent the failure mode where
`?? null` becomes pervasive defensive boilerplate that the
2026-05-11 evaluation flagged ("everyone writes `?? null`
everywhere because the warning fired").  4e.1 + 4h reduce
the *count* of sites where the warning could fire; 4e.2's
skip list ensures the warning only fires where defending
is actually needed; 4g.2 makes diagnosis fast when defence
is needed and not yet in place.

---

## C67 — Fail at startup, not at runtime (no programmer-side try/catch for internal bugs)

**Catalogue:** @F44 (logging — panic/assert).

### Question

When a loft program hits an internal-bug runtime panic (a
`todo!()`-stubbed native, a codegen mistake, an unimplemented
operator), should we add programmer-side error-recovery
constructs (`try { … } catch err { … }`, `#panic_safe fn(T)`
annotations, defensive `unwrap_or_else` boilerplate) so the
program can continue?

### Evaluation

Three competing pressures:

1. **Robustness perception** — users want loft programs to
   "just work."  A panic mid-execution feels broken regardless
   of whose fault it is.
2. **Programmer burden** — every defensive-wrap mechanism
   forces the user to remember a pattern.  Forgetting one
   wrap means a production crash.  The pattern proliferates
   ("everyone writes try/catch everywhere") until it's
   indistinguishable from the language not having had error
   recovery at all.
3. **VM-route fail-fast** — production loft programs run
   under a supervisor (systemd, kubernetes, containerd) that
   already monitors process exits and decides restart /
   escalate / page.  Hiding crashes from the supervisor
   defeats its job.

Three layers of failure exist:

- **Internal bugs** (todo!() stubs, type mismatches, codegen
  errors) — the LANGUAGE / RUNTIME made the mistake.  Should
  never reach a running program.
- **Startup-time external faults** (config invalid, port bind
  failure, missing deps) — the PROGRAM can't sensibly continue.
  The supervisor needs to know.
- **Steady-state external faults** (network drop, disk full,
  transient I/O error) — the LIBRARY handles them gracefully
  (auto-reconnect, retry, fallback to default).  The user-code
  in the loft program never sees them.

### Decision

**Closed (2026-05-13)** as a layered policy.  The user's framing:

> *"I do not want to burden the programmer with… write try
> catch or your program will fail.  So everything will need a
> try catch to function, we should not go that path.  Things
> have to function properly.  We can fail but that should be
> on startup and not during the running of a program."*
>
> *"We can still allow for runtime exceptions in the future,
> things can and will break.  Though the detection of it needs
> to be in a start-up phase, we go the VM route of failing
> fast so the VM manager is informed of a problem."*
>
> *"After initial startup we will do our utmost best to keep
> running."*

Three load-bearing rules:

1. **Internal bugs are caught at compile time, not at
   runtime.**  The codegen refuses to emit a binary that
   contains a reachable `todo!()` stub or any other
   would-panic-on-call construct.  If a loft program would
   panic from an internal mistake when called, the build
   fails with a clear message ("native fn `n_X` has no
   implementation; wire it in src/codegen_runtime.rs or run
   via --interpret").  This is the **compile-time** layer.
   Sibling work: every codegen path that historically emitted
   `todo!()` for an unimplemented native is now a hard
   compile-error per @P269.
2. **Startup faults exit the program with non-zero status.**
   No catch_unwind, no logging-and-continuing.  The supervisor
   sees the exit code and decides.  This is the **VM-route
   fail-fast** layer.
3. **Steady-state runtime faults are HANDLED BY LIBRARIES,
   not by user code.**  lib/web's WebSocket auto-reconnects.
   lib/server's eventual `serve(handler)` primitive owns its
   own per-request `catch_unwind` (logs + 500 + continues
   serving).  lib/io returns explicit error values rather than
   panicking.  The USER's loft program never writes
   `try { … } catch { … }` for these — the LIBRARY hides the
   mechanism behind a clean API.  This is the **best-effort
   keep-running** layer.

**No `try { … } catch err { … }` language construct, no
`#panic_safe` annotation, no programmer-side `?? null`
boilerplate** for internal-bug recovery.  Loft's typed-error
infrastructure (per CLAUDE.md) IS allowed — that's about
EXPLICIT user-domain errors (parse failures, validation,
business-logic invariants) where the user wrote code that
returns `Result`-like values and consciously handles them.
That's different from internal-bug recovery, which is what
this decision rejects.

### Revisit when

- A class of internal bug surfaces that compile-time analysis
  CAN'T statically prove safe (e.g. parser-generated dispatch
  to runtime-discovered code paths).  Then the lib/server-style
  catch boundary may need to extend deeper.
- A real production loft program is run under a supervisor
  that genuinely benefits from in-process recovery (e.g. a
  long-running game server where restart cost is high), AND
  the library-level `serve(handler)` primitives can't cover
  the use case.  Then a narrow loft-side error-recovery
  primitive may be evaluated.
- User-code patterns emerge that NATURALLY want exception-style
  flow (e.g. deeply-nested validation chains where the typed-
  error mechanism becomes more boilerplate than the catch
  would).  Then the typed-error infrastructure may evolve;
  this decision specifically blocks try/catch for the
  internal-bug-recovery use case.

Pointer from the source: see PROBLEMS.md row @P269 for the
specific incident this decision was crystallised in (server
process died on todo!() panic during the @P268 fix work);
the compile-time check shipped 2026-05-13 in
`src/generation/mod.rs::output_function`.  Memory-system
mirror: `feedback_fail_at_startup_not_runtime.md`.

---

## C68 — Keyed collections dedup on insert (`+=` AND `coll[key]=value`)

**Catalogue:** @F7 (hash), @F8 (sorted), @F9 (index).

> **REVERSED & IMPLEMENTED (2026-05-21).**  The original decision below
> (close @P306 as a `+=`-append-vs-`[key]=`-upsert split) was reversed the
> same day on **new evidence**: the world-chunk index is `hash<Chunk[cx,cy,cz]>`
> and inserting a chunk where one already exists at a coord MUST replace, not
> stack a shadowed duplicate — so dedup-on-insert is a correctness requirement
> of the architecture, not a preference.  The risk concern was retired by
> doing it the safe way: hash/index dedup via `Stores::dedup_keyed`
> (find + free + unlink + reclaim, then add) in `insert_record`; sorted via an
> overwrite-on-`found` branch in `vector::sorted_finish`.  Full suite green
> (no consumer relied on duplicate-key append).  **Both `coll += [entry]` and
> `coll[key] = value` now dedup by key (latest insert wins).**  @P306 is
> closed by CODE, not by this decision.  The historical decision is kept below
> for the trade-off record.

### Question

For a keyed collection (`hash` / `sorted` / `index<T[K]>`), what
should `coll += [entry]` do when an entry with the same key already
exists?  Append a second record (current behaviour — `len` grows,
lookup returns the first), or replace (dedup)?  Filed as @P306 during
the plan-44 hash-semantics sweep.

### Evaluation

- A keyed collection implies key UNIQUENESS, so silently coexisting
  duplicate keys are a footgun: `coll[key]` returns the first, the
  duplicate is dead weight, and `len` over-counts.  This also wastes
  space — bad for the cache-locality goal that the chunk work is built
  around (a chunk's working set should fit L1/L2; bloat from duplicate
  keys pushes it out).
- BUT making `+=` dedup means modifying the hot insert path of THREE
  data structures (`hash::add`, `vector::sorted_finish`, `tree::add`)
  to find-and-replace on key collision — the @P295 minefield (the hash
  deep-copy/off-by-one bugs all lived here).  High regression risk for
  a change to a primitive every consumer uses.
- `coll[key] = value` (the @P305 `OpSetKeyed` upsert) ALREADY provides
  a correct, deduping, memory-bounded insert-or-replace for all keyed
  kinds, uniformly across local / field / `&`-param.  It is the
  compact, locality-friendly path the chunk work actually wants.

### Decision

**Closed (2026-05-21) as a two-operation split:**

- **`coll += [entry]`** is APPEND — the low-level primitive.  On a
  keyed collection a duplicate key is the caller's responsibility (it
  appends; no dedup).  Cheapest when keys are known-unique (the common
  bulk-build case — `build_index`, snapshot loads — never inserts a
  duplicate, so it pays nothing for a dedup check).
- **`coll[key] = value`** is UPSERT (insert-or-replace, deduped by the
  value's key) — the idiomatic map/set write.  Use it whenever keys may
  repeat or compactness matters.

This keeps the hot append path untouched (no @P295-class risk) while
giving a correct dedup path.  It mirrors the common library split
(`push`/`append` vs `insert`/`[]=`).  @P306 is closed by this decision,
not by a code change.

### Revisit when

- A consumer needs bulk dedup-append (many possibly-duplicate keys at
  once) and the per-element `coll[key] = value` loop is measurably too
  slow — then a dedup-aware bulk insert (or a `+=` dedup variant) can be
  evaluated, implemented in the per-kind add with the @P295 lessons in
  hand.
- Duplicate keys are shown to corrupt a downstream operation
  (iteration, deep-copy, removal) rather than merely waste space — then
  `+=` dedup becomes a correctness fix, not a preference.

Pointer from the source: PROBLEMS.md row @P306; spec + matrix in
[plans/finished/44-hash-semantics/](plans/finished/44-hash-semantics/README.md) (cases
C09/C10 document the append-no-dedup behaviour this decision blesses).

---

## C69 — `!x` on a non-boolean is a null test, not logical-not

**Catalogue:** @F37 (operators — unary `!`), @F1 (null model).

### Question

`!x` where `x` is a non-boolean (integer, single, reference, …) reads as
*"is `x` null?"* rather than C-style logical-not.  Because the null sentinel
is **in-band** (`i64::MIN` for integer, byte `255` for boolean per @PLN17 / C73,
null `DbRef` for references), `!0` is `false` — `0` is a real value, not the
sentinel.  (This C69 decision is about *non-boolean* `!x`; boolean `!` is ordinary
coercing negation — both `null` and `false` give `!b == true`.)  A
crawler report (GitHub #253) hit this as a footgun: `f = 0; if !f { … }` never
runs, and a `gl_load_font` zero-handle check silently misses.  Should `!`
either coerce non-booleans C-style (`!0 ⇒ true`) or reject them as a
compile-time type error?

### Evaluation

Both proposed fixes break a load-bearing idiom:

- **C-style coercion (`!0 ⇒ true`)** would invert the meaning of every
  `!nullable` null test.  The stdlib `min` / `max` / `clamp` use `!both` to
  mean *"both is null"* (`default/01_code.loft`); coercion would silently
  change them to *"both is zero"*.
- **Compile error on `!<non-bool>`** rejects that same stdlib idiom, and any
  user code that writes `!handle` to mean "absent".

The asymmetry is **documented** (`LOFT.md` → *"`!value` asymmetry — read
carefully"*) and intrinsic to the in-band-sentinel memory model (the same
model that makes `C66`'s "return a sentinel, keep running" possible).  `!x`
as a null test is the *deliberate* shape: for a nullable `x` it is a real,
useful check; the surprise only arises when a reader imports C's
value-falsiness expectation.

What IS cleanly fixable is the **always-false** subset: `!x` on a statically
`not null` operand can never be the sentinel, so it is provably a no-op.  That
gets a compile-time **warning** (zero false positives — nullable operands and
boolean `!` stay quiet), mirroring the redundant-`??` warning.  Going further
(deprecating `!` on non-booleans, migrating the stdlib to `x == null`) was
weighed and declined: it removes a working idiom to chase a confusion that the
documentation + the always-false warning already address, and it churns the
stdlib's hottest helpers for cosmetic gain.

### Decision

**Closed — accepted design choice.**  Dated 2026-06-03.  `!x` on a
non-boolean stays a null test (sentinel-in-band).  Added safety net: a
`Level::Warning` at `!` of a statically `not null` non-boolean operand
("always false — `!x` tests whether x is null …"), landed in
`src/parser/vectors.rs::parse_single`.  Regression guards:
`gh253_bang_on_not_null_warns` (warns) and `gh253_bang_on_nullable_is_quiet`
(no false positive) in `tests/parse_errors.rs`.  GitHub #253 closed `by-design`
pointing here.

### Revisit when

A concrete loft program demonstrates that the null-test idiom causes a
recurring, hard-to-diagnose class of bugs that the always-false warning + the
documented asymmetry don't catch — AND a replacement (e.g. reserving `!` for
boolean with an explicit `x == null` null test) has been prototyped to show it
doesn't churn the stdlib's hot paths or surprise the in-band-sentinel model.
"I expected C semantics" is not sufficient evidence.

---

## C70 — No per-library IR snapshot / cache

**Question.** @PLN11 (Data-as-store) caches the compiler IR as mmap'able
store records.  Should a **library** be able to ship (or the toolchain
cache) *its own* IR snapshot independently, so a never-seen `use`
combination doesn't pay a full parse on first run?

**Evaluation.** Two independent reasons, both permanent:

- **A library cannot cleanly write its own IR.**  The IR is global-index:
  `def_nr` / `known_type` are absolute and parse-order-dependent (core and
  every `use`d lib append into one global `Data.definitions`).  A library
  snapshotted in isolation would have to be **relocated** by name into
  whatever prefix it lands in — the single brittlest mechanism in the whole
  caching design, and it optimizes only the least-common case (the first run
  of a brand-new lib-set).
- **The loft source is the better representation of a library's state.**  For
  distributing / versioning / inspecting a library, the `.loft` source — not a
  serialized IR image — is the right artifact.  And there is **no efficiency
  case** for a serialized per-library form: @PLN82 established by measurement
  that JSON (de)serialization is **not** faster than parsing natural loft
  source (both deserialize text into the same heap graph; ~15–24 ms load ≈
  ~11–23 ms parse).  A per-library IR cache would be a worse, harder-to-relocate
  stand-in for something the source already expresses well *and* parses just as
  fast.

The whole-bundle snapshot (core + a script's sorted lib-set, as one image with
internally-consistent indices) sidesteps relocation entirely and captures the
repeated-run win that actually matters for the dogfood consumers.

**Decision.** **Closed — declined.**  @PLN11 caches only **whole-prefix
bundles** (core stdlib; core + per-script lib-set); never independent
per-library IR.  Dated 2026-06-02 (first raised 2026-05-31).

**Revisit when.** A measured workload shows first-run parse of a never-seen
`use` combination is a real bottleneck (repeated runs already hit the bundle
cache), AND a relocation scheme is prototyped that doesn't reintroduce the
global-index brittleness — bring the parse-time profile and the relocation
design together, not separately.

Pointer from the source:
[plans/11-data-as-store/](plans/11-data-as-store/README.md)
§ What gets cached (per-library "dropped, not deferred").

---

## C71 — Native libraries compile, scripts interpret — the steady-state execution model

**Catalogue:** @F53 (native backend), @F55 (package management).

> *Build-plan seed:* the engine-host design exploration —
> [plans/18-engine-host/ENGINE_HOST.md](plans/18-engine-host/ENGINE_HOST.md) (tier model per
> LAVITION § Execution granularity, entry-gate probes, the main-loop IO contract,
> dogfood prior art from @PLN6 / @PLN51).

**Question.** As loft matures toward the lavition engine deployment model, what is
the correct execution model for the combination of stable libraries and the user's
own scripts?  Should everything compile to native artifacts, everything stay
interpreted, or should the two coexist?

**Evaluation.**

The split model — native for stable/published libraries, interpreted for the
user's active script — has a decisive set of advantages in loft's specific context:

- **Best of both worlds.** Native gives speed for heavy, stable code (libraries,
  engine); the interpreter gives fast iteration (no `rustc` per save) for the code
  under active edit.  This IS the lavition model: native engine + libraries,
  interpreted game scripts for rapid prototyping.
- **The shared-ABI advantage is unique to loft.**  The store / `DbRef` heap is
  already a shared ABI between the interpreter and native code — data crosses the
  boundary as `DbRef`s into the same `Stores`, with no marshalling or copying.
  This is the hard part in Python/JNI/FFI; loft gets it for free because
  both modes address the same `Stores` instance.
- **The mixed-mode dispatch primitive already exists.**  `OpStaticCall` →
  `library_names` → `extensions::wire_native_fns` → `try_dlsym` (and
  `native_packages` / `src/extensions.rs`) implements exactly this model today:
  interpreted bytecode calls compiled Rust via the same `#native` / `#rust`
  mechanism the stdlib uses.  This is not a new idea — it IS the current stdlib
  design, extended to user libraries.
- **The native backend's cross-mode byte-identical equivalence** (the @PLN11
  harness) guarantees a library behaves identically whether run interpreted
  (during development) or native (shipped).

**Performance implication — supersedes E2 / full zero-copy as the startup-perf
endgame (measured 2026-06-04).** The `bench_read_data_breakdown` profiling
(PERFORMANCE.md § Open work, E2 row) found warm-load cost is allocation-bound:
it is the materialisation of library bodies + variable tables into native
`String` / `Box<Type>`.  In the native-library model you NEVER materialise those
(libraries are native); you load only the small library interface (type schema +
function signatures + symbol map).  The allocation cost is AVOIDED, not
eliminated via a `VH` zero-copy rewrite.  E2 / full zero-copy therefore
drops to low priority / deferred-by-this-decision for perf purposes.  The store-IR
foundation and the `IrNode` handle still have architectural and self-hosting value;
the perf rationale for E2 is superseded.

**Cache design (local scope — 2026-06-04 decision).**

- **Purely local workspace for now** — no registry/per-target distribution, no
  WASM concerns.  First-use compile latency is accepted.
- Cache native artifacts **per library** (max reuse; native `init()`-sequencing
  composes independently-compiled libs at load — this is why per-library native
  artifacts work where per-library IR snapshots did NOT; see C70).
- Eviction = **idle-TTL with touch-on-use**: update an entry's last-use timestamp
  on every cache hit; a startup GC sweep deletes entries idle longer than
  `LOFT_CACHE_TTL_HOURS` (default 24 h).  The current
  `cache::prune_program_cache` (oldest-first size-cap) keeps its role as a
  runaway-backstop; idle-TTL is the primary policy.

**The build fingerprint — the correctness crux.**  A native artifact is generated
Rust that `extern`-links `libloft.rlib`, so it is valid only against the exact
loft build whose rlib it links.  The cache key MUST fold the **loft rlib CONTENT
hash** (already memoised once per process in `native_utils::native_cache_key` per
the BUILD2 design — `src/native_utils.rs`) + rustc version + target + feature-set.
It must NOT key on git-HEAD `BUILD_ID` (does not change on an uncommitted loft
rebuild) NOR on mtime (fragile / over-invalidates).

The rlib hash is the load-bearing term: the artifact links the rlib, not the
executable.  Treat them as one "loft build fingerprint"; the rlib hash is the
correctness guarantee.

Two enforcement points ("do both"):

1. **Nuke-on-recompile** — a startup self-check compares the current loft build
   fingerprint against a stored marker; on mismatch it clears the native artifact
   cache (fast cleanup of rebuild orphans; the recompile happens via external
   tools — cargo/make — so loft can only react at its next startup).
2. **Fingerprint folded into every per-artifact cache key** (the lazy backstop).

Together these make `make rebuild-native-cdylibs` obsolete.  Known offender to
migrate: the `@P341` native-PACKAGE rlib path still folds **mtime** (per
PERFORMANCE.md § BUILD2 notes) — that is the hole behind hitting the
"generated rust-code error" too often.

**Three-layer model.**

| Layer | Mechanism | Goal |
|---|---|---|
| rlib-hash in each artifact key | cache key invalidation | correctness — never link a stale artifact |
| nuke-on-recompile (startup marker check) | cache sweep | fast cleanup of rebuild orphans |
| idle-TTL GC (24 h, touch-on-use) | background eviction | space — genuinely-unused sets age out |

**Validation layer / developer-vs-customer framing.**  The build fingerprint is
eventually owned by a library validation layer: an artifact's validity =
content-hash · target · features · loft-build-fingerprint · (eventually)
signature.  The cache then becomes dumb storage + the idle-TTL janitor.  This
fingerprint serves different audiences on different timelines: loft DEVELOPERS
need it now (rlib changes constantly, often uncommitted → git-HEAD useless);
customers on RELEASES are covered today by `LOFT_VERSION`; customers on DAILY
BUILDS will need the fingerprint (same/rolling version but different codegen →
`LOFT_VERSION` fails).  Building it now for developers IS the
customer/daily-build mechanism.  Dovetails with reproducible builds (BUILD2
confirmed the rlib is byte-deterministic — `src/native_utils.rs`).

**Remaining risks and open points.**

- **Dispatch coverage** is the real engineering risk: simple `#native` functions
  work, but generics, closures crossing the boundary, and complex exported types
  are hard (see PACKAGES.md "What must be native" / the C-ABI boundary).  Needs a
  coverage pass.
- **Dev-interpret fallback**: a library under active edit should still interpret
  (no `rustc` per save); "always native" = once the library is stable/published.
- **WASM/browser is a different model** — no `rustc` at runtime, so `--html` stays
  whole-program AOT-to-WASM; native libs don't apply there.

**Sequencing.** Tactical now (developer-facing, local): per-library native artifact
cache with rlib-hash key + nuke trigger + idle-TTL; extract a single
`loft_build_fingerprint()` reusing BUILD2's memoised rlib hash
(`native_utils::native_cache_key`, `src/native_utils.rs`) so it has a clean seam
to move into the validation layer.  First concrete step: audit every
native-artifact cache key for mtime/git-HEAD usage (`@P341` is the known offender)
+ extract `loft_build_fingerprint()`.  Eventually (customer-facing): fold the
fingerprint into the library validation layer; becomes load-bearing when daily
builds ship.

**Goal alignment ([GOALS.md](GOALS.md)) — and the guardrails this decision carries.**

C71 is the **Purpose made concrete**: *do the hard plumbing so it's fun to pick
up* — libraries **done** (compiled, fast, stable) so the script is **written**
(interpreted, instant iteration, no `rustc` per edit).  The **shared store /
`DbRef` heap is the structural reason it can be goal-true**: one heap means one
memory model and zero-marshalling dispatch, which is what lets it satisfy E and F
*across* the boundary instead of bolting on a second runtime.

Per goal, in brief:

- **F (friction-free): aligned, with a hard constraint.**  The native-vs-interpret
  choice must be *automatic and invisible* (never a user annotation), and
  un-native-able code (generics / boundary-crossing closures) must **silently fall
  back to interpret, never error** — F's "absorb the cost, don't hand the user a
  form."  First-use compile latency is a fun-on-pickup cost (accepted), not an F
  violation (F is syntax/proof friction, not wait-time).
- **E (predictable memory): aligned *because of* the shared store.**  No separate
  native heap → a value's lifetime is its scope whether allocated by interpreted
  or native code (post-rc-removal, both free at scope end).  Constraint: the
  boundary must not grow an "except across a native call" rule — `LOFT_STORE_GUARD`
  must cover the mixed path.
- **A (soundness): both an instrument and a surface expansion.**  The build
  fingerprint is itself a Goal-A veil-lifter — it forbids the "stale artifact
  silently links after a build / rustc bump" failure A is *defined against*.  But
  C71 puts more, less-battle-tested native code across the interpret↔native
  boundary, so the sanitizer (Miri / ASan / `stack_align_guard`, esp. macOS-ARM
  alignment) must grow a leg for the *mixed* boundary — it **adds to the soundness
  floor, it does not clear it**.
- **D (parity): relies on it, adds a row.**  Built on the cross-mode byte-identical
  equivalence, but introduces a fourth combination — interp-script + native-lib
  must match *both* all-interp and all-native; the differential sweep must assert
  the mixed run agrees.  (WASM excluded — stays whole-program AOT, consistent with
  D treating backends as independent implementations of one semantics.)
- **C / B:** C (dogfood) strongly served — fast libs + fast script iteration is the
  dogfood ideal — but **gated on the dispatch-coverage gap** (the consumers use
  closures / generics).  B compatible and *enables* the daily-builds cadence (the
  fingerprint makes daily updates safe for customers).

**Guardrails (must hold, not later polish):**

1. **Sequencing vs the two floors.**  C71 is capability work, which GOALS.md gates
   on the soundness floor — and C71 *enlarges* that floor with the mixed boundary.
   So grow the A / D detectors to the mixed boundary **as part of** the work, not
   after; do not let C71 run ahead of the soundness floor it extends.
2. **F line:** the native/interpret decision stays automatic + silent-interpret
   fallback — zero user-facing surface, no "can't compile this native" error.
3. **E line:** the boundary stays memory-model-transparent; `LOFT_STORE_GUARD` is
   extended to the mixed path and the exceptionless rule holds.

**Decision.** **Native libraries compile once (cached per rlib-hash fingerprint),
user scripts interpret.** This is the steady-state execution model loft optimises
toward.  Dated 2026-06-04.

Full narrative, risks, and sequencing: [BROADENING.md § Native-library execution
model](BROADENING.md#native-library-execution-model--the-steady-state-design).

**Status (2026-06-04).**  The dispatch **mechanism is proven end-to-end** — an
interpreted script calling an auto-generated, auto-compiled cdylib over the shared
`*mut Stores` (zero-marshalling), across **all common types both directions**
(scalars, vectors, structs, text, plain+data enums, keyed `sorted`), plus the
source-form lean interface and the `use`-shaped core (`mark_native_exports` →
`build_shared_cdylib` → `wire_shared_native_fns`, a normal library fn dispatched
with no `#native`).  See [@PLN11 Arc N](plans/11-data-as-store/README.md#arc-n--native-library-execution-model-c71-build-out)
and its **§ Landing sequence** for the ordered, landable path from here to the
steady state (A: wire `use`; B: make it invisible; C: soundness sweep; D: polish).

**Revisit when.** The dispatch-coverage audit reveals that the C-ABI boundary
cannot express a critical class of library API without marshalling cost that
erases the native-speed advantage — and a concrete measurement shows this matters
for a real consumer.  "WASM needs native libs" is not a trigger (WASM is a
separate model, always AOT).

---

## C72 — REPL session resume does not persist RNG generator state

**Catalogue:** @F49 (REPL), @F43 (random numbers).

**Question.** Should REPL auto-resume snapshot and restore the random
generator's internal state, so the random stream continues identically across a
stop/resume (exact-deterministic resume)?

**Evaluation.** Restoring saved RNG state makes the stream reproducible from the
saved image: anyone who reads the session file could predict or replay future
`random()` outputs, and a restored state re-issues values that callers assumed
were fresh.  That is a predictability / security hazard for any use of
randomness (tokens, nonces, shuffles), paid for a feature already covered
another way — users who want a reproducible stream set an explicit seed
(`random_seed`).  Drawn random *values* already restore exactly via the store
image (they are ordinary stored values); only the generator's forward position
is at issue.  The PCG state also lives in the `random` cdylib, not a store
(src/ops.rs), so persisting it would be added surface, not a free side effect.

**Decision.** **Closed — declined (2026-06-08).**  Resume restores stored values
verbatim but does NOT persist or restore RNG generator state; on resume the
generator continues fresh (re-seeded from entropy, as on any launch).
Deterministic streams stay an explicit-seed opt-in.  This also keeps the session
image free of generator state, narrowing what a saved image exposes.

**Revisit when.** A concrete, non-security use case needs byte-identical RNG
continuation across resume that an explicit seed cannot satisfy.

---

## C73 — `boolean` is three-state (false / true / null); `==` is raw, truthiness coerces

**Catalogue:** @F3 (scalar types — boolean), @F1 (null model).

**Question.** `boolean` was the only common-value scalar whose zero-value collided with
its null sentinel (null *was* `false`), so a nullable boolean couldn't distinguish
"absent" from "false" — unlike `integer` (0 ≠ `i64::MIN`), `float`, `text`, plain `enum`.
A `hash → boolean` map couldn't express absent / false / true.  Should boolean become
three-state, and if so, what are the comparison and coalescing semantics?

**Evaluation.** A boolean is stored in one byte — the same storage class as a 2-variant
plain enum, and plain enums already reserve byte `255` for null.  So the third state has
room for free; boolean was the lone byte-scalar flattening to 2-state.  Two semantic
candidates for `==`:

- **A — raw compare** (chosen): `==`/`!=` compare the raw byte, so `null == false` is
  `false` and `b == null` is the dedicated null test.  Truthiness contexts
  (`if`/`while`/`!`/`&&`/`||`) coerce `null → false`.  This is **exactly what `integer`
  already does** (`0 == null` is `false`; `n == null` is the null test; `if n` treats
  null as falsy) — so it makes boolean *consistent* with every other type.
- **B — coerce in `==`** (rejected): `null == false` would be `true`, forcing `== null`
  to be a special raw test bolted onto a coercing `==`.  That introduces a *new*
  inconsistency (boolean `==` coerces, integer `==` doesn't) — the opposite of the goal.
  Evidence settled it: `0 == null` is `false` for integer, so A is the consistent choice.

**Decision.** **Three-state boolean, design A.**  Dated 2026-06-10 (@PLN17).
- Representation: `false`=0, `true`=1, `null`=255 (byte), held/distinguished everywhere a
  boolean lives — locals, params, returns, tuples, struct fields, vector/keyed elements.
  A `boolean not null` is 2-state.
- `==`/`!=` are **raw** (distinguish null); `b == null` is the null test; truthiness and
  `&&`/`||`/`!` **coerce** `null → false`; `??` / `?? return` work (null-check is `== 255`,
  not truthiness — so `false ?? x` keeps `false`).
- Native mirrors the interpreter via the `u8`(storage)/`bool`(expression) two-form split
  (like `text`'s String/Str); heap storage mirrors plain-enum byte storage.
- **Supersedes the #256 guard cluster** (which *rejected* `null`/`??`/`== null` on
  boolean because false was indistinguishable from null — no longer true).
- Cross-mode byte-identical on both backends; regression: `tests/scripts/292-pln17-three-state-boolean.loft`.
  Full design + decision history: [`plans/17-three-state-boolean/`](plans/17-three-state-boolean/README.md).

**Revisit when.** Never, barring a fundamental change to the in-band-sentinel memory
model.  (The non-boolean tail — the construction-vs-parse default for an *omitted* field —
is resolved for enums by @PLN116 below: a bare enum field has no zero value, so it is a
compile error rather than a silent zero-fill.  A scalar's zero-fill stays valid: `0` is a
real integer, unlike an enum's `0` which is null.)

---

## @PLN116 — the `x?` default-fallback operator + enum-field non-null soundness

**Decision.** (1) **Notation:** the default-fallback is postfix `?` (`x?`), tightest
precedence — not `?? _` or a named form.  loft has no exceptions / early-return-on-null, so
`x?` carries no hidden control flow (local, total, value-in-value-out), and `.` already
null-propagates (C80), so both neighbouring `?`-slots are vacant.  `??` lexes greedily over
`?`, so `a ?? b?` is `a ?? (b?)`.

(2) **One default predicate** (`has_default`) feeds BOTH `x?` and the `S{}` zero value —
there is never a second notion of "T's default".

(3) **A bare enum field in a record has no default.**  An enum's 0 is its null/undefined
value (variants are 1-based), so zero-filling a *non-null* enum field puts null into a
non-null slot — the very unsoundness the null model forbids elsewhere (a scalar's `0` is a
valid value; an enum's `0` is the *absence* of one).  So a bare (non-`Optional`) enum field
with no `= expr` leaves the record with **no default**: `x?` on it and `S{}` omitting it are
BOTH compile errors.  A *bare* enum still discharges to its first variant (`x?` on `E?`); a
genuinely `Optional` enum field (`Color?`) defaults `null`.

**Why now (pre-1).** This is a contract-1 soundness fix: after =1, compatibility freezes it
forever.  Blast radius was tiny — three in-repo defaults where the old zero-fill was relied
on (`File.format = NotExists`, `Lexer.scanned = Unknown`, `Definition.structure =
Function`), each overwritten before any read.

**Revisit when.** The marked-default-variant marker (let an enum nominate its own default
so a bare field may default to it) lands — an additive extension, not a change to this rule.

Full design + closure record: [`plans/116-default-fallback-operator/`](plans/116-default-fallback-operator/README.md).

---

## Adding a new entry

When closing a question, append a new `##` section using the
format above.  Follow with a one-line pointer from the source
document's "Out of scope" table:

```markdown
| CXX | Title | Closed — see [DESIGN_DECISIONS.md § CXX](DESIGN_DECISIONS.md#cxx) |
```

Do not move the question itself out of the source doc's history.
Strike it (`~~…~~`) and point at this register.  That keeps the
original context discoverable from git blame / git log without
cluttering active tables.

## C74 — A mutated scalar may be captured by only ONE closure

**Catalogue:** @F22 (closures & lambdas).

**Question.** Should a bare scalar local that one closure mutates be
sharable with other closures (`run2(fn() { print(t) },
fn() { t = t + 1 })`) — JS/Python-style shared upvalues?

**Evaluation.** The shape only works through shared heap cells
(`__cell_<T>`) referenced by several closure records at once, and the
store model gives that sharing no defined owner: plan-57 removed the
ref-count, so `free_named` frees the cell at the FIRST record's death
and silently no-ops for the rest ("first death wins") — a latent
use-after-free class whenever one sharer dies early.  The parse-side
is equally fragile: closure-record attribute types freeze at each
lambda's pass-1 epilogue while the boxing decision (`scalars_to_box`)
accumulates until the parent's body end, so a reader lambda parsed
before the writer baked in the unboxed layout and crashed at runtime
(#314, interp CONST_STORE panic / native codegen failure).  No
consumer needs the shape: the @PLN18 kernel that surfaced it
immediately preferred a struct (`w.n`), which also makes the sharing
visible at every use site.  First worked example of
[GOALS.md § "Stability trumps features"](GOALS.md#stability-trumps-features).

**Decision.** **Closed — rejected at compile time.**  Dated
2026-06-10.  When more than one closure captures a scalar that any
closure mutates, the parser reports *"sharing a mutable variable
between closures is not supported — hold the shared state in a struct
field instead"* (`Parser::reject_shared_mutable_scalar_captures`,
`src/parser/vectors.rs`; chokepoint: the parent's pass-1 body end,
where `scalars_to_box` is final).  Kept as supported: the
single-closure accumulator (`fn() { sum = sum + x }` — one record,
one owner, sound), read-only sharing of a scalar across any number of
closures, and struct-field state captured by any number of closures.
Supersedes the C38 plan-22 addendum sentence "the outer scope and all
closures share the same cell" for the N≥2 case.  Regression guards:
`tests/issues.rs::issue_314_scalar_shared_by_two_closures_rejected`
and `::issue_314_single_closure_accumulator_still_works`.

**Revisit when.** A real consumer presents a shape that is materially
clumsier as a struct AND the cell gets a single defined owner first
(e.g. the parent frame owns the cell and records never cascade-free
it).  This is a default to keep in mind, not a hard line — reevaluate
on evidence.

## C75 — Closure-carrying struct values are frame-bound

**Catalogue:** @F22 (closures & lambdas).

**Question.** May a struct holding a capturing closure leave the function
whose frame owns the captures — be returned, written into an argument's
field, or stored in a collection?

**Evaluation.** The closure record holds raw 12-byte DbRefs into the
constructing frame's stores (Reference captures and `__cell` scalar boxes
alike).  Every escape route copies the record's bytes — `OpClaimChildRec`
clones the DbRefs but nothing transfers the stores they name — so the frame
frees them at return and the free-bitmap hands the slots to the next
allocation: the escaped closure then silently reads and writes an unrelated
live object (#318; probed matrix in the issue, probes in
`/tmp/p_followups/e*.loft`).  Within-frame use — locals, downward argument
passing, #313's whole matrix — is sound.  A real ownership transfer through
the deep copy is a substrate design (cross-store fix-ups, native mirroring);
no consumer needs the escape today.

**Decision.** **Closed — rejected at compile time.**  Dated 2026-06-10.
Three sinks reject on the transitive `Parser::type_carries_closure`
predicate (derived from the registered DB layout, order-stable): returning a
closure-carrying struct type (`definitions.rs`, the pass-2 body hook),
writing a capturing closure into a struct rooted at an argument
(`set_field_check`'s fn arm), and collections of closure-carrying structs
(`sub_type` — extends the plan-15 CLOSED `vector<capturing fn>` cell, which
a struct wrapper had silently bypassed).  Struct assignment is copy-at-value
(C38), so a local alias cannot smuggle a write past the argument check
(probe e10).  Returning a BARE capturing closure stays supported on interp
(case-C factory transfer); its native divergence is #323.  Regression
guards: `tests/issues.rs::issue_318_*`.

**Revisit when.** A consumer needs factory-built closure-holding structs AND
the deep-copy path gets a designed ownership transfer (claim the captured
stores into the host, or re-point the record at host-owned copies) —
verified on both backends.

## C76 — Selective imports group with `()`, not Rust-style `{}`; flat comma list dropped

**Catalogue:** @F47 (library imports / module system).

**Question.** How does a `use` import multiple names from one library — a flat
top-level comma list (`use lib::a, b, c;`), Rust-style braces
(`use lib::{a, b, c};`), or parentheses (`use lib::(a, b, c);`)?

**Evaluation.** The flat list reads poorly — `b`/`c` don't visually bind to
`lib::`, and at a glance look like separate statements.  `{}` is loft's
block/struct-literal delimiter; reusing it for imports invites confusion with
struct construction and a future `use lib::{ … }` block.  `()` is loft's existing
arg-list/grouping delimiter, reads as "these names belong to `lib::`", and is
lighter than braces.  Per-name `as` aliasing (@PLN22 Phase 3) composes inside any
of the three.

**Decision.** **Grouped `use lib::(a [as x], b, c);` (@PLN22 Phase 4, 2026-06-14).**
A single `use lib::name [as bind];` is unchanged; multiple names MUST be
parenthesised; the flat `use lib::a, b;` list is a hard error ("import multiple
names with parentheses") that still binds the names (recovery).  Rust-style `{}`
braces are NOT adopted — `{}` stays reserved for blocks/structs.  Sole flat-list
site migrated; tests `imports::pln22_phase4_grouped_import` /
`pln22_phase4_flat_list_rejected`.

**Revisit when.** A concrete need arises for nested/path grouping that `()` can't
express (e.g. `use a::(b::c, d)`), with a parse that doesn't collide with the
struct-literal or call grammar.

## C77 — Binding ownership: heap aliases by default; `&` binds a live reference

**Catalogue:** @F21 (references `&T`).

> **CORRECTED (2026-06-23, @PLN87).** The "`&` makes a *reassignment write back*" reading
> below is superseded: **`&` binds a live REFERENCE** (read- and write-through to an
> addressable source), not a reassignment annotation, and it is a binding marker — not a
> general operator. Heap-aliases-by-default still holds. See
> [OWNERSHIP_MODEL.md § The law](OWNERSHIP_MODEL.md) and
> [plans/87-reference-default-binding.md](plans/87-reference-default-binding.md).

**Question.** When `a = x` / `a = x.f` / `a = x.v[i]` binds from a value backed by
another store, is the binding a COPY (independent value), a VIEW (alias), or chosen
per binding *form*? loft today does all three by form — whole-value eager copy, the
#415 struct-field copy-on-bind, the `a = x.v[i]` element view — the copy-vs-view
inconsistency #426 surfaced.

**Evaluation.** Three candidate invariants:
- *View-by-default* (status quo): element/field reads alias, whole-value copies. The
  `=` ambiguity is permanent and non-local — "is this a copy?" can't be read off the
  line — and the split manufactures the store-lifetime bug class (Cluster A, #415, #426).
- *Copy-always*: uniform but pessimal (eager copies everywhere) and still cannot
  express write-through.
- **Value-semantics by default, copy/share/move chosen by a path-sensitive
  liveness+mutation analysis** ([OWNERSHIP_MODEL.md § The law](OWNERSHIP_MODEL.md)):
  observably every binding is an independent value; the compiler *shares* while no
  aliasing write is possible and *moves* when the source is dead. Uniform across all
  forms — the source is just a path expression.

**Decision — REVISED to reference-default + `&`-to-reassign (2026-06-22).** An initial
value-semantics direction was reconsidered against loft's *actual* behaviour (verified
both backends): heap values are **aliased/shared by default** — a binding or param to a
struct/vector aliases the source, and in-place field/element mutation (`o.field = x`,
`o.v[i] = y`, `a = vv[0]; a[i] = z`) writes through. So `a = vv[0]` is a **view**, and
**#426 A/C are correct as-is** (not bugs). The ONE change: a non-`&` **whole-binding
reassignment** (`o = Obj{...}`) becomes a *local rebind* (today it overwrites the source
in place); **`&` makes the reassignment write back** to the source — the *same* `&`
notation loft already uses for `&vector<T>` parameters, now at a local binding. So `&`
has one uniform meaning — *"reassigning writes back"* — load-bearing **only** when the
body reassigns the binding; a `&` on a struct that merely mutates fields is **redundant**
→ the **W4 redundant-`&` lint** (it fixes the recurring '`&Object` is needed to mutate an
object' confusion — `&` is needed to *replace* one, not mutate it). No lifetime
annotations (the borrow checker infers source-outlives-binding from scope, as it already
does for `&` params — C38's objection was to reference *types*, not this binding
*notation*). This is **smaller than full value-semantics** (no copy-on-write, no p379
rewrite — p379's field mutation already writes through) and is the concrete content of
the OWNERSHIP_MODEL beacon. Consistent with C64 (tuple struct-ref elements already use
MOVE). See [OWNERSHIP_MODEL.md § The law](OWNERSHIP_MODEL.md).

**Revisit when.** The reference-default aliasing proves a net footgun — a real consumer
is repeatedly bitten by a field/element write propagating through a view it did not
intend to alias, and the cost of those bugs exceeds the value-semantics migration
(copy-on-write + `&` on every alias) it would take to remove them. Only then reconsider
value-semantics-by-default; until then the W4 lint + the documented view default are the
cheaper guard.

---

## C78 — The Rust-engine ↔ loft-library boundary: mechanism not genre, and no black boxes above the engine

**Question.** When something is *hard* and would be *reused* — a world model, gameplay
primitives, rendering composition, a streaming substrate — should it move down into the
**Rust engine** (shipped with the compiler: paid once, fast, opaque), or stay up as a
**loft library** (written in loft, slower to write, readable)? Stated the other way:
what is the Rust core *allowed* to swallow?

**Evaluation.** Three forces act on the line, and they do not all pull the same way:

1. **Cost (paid-once)** — *pulls down.* Complexity is conserved: building reliable
   servers and efficient cross-platform games is hard by nature, and you can't delete
   that, only relocate it. The only question is who pays it and how many times. The Rust
   engine pays the hard-and-universal complexity **once**; every loft program above it
   pays zero, instead of re-writing it over and over. This is "easily" (Goal F) achieved
   by *relocating* complexity, not by pretending none exists.
2. **Neutrality (mechanism vs policy)** — *pulls up.* The compiler must encode **no
   worldview.** A genre baked into the compiler stops being a *choice* and becomes a
   *tax everyone pays*: a side-scroller author should not carry one byte of a hex world.
   So an opinionated game-type model is **policy** and stays a library, even though it is
   hard (force 1) and would be reused by every world-game author.
3. **Transparency (no black boxes above the engine)** — *pulls up.* A game developer
   must be able to **read** the library, **learn** how it works, and **fork** its
   behaviour and primitives for their own game. The moment code goes into Rust it is a
   black box to them — a different language, needing Rust skill and an engine recompile
   to touch. So "should a developer ever want to open this?" pulls game-facing code up
   into loft regardless of how hard it is.

Forces 2 and 3 **override** force 1. The naive test "hard-and-universal → Rust" is
wrong; the correct test is tighter: **universal AND something a game developer should
never need to read.** That narrows the Rust core to its minimal opaque floor.

**Decision.** **Closed — accepted architectural principle.** Dated 2026-06-23. Four
load-bearing rules:

1. **The compiler ships mechanism, never a genre.** Opinionated game-type models (the
   hex-based streaming world is the flagship case) stay **loft libraries**, never baked
   into the compiler — so they are *optional* (don't `use` it → not limited by it),
   *replaceable* (write your own side-scroller library), and *unprivileged* (the
   author's own flagship gets no special compiler status; it is just another `use`,
   on equal footing with a stranger's library). This is "adoption is a result, not a
   steering input" applied to architecture: the shared floor is not bent toward the one
   game that exists yet.
2. **The Rust/loft line is "should a game developer ever need to open this?"** — *No*
   (codegen, the type checker, the store allocator, memory management) → Rust: paid once,
   opaque, fine. *Yes, ever* (the world model, gameplay primitives, rendering
   composition) → loft, **even when hard**. "Hard" is not what sends code into Rust;
   "the developer never opens it" is.
3. **Libraries are kits of composable primitives, not sealed monoliths.** "Change the
   behaviour and primitives for their own use" only holds if a developer can lift **one**
   primitive (the streaming, the chunk loader, the spatial query) and recombine it
   without forking the whole library. Coming-apart-cleanly into visible primitives is
   part of the library's contract, not a nicety — and it is harder to write than a
   monolith.
4. **The engine is "unlearnable from above" — a property to *defend*, not merely
   observe.** Learnability is relative to (code, audience): the engine is correctly
   **below the game developer's horizon** (the relief — "paid once so you can forget
   about it"; *forget about* literally means *don't have to learn*), while being **fully
   learnable to an engine contributor** (what the whole `doc/claude/` corpus + the
   matrix-first method exist for). Opaque is only safe because **dependable** — you can
   only afford to not-learn what won't surprise you, so this rule rests on Goal A
   (soundness) and Goal E (predictable memory). The failure mode is the engine *leaking
   upward* and forcing itself to be learned: every time a developer must understand a
   store-lifetime quirk to get their game running (the crawler survival guide,
   [loft#248](https://github.com/loft-lang/loft/issues/248)) the engine has become
   *involuntarily* learnable — exactly the [STRONG_POINTS.md](STRONG_POINTS.md) #3/#4
   turn-offs. "Not learnable from above" is the goal; "the dev had to learn it anyway"
   is a bug.

**Anti-cage.** This is [GOALS.md § Purpose](GOALS.md#purpose--what-loft-is-for)'s "win
the dependability *without the cage*" stated as a boundary. The AS/400's engine bought
its reliability as a *closed* box — opaque to everyone. loft's engine is closed only to
the developer's *concern*, and open to anyone who wants to work on loft itself; everything
above it is loft source the developer can read, learn from, and fork.

**Not in tension with C71.** [C71](#c71--native-libraries-compile-scripts-interpret--the-steady-state-execution-model)
(native libraries compile, scripts interpret) is about *how a loft library executes*
(native vs interpret); C78 is about *where code lives* (Rust engine vs loft library).
They are orthogonal: a loft library may be native-compiled for speed (C71) and still be
transparent loft *source* the developer reads and forks (C78) — the native artifact is a
derived, invisible cache, and the loft source stays the truth (cf. [C70](#c70--no-per-library-ir-snapshot--cache):
"the loft source is the better representation of a library's state").

### Corollary — the move-to-Rust alarm (added 2026-06-23)

Needing to push something *out of a current loft library and into Rust* should feel
like a **failure of language design** — and that feeling is **load-bearing, kept on
purpose.** It is the conscience that holds the Rust core minimal: a builder who felt
*fine* pushing code into Rust would let the opaque floor metastasize one "just drop it
to Rust" at a time, and transparency (rules 3–4) would erode silently. The discomfort is
the forcing function that grows *loft* instead of the engine.

Aim it precisely: the failure is not "code moved to Rust," it is **"code moved to Rust
because loft couldn't express or run it."** Two reasons a library reaches for Rust, and
only the first is the failure —

1. **loft couldn't do it.** The code *is* game-facing, but the language wasn't expressive
   or fast enough. *This* is the alarm's true target. The disciplined response is the
   dogfood loop: treat the library's pull toward Rust as a **bug report against the
   language** and fix loft (add the expressiveness, make the slow primitive fast) — never
   let the library escape downward. ("Find the old conservative mechanism and narrow it,"
   turned on loft's own surface.)
2. **It was never game-facing code.** It is genre-neutral *mechanism* that started in a
   library only because that is the cheapest place to prototype. Relocating it down is
   rule-1/rule-2's boundary *correcting itself*, not loft failing — relocate without
   shame. The streaming substrate (see Revisit-when) is this case.

The test that separates them is C78's own: *would a developer ever want to open this?*
**Yes** + dropping it to Rust → the failure; fix the language. **No** → engine mechanism
wearing a library's clothes; moving it is housekeeping. Even then there is a gentler step
before Rust: neutral-but-readable mechanism can sink into a **lower loft library** and
stay transparent; it earns *Rust* only when it is *also* never-opened **and** needs native
speed. So the alarm should ring loudest at a jump *straight from game-facing loft to
Rust* — that one is almost always the failure case, not the correction.

**Revisit when.** A concrete consumer need shows a *genre-neutral mechanism* currently
in a loft library is a real, measured bottleneck only the Rust engine can fix, **AND**
moving it down buries neither a worldview choice (rule 1) nor a primitive a developer
needs to read/fork (rule 3) — bring the profile and the boundary analysis together. The
likely live case is the **streaming substrate**: loading/unloading spatial chunks from
the store as the player moves is genre-neutral (a side-scroller and an open world both
want it), so it may rightly sink into the engine or a lower neutral library *while the
hex tiling stays up* in the world library on top of it. Note: "it's hard" or "it'd be
reused" **alone is not sufficient** — that is force 1, which forces 2 and 3 override.

---

## C79 — Ownership is internal; no user-facing borrow checker

**Catalogue:** @F21 (references `&T`).

### Question

loft's ownership/`deps` system is described as "loft's borrow checker, Rust as the reference
model" ([OWNERSHIP_MODEL.md](OWNERSHIP_MODEL.md)). Should that surface to the programmer —
compile errors for ambiguous/unsafe aliasing and lifetimes, à la Rust — or stay entirely
internal (the compiler always finds a valid lowering, never rejecting)?

### Evaluation

A user-facing borrow checker gives a simpler, more predictable compiler (it *checks*
annotations rather than *solving*) and zero surprise copies — but it imports Rust's #1
learning hurdle into a rapid-prototyping scripting language whose stated aim
([GOALS.md](GOALS.md)) is *fun on pickup* and *the most natural solution for the programmer*.
An internal system keeps the surface clean (write naively, it works) at the cost of a harder
compiler obligation: the analysis must be **total** (never stuck), copying when it cannot
prove an alias is safe.

### Decision

**Closed (2026-06-24) — INTERNAL only.** No user-facing ownership errors. The compiler
always produces a correct free/copy/move, copying when unsure; the one deliberate user-facing
ownership concept is `&` (a live reference, opt-in shared mutation —
[OWNERSHIP_MODEL.md § The law](OWNERSHIP_MODEL.md), @PLN87). "Rust as the reference model"
means **soundness of the internal analysis**, not Rust's UX. This was always the plan.
Consequence: `O-Complete` ([formal/ownership.md](formal/ownership.md)) is the load-bearing
invariant — an incomplete fact is a miscompile / leak, not a recoverable compile error, so the
failure to fear is *incompleteness*, not just unsoundness.

**Revisited (2026-08-05) — loft may DECLINE what it cannot implement safely.** The revisit
clause below fired, and the maker widened it past *"a single case"*:

> "We can decline code where we cannot create a safe implementation. If a user explicitly takes
> a reference that is an ownership decision and it has consequences."

So the boundary is no longer *one named diagnostic* but a **principle with a precondition**:
where loft cannot produce a lowering that honours what the author wrote, it REFUSES the program
rather than silently substituting something else. The half that does not move is the important
one — loft still never rejects a program it *can* compile correctly, there are still no lifetime
annotations, no move-vs-borrow puzzles, no "cannot borrow" on ordinary naive code, and
`O-Complete` is untouched: the analysis must still be total wherever a valid lowering exists.

What changed is the answer when one does not. Writing `&` is not a performance hint, it is an
**ownership decision**: it asks for a live link, and B-Ref-Alias promises that every write
through it reaches the source. When the program then disturbs the place that reference names —
removes from the container, re-keys the element, replaces the container — there are exactly
three possible answers. Honour it with per-reference runtime arithmetic (declined: not worth it
for an edge case). Silently downgrade the reference to a copy (**this is what the principle
forbids** — it makes the author's explicit decision a lie, and it loses a write with no
diagnostic). Or decline the program. loft declines.

**Why an error and not "some defined behaviour" — the maker's reason, and it generalises.**
2026-08-05: *"If we cannot fulfil what a programmer asks for us that should be an error instead
of silently different semantics … otherwise we will have to support this seemingly
undefined/strange behaviour into our future, even if we might want to introduce logic that makes
an implementation possible for these cases."*

That is a **forward-compatibility** argument, and it is the one that makes this a general method
rather than a one-off. [COMPATIBILITY.md § The error surface is one-directional](COMPATIBILITY.md)
says loft may always DROP an error and, after the freeze, may never ADD one. So the two options
are not symmetric:

- **ship the silent copy** → that semantics is in the contract forever. If loft later builds the
  machinery to honour the reference properly (re-pointing the link, re-inserting the re-keyed
  record), it cannot switch to it: programs would silently change meaning. The workaround becomes
  permanent, and it is a workaround nobody chose;
- **ship the error** → nothing depends on it, because nothing compiles. The day a safe
  implementation exists, dropping the error is a pure widening — every program that compiled
  still compiles, and the ones that did not start working.

So the error is the option that **keeps the door open**. This is how loft resolves a consistency
problem in general: where two plausible behaviours exist and neither is clearly right, refusing
is the choice that does not commit the language to the wrong one.

First application: **B-Ref-Reshape** ([formal/binding.md](formal/binding.md)), @PLN130 F9 /
[loft#779](https://github.com/loft-lang/loft/issues/779) — all three disturbances (removal,
re-key, container reassignment).

### Revisit when

A concrete consumer hits a case where silently copying is a real, measured cost AND a narrow,
*clearly-diagnosable* surface (e.g. "this reference would outlive its source") would be more
natural than the copy — i.e. one named case earns a user-facing diagnostic. Even then: a
single case, never the general Rust model. **(Fired 2026-08-05 — see the revisit above, which
generalised "one named case" to "wherever no safe implementation exists". The clause is kept
because its bar is still the right one for any FURTHER widening: a measured cost and a narrow,
clearly-diagnosable surface, never the general Rust model.)**

---

## C80 — The spreadsheet fault model: nothing stops a running calculation

**Catalogue:** @F38 (arithmetic safety), @F1 (null model), @F44 (logging — panic/assert).

### Question

When a runtime fault occurs mid-execution — a calculation that can't produce a value
(`s / 0`, an overflow it can't represent, `v[99]` out of bounds, a deref of an absent value),
or an explicit `panic("msg")` / `assert(cond, msg)` — should the program stop (halt the run /
skip the rest), as C66's *development* mode does, or keep running?

### Evaluation

It mimics a **spreadsheet**: one cell with a bad formula shows an error in *that* cell and
never stops the other cells from recalculating. That is what normal programmers expect of a
robust system, and it is the most natural mental model — far more so than "one bad value
anywhere aborts everything after it." In a game loop it is the difference between *one mob
behaving strangely* and *the whole world freezing because one mob did*. The visible signal is
the **null itself** (you see it in the result); there is **no per-fault log by default** — in a
spreadsheet most empty cells are normal, so logging each uncomputable would be pure spam. A
programmer who wants to trace *where* uncomputables arise can opt into a **debug log level**.
This is the same through-line as [C79](#c79--ownership-is-internal-no-user-facing-borrow-checker)
(ownership) and the differential-oracle decision: the safety lives *under* a surface that keeps
going.

This **deliberately deviates from the norm** (most languages abort on a fault) — accepted on
purpose, because it directly drives the **fun** of building a game ([GOALS.md](GOALS.md)). A
developer iterating on something half-working keeps seeing it **run** — one broken formula
doesn't blank the screen, one bad mob doesn't freeze the world — instead of the
crash → read-trace → fix → rerun loop that kills creative flow. Here robustness is a
*creativity* feature, not only a safety one: you stay in the world, tweaking, while it keeps
moving.

It also corrects a common misread of the "safe language" promise. Rust's reputation — *"if it
compiles, it works"* — oversells what compilation buys: the borrow checker and type system
remove **memory** bugs and data races, but a *logic* fault (`unwrap` on `None`, an out-of-bounds
index, an overflow) still **panics — it halts at the first problem.** "It compiles" means "it
won't corrupt; it'll stop cleanly," **not** "it keeps working." loft's robustness is on a
*different axis*: not "prevent a class of bugs, then halt on the rest," but **keep running
*through* a fault** — degraded and local.

And this is **not specific to games.** A **server** that terminates on one bad request is an
outage; a **kernel** that stops is a dead machine. *Keep running* is the right default for any
long-running system — anywhere termination is the larger failure. The narrow exception is a
context where *acting* on a bad value is worse than stopping (a physical actuator) — and even
there the answer is an **explicit check at that boundary** (`?? safe`, validate-before-act),
not making the language halt globally. Keep-running plus a local guard beats a global stop.

And it beats the *other* norm too — **exceptions**. `try`/`catch`/`finally` does keep a program
running after a small error, but at a steep price: surviving the error means **unwinding the
stack** — heavy machinery to tear down frames and run cleanup, invoked for one bad value. Worse,
it is **error-prone**: a missed case in a `finally` corrupts state that was *fine without the
exception* — the recovery mechanism itself *introduces* the corruption. loft sidesteps both by
making a fault a **value, not a control-flow event**: the failed operation yields null *in
place*, execution continues linearly, **nothing unwinds**, and there are **no cleanup blocks to
get wrong**. Error handling becomes **data flow** (null propagates like any value), not control
flow — so it cannot leave the half-unwound, half-cleaned-up state that is exceptions' own
corruption surface.

### Decision

**Closed (2026-06-24) — NOTHING stops a running calculation. Revises C66.**

1. **Uncomputable → null, by default, silently.** `s / 0`, overflow, out-of-bounds index,
   deref of an absent value all yield **null** with no `??` needed. `??` means "give me a
   *non-null* fallback," not "rescue from a trap." (The trap discipline — "overflow traps, NOT
   a silent null" — is **reversed**.) **No per-fault log by default** — uncomputable values are
   common (the spreadsheet's empty cells), so logging each would be spam; a programmer who
   wants to trace them opts into a **debug log level**. (Refined at implementation — see the
   *Implemented* note below: an **unguarded** div-by-zero is the one exception, emitting a
   single Warn so an undefended fault is not invisible.)
2. **A calculation fault never halts the run or skips a later statement.** The script is a
   sequence of independent steps. Step A producing null does not abort; step B still
   **executes** (and gets null if it consumed A's null — null is contagious — but it *runs*).
   No step is skipped because something earlier "happened to be wrong."
3. **`panic` / `assert` are NOT calculations — they keep their C66 split.** They are explicit
   developer signals, so during **development and testing they still HALT** (you want a
   deliberate `assert(false)` / `panic(...)` to stop and show you, exactly as today). In
   **production** they **log and continue** — one assertion never takes down a running system.
   Only the *implicit* calculation faults of point 1 move to universal null-and-continue.
4. **Startup still stops** ([C67](#c67--fail-at-startup-not-at-runtime)): bad config /
   port-bind / missing deps exit *before* the run begins — "can't open the spreadsheet," not
   "a cell errored." A running calculation never stops.
5. **Implicit faults are MODE-INDEPENDENT — and tested via the debug log.** Points 1–2 hold
   **identically** in development, test, AND production: a div0 / overflow / OOB / deref yields
   null-and-continue the same way everywhere. There is **no dev-vs-production split for
   calculation faults** — only the explicit `panic`/`assert` signals (point 3) split by mode. A
   mode-dependent implicit-fault path is deliberately rejected: a fault that behaves one way in
   test and another in production is its own class of bugs ("works in test, degrades differently
   live"). Because these faults are **silent** by default (no per-fault log), the project builds
   **debug-level logging infrastructure** (normally invisible) that traces every uncomputable,
   and the **test suite enables that debug level to VALIDATE** that the expected faults fire and
   yield null. The debug log — not a halt — is how a calculation fault is *asserted* in a test;
   it replaces the old dev-halt as the observation mechanism, so a test can confirm "this divided
   by zero and produced null, and the rest still ran."

Robustness payoff: a system degrades **locally** (one value, one entity), never **globally**.

**Implemented (2026-06-24, formalize4).** Div/mod-by-zero and integer `+`/`-`/`*` overflow now
yield the null sentinel and CONTINUE on both backends (the interpreter and `--native`); OOB
already did, and a null deref never trapped — so the implicit-fault class is fully on the
spreadsheet model. Two refinements to point 1's "silently" came out of validation review:
(a) an **UNGUARDED** divide-by-zero emits **one** Warn log — the "no guard already" signal, so an
undefended fault is observable (it surfaces when a logger is attached, e.g. in tests; a default
CLI run with no logger is still quiet) — while a **guarded** site (`?? ` / a null-check) reports
nothing; (b) **overflow is silent** at every site (the null result is the signal — and silent
overflow is the rustc-release default; loft's is null rather than a wrapped wrong answer).
Specced in [formal/operational.md](formal/operational.md) (E-Uncomp + the E-Report logging rule);
deviation **D-op-4 closed**. The opt-in `--dev-soft-halt` debug flag still surfaces these
recoverable faults uniformly for one-shot triage — an explicit debug tool, not a mode split.

### Revisit when

A consumer shows that silent null-and-continue (plus the opt-in debug log) genuinely loses
critical information — e.g. a class of uncomputable that *should* be noisy by default. Even
then the fix is more **observability** (turning a debug log on by default, a louder surface),
never a halt.

---

## C81 — `&` stays one token, disambiguated by position (bitwise-and vs reference)

**Catalogue:** @F21 (references `&T`), @F37 (operators — bitwise `&`).

### Question

`&` does double duty: an **infix** `&` is bitwise-and (precedence level 6, `a & b`), a
**prefix** `&` is the reference-type annotation (`b = &a`, [binding.md](formal/binding.md)).
Should the grammar split them into two distinct tokens (a separate reference sigil), or keep
one `&` resolved by position? (formal/grammar.md deviation **D-gram-4**.)

### Evaluation

The two are told apart purely by **position**: a `&` with a left operand is infix bitwise-and;
a *leading* `&` is the reference annotation. @PLN87 plus A1 (binding.md D-bind-7) made a prefix
`&` a **parse error in every non-binding position** — assignment target, operand, argument,
collection element, condition, bare statement, block-final — so it can never reach an
expression slot. The disambiguation is therefore **total**, not heuristic. Rust makes the same
call (`&` is both reference and bitwise-and, disambiguated by position); a new sigil would add
surface for zero semantic gain and break the familiar reading. The coupling to binding.md
("prefix `&` only at a binding") is a *documentation* obligation — the rule is stated in both
the grammar and binding areas — not a soundness gap.

### Decision

**Closed (2026-06-24) — KEEP one `&` token, disambiguated by position.** The
prefix-`&`-only-at-a-binding rule (binding.md `B-Ref-AnnotationOnly`, enforced through
D-bind-7) is what makes the position rule total, so the overload is a **decided edge**, not a
deviation. D-gram-4 leaves formal/grammar.md.

### Revisit when

A concrete grammar/tooling need (a *generated* parser that cannot reproduce the positional
rule) hits an actual ambiguity — not merely the doc coupling.

---

## C82 — loft's surface is deliberately not context-free

### Question

loft's grammar resolves some constructs with **speculative backtracking** (type-vs-variable;
struct-init `S { … }` vs block `{ … }`) and **lexer modes** (string interpolation `"{e}"`), so
no context-free grammar accepts exactly loft. Should we pursue a CFG (so an external,
grammar-based tool could be derived), or accept the context-sensitive surface? (formal/grammar.md
deviation **D-gram-2**.)

### Evaluation

The context-sensitivity buys real ergonomics — the `S { … }` struct-literal shorthand and
format-string interpolation — that a CFG-clean grammar would force into clumsier syntax. No
consumer has needed a CFG: the hand-written two-pass recursive-descent parser **is** the spec,
and tooling (LSP, formatter, the doc viewer) reuses it rather than a generated parser. Chasing
a CFG would constrain the surface for a benefit nobody has asked for, and the backtracking is
bounded in practice (a couple of fixed lookahead/disambiguation points, documented in
[COMPILER.md](COMPILER.md)).

### Decision

**Closed (2026-06-24) — ACCEPT the non-context-free surface; do not chase a CFG.** The
hand-written parser is the grammar; formal/grammar.md states the one fact that used to live
only in code (precedence + associativity — D-gram-1, now lifted into LOFT.md) and records the
context-sensitive points as accepted. D-gram-2 leaves formal/grammar.md as a decided edge.

### Revisit when

A concrete consumer needs a CFG (a third-party grammar-based tool that genuinely cannot reuse
loft's parser) **and** the context-sensitive points can be expressed without unbounded
backtracking.

## C83 — The internal representation follows the user-visible contract; never widen storage for implementation convenience

**Catalogue:** @F3 (scalar types — integer), @F4 (width integers).

### Question

The user-visible type `integer` is **i64** (verified — see below). Should the internal
representation therefore widen to a uniform i64 — e.g. make `Value::Int` carry `i64`, and store
every integer field/element as 8 bytes — to "match the type" and simplify the code? More
generally: when a user-visible type is wide, is it acceptable to store it wide everywhere because
that is the easy, uniform choice?

### Evaluation

**The user-visible i64 contract is already met, at no cost to the representation.** A boundary
matrix confirms a value above i32 range survives **every** observable round-trip — arithmetic
(`*`, `/`, `%`, `-`), bare literals, struct fields, vector elements, function args/returns,
comparison, negation, tuples, and field mutation — **identically on the interpreter and
`--native`**. The runtime computes on `i64` throughout.

The internal storage uses a **compact value-size encoding** — `Value::Int(i32)` when the value
fits in 32 bits, `Value::Long(i64)` only when it doesn't — and narrow stored fields use the
smallest sufficient width. This is **deliberate**, not an accident waiting to be "fixed":

- A tree-walking interpreter and a word-addressed heap are **memory-bandwidth bound**. Doubling
  every integer IR node and every stored integer to 8 bytes — most of which hold small values —
  is a direct, measurable bandwidth tax for **zero user-visible benefit** (the contract is
  already i64).
- "Store it wide because it's uniform/easier" optimises the *implementation's* convenience at the
  *user's* expense. The dependency must run the other way: **the internal model follows the
  user-visible contract, and serves it at minimum bandwidth.** A wide *contract* does not imply
  wide *storage* — it implies storage as narrow as each value allows, with the wide semantics
  preserved on read/compute.

This is why `formal/types.md` deviation **D2 closes by reconciliation, not by an IR rewrite**: the
rule (`integer` = i64) is satisfied user-visibly; the compact encoding is the intended design, so
the spec records the encoding as conformant rather than the code widening to match a mis-stated
rule. (The earlier "widen `Value::Int` to i64" attempt was correctly **reverted** — it solved the
wrong problem and introduced a silent-truncation hazard in the IR.)

### Decision

**DECLINED — blanket-widening storage to match a wide user-visible type (2026-06-24).** General
tenet: **internal representation follows the user-visible contract and is memory-bandwidth-
conscious; storage is never widened for implementation convenience.** For integers specifically:
the compact `Int(i32)`/`Long(i64)` encoding stays; `integer` = i64 is the law on read/compute;
D2 is closed by reconciliation. The same tenet governs any future representation choice (narrow
fields, packed records, sentinels): pick the smallest encoding that preserves the user-visible
semantics.

### Revisit when

A **measurement** shows the compact-encoding dispatch (the `Int`-vs-`Long` branch, or per-width
field handling) costs more than the bandwidth it saves — at which point a *measured* widening of a
*specific* hot path is on the table, never a blanket one. Independently, if a **user-visible** i64
truncation is ever found (a value a user can observe being clipped), fix that narrow path — still
without blanket widening; the contract failing is the trigger, not the representation being
non-uniform.

## C84 — `server` ships as minimal TCP/WS primitives, not a fully-featured HTTP framework

**Catalogue:** the `server` library (`loft-lang/loft-libs-net`); see
`LIBRARIES.md`. Supersedes the declined design once held by
`lib_plans/future/08-server/`.

### Question

The `08-server` design specified "a fully featured HTTP server library" — an `App` object with
`route`/`get`/`post` registration, a `Middleware` enum, `AuthConfig` (JWT / session / API-key /
Basic), `TlsConfig` + ACME / Let's Encrypt, `serve_dir` static serving, `parse_json` body parsing,
sessions, CORS, and rate-limiting, spread across 12 loft source files. Should `server` ship that
framework surface?

### Evaluation

Every real consumer (the audience generative-art demos, the tic-tac-toe / multiplayer-editor
milestones, the routing consumer) needed only two things: answer an HTTP request, and run a
WebSocket connection — the routing itself is a `match` on `req.path`, which loft already expresses
cleanly. The framework layer (an `App`/route/middleware abstraction over that `match`) added a
large surface with no consumer pulling on it, and the heavy pieces (JWT/session auth, ACME/TLS
automation, rate-limiting) are each their own library-sized problem better solved when a consumer
actually needs them, on evidence, than pre-built into the base server. The dogfood loop pointed at
primitives, not a framework.

So the shipped library is one file (`server/src/server.loft`) over a thin native socket +
`tungstenite` layer: `listen` / `next` / `next_nonblocking` + typed `respond*` helpers for HTTP,
single-client WebSocket (`ws_upgrade` / `next` / `send` / `send_binary`), and a Rust-driven
multi-client event pump (`run(on_event: fn(WsEvent))` / `poll_event` / `broadcast` / `send_to` /
`disconnect`). Small, legible, and exactly what the consumers exercise.

### Decision

**DECLINED — the fully-featured HTTP-framework design for `server` (2026-07-02).** `server`
ships as minimal TCP/WS primitives; applications route with their own `match`. `08-server` is
closed as a declined design (not deferred work). The `App`/routing/middleware/auth/TLS/sessions
surface is not on any roadmap.

### Revisit when

A **real consumer** hits a concrete wall that primitives-plus-`match` cannot reasonably clear —
e.g. a genuine need for pluggable auth or automatic TLS certificate management in a shipping loft
program. Bring that consumer's use case as the evidence; scope the *specific* piece it needs (auth,
or TLS, or static serving) as its own addition, not the whole framework at once.
## C85 — Overflow arithmetic types NON-null; the game keeps running (don't force `integer?` on every `*`/`+`/`-`)

**Catalogue:** @F38 (arithmetic safety), @F1 (null model). Refines [C80](#c80--the-spreadsheet-fault-model-nothing-stops-a-running-calculation) and the @PLN25 `(N-Div)`/`(N-Arith)` rules (formal/types.md § DN3).

### Question

@PLN25/DN3 types a fit-failing op as `τ?` so an un-discharged null can't reach a non-null
slot: `a / b` and `v[i]` are `integer?`, forcing a `?? d` / guard / `τ?` declaration. Integer
overflow (`a*b`, `a+b`, `a-b` exceeding i64) is *also* a fit-failing op — per C80 it yields
`null` at runtime. So should `a * b` type `integer?` too, i.e. should EVERY multiply/add/subtract
return a nullable integer?

### Evaluation

**No.** Consistency argues yes, but it's the wrong call — the deciding factor is *fault
reachability vs. op frequency*, and overflow is the one place they're badly mismatched:

- **The fault is extraordinary.** i64 overflow needs both operands near 3×10⁹. Division-by-zero,
  out-of-bounds index, and bad parse happen with *everyday* values; `a*b` overflow essentially
  never does in normal code. `τ?`'s ergonomic cost is worth paying only for reachable faults.
- **The op is ubiquitous.** `*`/`+`/`-` are in nearly every arithmetic expression. `integer?`
  would propagate through *all* of them — `x = a*b + c*d` becomes nullable, every accumulator and
  loop needs `??`. It poisons the common path to guard a case that ~never fires. Division/index
  are far rarer, so their `τ?` doesn't metastasize.
- **Traps are OFF the table** (a firm maker rule): a running program NEVER halts because a
  calculation faulted — a player does not want the game to stop because the compiler decided
  stopping was better than continuing (C80, the spreadsheet). So overflow → `null` + continue,
  never a trap and never a silent two's-complement wrap (which would be a wrong value, violating
  C80's honesty).
- **Range-tracking keeps the safe cases exact for free** — `u8*u8` (→ 40000), `i32*i32` (fits
  i64), `limit(...)`-bounded operands are provably-fit → non-null. Only the default
  `integer*integer` is even in question.

The residual is a bounded soundness edge: a non-null `integer` result of `*`/`+`/`-` may hold the
overflow sentinel in the extraordinary overflow case — exactly parallel to a non-null `float`
holding `NaN`/`inf` from float ops. `?` is for *declared* absence; an overflow is an *exceptional
arithmetic result*, not a declared nullable. The `?? d` escape still works opt-in for code that
genuinely runs in the extraordinary regime (`(a*b) ?? d` fires on the sentinel).

### Decision

`a*b` / `a+b` / `a-b` type **non-null** `integer` (range-tracking narrows the provably-fit cases).
Overflow yields the `null` sentinel at runtime and execution continues (C80) — no trap, no wrap.
This is a **deliberate exception** to DN3's "fit-failing ops yield `τ?`": that rule stays for the
*reachable-fault* ops (`/`, `%`, `v[i]`, `s[i]`, and text→int **parse**, which SHOULD be `τ?`);
overflow-arith is a decided edge, not a deviation to close. Forcing nullability on every
arithmetic operation would be consistency at the expense of good taste — and, given no traps, it
would make the compiler block a game over a fault its player will never hit.

### Ratified: the in-band sentinel COLLISION is accepted, not a bug (2026-07-13)

The residual above has a concrete face: because null is an **in-band sentinel** (`i64::MIN` for
`integer`), a value that equals that sentinel reads as null — `i64::MAX + 1`, `abs(i64::MIN)`,
`1 << 63`, or a legitimate `-9223372036854775808` arriving as *data* (a file / wire / input) all
compare `== null`. This was carried on the @PLN102 pre-freeze **debatable** list ("silent loss of a
valid value"). The owner's ruling closes it as **accepted**:

- **Semantically it is just an overflow.** The contract is "don't rely on it; the program may
  malfunction" — identical to what a programmer already owes any overflow. It is not a new hazard
  class beyond the arithmetic edge C85 already accepts.
- **It is strictly BETTER than the two's-complement alternative.** A wrapped `i64::MIN` is a value
  that looks completely valid, so it corrupts silently and unrecoverably. The null bit-pattern is
  **detectable and handleable** — `x ?? d`, `if x == null`, null-propagation through the rest of the
  expression — so a program that cares can recover, and one that doesn't malfunctions exactly as it
  would under wrap. Detectability is a gain, not a cost.
- **Computed overflow and received `i64::MIN` reduce to one rule.** `i64::MIN` simply is not a valid
  non-null `integer` in loft — it *is* the null. Reserving one value out of 2⁶⁴ to make "undefined →
  null" total across every type is a clean trade; a program using the extreme edge of the range is in
  the same "know your representation" territory as one relying on wrap.

So the collision is **not** an error to add and **not** a flip blocker — it is the consistent,
already-decided consequence of the C80 total-null model and this C85 rule. (Same shape for the other
in-band sentinels: `NaN` for `float`/`single`, `255` for `u8`/`bool`, disc-0 for enums, `"\0"` for
`text` — each reserves one value so the null rule stays total.)

## C86 — Whole-value heap binds COPY; aliasing is a last-use ELISION (the rustc rule)

**Catalogue:** @F21 (references), @I60 (deps) — the ownership model's bind semantics.
Corrects [OWNERSHIP_MODEL.md § The law](OWNERSHIP_MODEL.md#the-law--whole-value-binds-copy-projections-view--binds-a-live-reference);
reclassifies formal/ownership.md D-own-4.

### Question

`OWNERSHIP_MODEL § The law` claimed a binding to a heap value "aliases; it does not
copy" — but on BOTH backends `p = o` (struct), `b = x` (vector), and `af = bx.v`
(the #415 field read) all COPY, and only projection reads (`a = vv[0]`) alias.  Should
the code migrate to the written law (everything aliases), or the law to the code?

### Decision (maker, 2026-07-03)

**The law migrates to the code.** `p = o` is a COPY by contract; it becomes `p = &o`
(an alias) **only when `o` is not used afterwards — the rustc rule — as an
optimization.**  Concretely:

- **Whole-value heap binds COPY** (struct, vector, and a **vector-typed** field read
  bound to a local — the #415 behaviour is the *correct* semantic, not a stopgap).
  *Read "field read" narrowly*: #415's scope is a field whose TYPE is a vector
  (`av = bx.v`), because that is a whole value rather than an interior place. A
  **struct**-typed field read (`w = o.inner`) is a projection and stays a view, below.
- **The copy may be ELIDED to an alias when the source is provably dead afterwards**
  (`use_analysis::ElidePlan` — the existing last-use elision).  Elision is never
  observable: a mutated or escaping source keeps the copy.
- **Projection reads stay VIEWS** — a **struct-typed** projection, whether an element
  (`a = vv[0]`, the #426 decided feature; the container kind is irrelevant — vector,
  hash, sorted and index all view) or a field (`w = o.inner`); in-place
  path mutation (`o.field = x`, `o.v[i] = y`) writes through.
- `&` remains the explicit live-reference opt-in (@PLN87, unchanged).

### Rationale (maker, verbatim)

> "In my head that is the easiest to remember rule for programmers: variables are
> their own thing, and you do not have to remember how they are constructed too much
> for their semantics."

A variable's semantics should not depend on its construction provenance — `af = bx.v`
behaves like `b = x` behaves like `p = o`: you own what you bound, full stop.  The
alias is the compiler's business (elision on provable last-use), never the
programmer's memory burden.  This serves the fun-on-pickup goal
([GOALS.md](GOALS.md)) the same way the no-traps rule (C80) does: fewer rules to
carry, no spooky action at a distance.  The principle's one deliberate boundary is
projection reads (`a = vv[0]` views, #426): an element read is understood as
*reaching into* the container rather than *taking* from it — if that distinction ever
proves a recurring source of user surprise, that is a #426 revisit, not a C86 one.

**Guarded (2026-08-05, @PLN130 F7).** The whole boundary — B-Copy, B-View and `&` — is pinned
cell by cell on both backends by `tests/scripts/201-bind-copies-projection-views.loft`. It was
previously unguarded, and @PLN130 F7 proposed deleting B-View outright on a one-cell reading
before the sweep showed all 30 cells already conforming. The decision above is what settles the
question; the test is what stops a future change from moving a cell quietly.

**A view's LIFETIME, the other half of B-View (2026-08-05, @PLN130 F2/F4/F8).** "Projection
reads stay VIEWS" says what a binding means, not how long it may mean it. A view is sound only
while the place it names exists, so loft materialises the binding — a copy taken at the bind,
reported to the author — when the container is **reshaped** (`a.remove(i)` renumbers positions),
**re-keyed** (the element would be reachable by no key), or **reassigned** (`bx = T{…}` leaves
the view's place with nothing to point at). Reassignment was the one that shipped broken: the
view answered the replacement's value, and on `--native` read freed memory.
Overwriting a place is not destroying it — `d.mid = Mid{…}` writes into the place `d.mid`
already occupies, so a view of `d.mid.inner` still sees it and still writes through.

**This applies to a PLAIN bind only.** A `&` binding is a live link and writes through
unconditionally ([formal/binding.md](formal/binding.md) B-Ref-Alias, the maker's rule
2026-08-05: *"a reference … should allow for writing it too. In all cases"*), so it may never be
materialised. What loft does instead is **refuse the removal**: taking a reference into a
container and then reshaping it while the reference is still live is a compile-time error
(B-Ref-Reshape, the maker's companion rule of the same day). That closes D-bind-8 and makes the
pair total with no runtime machinery — a `&` always writes through, because the one shape where
it could not does not compile. Across a frame the refusal covers a PLAIN parameter too, because
a parameter aliases the caller's element either way (measured; there is no bind site in the
callee to materialise at). See [loft#779](https://github.com/loft-lang/loft/issues/779).
Full rule:
[OWNERSHIP_MODEL.md § A view lasts as long as the thing it names](OWNERSHIP_MODEL.md#a-view-lasts-as-long-as-the-thing-it-names--and-loft-says-when-it-does-not);
pinned by `tests/scripts/774-view-outlives-reassigned-container.loft`.

### Consequences

- formal/ownership.md **D-own-4 reclassifies**: the #415 copy is correct; the
  implementable residual — derive the copy/alias/elide decision from the
  `ownership_of` fact + last-use instead of the syntactic `struct_vec_field`
  branch — folds into D-own-1.
- `O-Borrow`'s "a value aliasing another" scopes to projections/params/`&τ`, not
  whole-value binds.
- The ecosystem keeps its semantics (every consumer was built on copy-on-bind); the
  doc-only correction costs zero runtime change.

### Revisit when

A profiler shows bind-copies dominating a real consumer AND the elision's coverage
cannot be extended — that argues for widening `ElidePlan`, never for flipping the
semantic. **The widening is designed** →
[plans/102-stability-contract/alias-where-correct.md](plans/102-stability-contract/alias-where-correct.md).
The crystallized principle (owner 2026-07-16): **every variable is its own value — copy is the
semantics, always; a "link" (shared store) is either the compiler's TRANSPARENT optimization
(realized only where a link is safe AND unobservable) or the programmer's EXPLICIT `&`.** So the
design widens `ElidePlan` to link in more of the safe + unobservable set (byte-value-identical, no
contract-key), and the `th = t.tr_h; th[i]=v` lost-write case is NOT silently linked (that link would
be observable — the spooky action C86 forbids); it is surfaced by the dead-store lint pointing at `&`.
`#415`'s UAF cannot return (the safety gate) and nothing a program observes changes.

## C87 — `#rust"..."` template path is KEPT; do NOT migrate it away to per-Op emitters (@PLN81 closed)

### Question

Should the ~200 `#rust"..."` inline-Rust annotations in `default/*.loft` be migrated to
hand-written `OpEmitter`s (`src/generation/ops/`) so Op emission has a single source of truth, and
the template-substitution path (`calls.rs::output_call_template`) + `Value::RawExpr` deleted?
(This was @PLN81 / "plan 13".)

### Decision (2026-07-08)

**No — closed by decision.** The `#rust"..."` template path stays. `#rust` **inline** is a
first-class, *recommended* library-authoring mechanism (loft-ship **Tier 1**: "prefer `#rust`
inline over `#native` external whenever the Rust is small"; ✓ across all four targets per
PACKAGES.md), so it is a **kept public feature**, not stdlib-internal debt. Deleting the template
path would break the documented `#rust` inline library route.

### Rationale

- Premise inverted since @PLN81 was filed (2026-05-02): what looked like a redundant second path
  became the ecosystem's small-native-code path.
- Authoring cost cuts the wrong way: a new Op is a one-line `#rust` annotation today vs a struct +
  impl + register call after — @PLN81's own "cost" section flags this regression.
- The real concern (one less-bug-prone emission path — the @P203 double-substitution class) is
  better served by HARDENING the template path (the differential oracle + regression guards) and
  keeping `#rust` co-located, not by a ~200-site migration to a second mechanism.

### Revisit when

Codegen consistency genuinely needs consolidation — in which case the correct direction is the
REVERSE (fold the ~5 hand-written emitters INTO `#rust`, making `#rust` the single source of
truth), a fresh plan, NOT @PLN81's "everything → emitters, delete the template path."

## C88 — the scope-exit free gate stays dep-derived; simplify it (if ever) by promoting @PLN94's ownership oracle to authority, NOT by @PLN79's "drop the gate half + rely on idempotent free" (@PLN79 closed)

### Question

Should the multi-condition `OpFreeRef`-emission gate at scope exit (`src/scopes.rs`) be simplified
by stripping its dep-derived half — `let emit = (dep.is_empty() || is_work_ref) && !in_ret &&
!function.is_skip_free(v)` → `let emit = !in_ret && !function.is_skip_free(v)` — decoupling cleanup
correctness from dep-tracking precision by relying on `OpFreeRef` being safe to call on an
already-freed slot (`codegen_runtime.rs:100-104`)?  (This was @PLN79 / "plan 10".)

### Decision (2026-07-09)

**No to @PLN79 as written — closed by decision.** The plan's driver was mis-framed (it was opened
as a @P203 fix; @P203 turned out to be a template double-substitution, tracked separately) and its
proposal has been superseded by the ownership rework. The *concern* — cleanup correctness should not
depend on dep-tracker precision — is valid and, if anything, sharper today, but @PLN79's blunt fix
("emit more frees, rely on idempotency") is now the wrong shape.

### Rationale

- **The gate moved and grew.** It is no longer `scopes.rs:1053`'s three-condition form; it is
  `scopes.rs:3776`'s `let emit = (owns || is_work_ref || inject_free) && !in_ret &&
  !function.is_skip_free(v) && !captured_ref`, where `owns` is `dep.is_empty()` plus an @P302
  keyed-collection self-dep exception. The dep-coupling @PLN79 flagged has accreted MORE special
  cases (@P302, #323 `captured_ref`, @PLN94 `inject_free`) — the condition-thicket signal is
  stronger, not weaker.
- **The right decoupling vehicle now exists and it isn't @PLN79's.** @PLN94 landed a flow-sensitive
  `ownership_of` oracle (currently a pure observer cross-check). The clean simplification is to
  promote that oracle to the emission *authority* and collapse `owns`/`is_work_ref`/@P302/
  `captured_ref` into one ownership query — the same move @PLN85 already made on the free-side
  tracker. @PLN79 predates this and proposes a cruder decoupling than what's now available.
- **No driver.** No open bug is caused by the gate's complexity; @P203 is closed elsewhere; the gate
  territory got massive work (@PLN85 store-lifetime retirement, @PLN90 ownership, @PLN94 CFG
  dataflow oracle).

### Revisit when

@PLN94's `ownership_of` oracle graduates from observer to authority (it is still stabilizing —
caught divergences #495/#500/#501 this cycle). At that point, "make the oracle the free-emission
authority and dissolve the special-case gate" is a **fresh plan**, NOT @PLN79's drop-the-half
proposal. The phase-00 characterisation + @P203 strace evidence stay in
`plans/79-scope-exit-emission/` as historical record.

## C89 — No tuple-style enum variants; a matcher reads like grammar and is never forced

### Question

Should loft add positional / tuple-style enum variants (`Ok(a)`, `Num(i64)`) alongside its struct
variants (`Ok { value: a }`, `Num { v: integer }`)?  And how far should the @PLN35 PEG match-pattern
surface go syntactically?

### Decision (2026-07-10)

**Tuple-style variants: permanently declined — never planned.**  loft enum payloads are always
NAMED fields.  The @PLN35 structural matcher stays, under two conditions: it is **never forced**, and
its surface **reads like grammar notation, not regex**.

### Rationale

- **The root problem is ACCESS, not spelling.**  `Ok(a)` *wraps* the value, so the only way to read
  `a` is to destructure it — you are forced through a matcher just to read data.  Because
  match-to-read is painful everywhere, a whole slew of *derived construction* grows up to mitigate
  it: `?`, `unwrap`, `if let`, `let-else`, `.map`/`.and_then`, `matches!`.  The complexity is not the
  variant; it is the mitigation the wrapping makes necessary.
- **loft refuses this at the root.**  Enum payloads are named fields — you read `e.field` directly,
  and matching is for *dispatch* (which variant), never for *extraction*.  A maybe-value is `τ?` read
  and discharged inline with `??` (the anti-`Ok(a)`: no `Some`/`Ok` wrapper, no combinator tail).  So
  there is nothing to mitigate and the mitigation-syntax sprawl never gets a foothold.
- **A matcher is fine because it is never forced.**  @PLN35 PEG patterns are legitimate *structural
  dispatch* — recognizing the shape of a token stream / AST / vector — reached for by choice, never a
  tax the data model imposes to read a value.  "Never forced" is the invariant; it is also why no
  mitigation layer forms around it.
- **Read like grammar, not regex.**  The PEG surface must read like the standardized way parser logic
  is written (PEG / EBNF-style: a sequence of named elements with `|`, `?`, `*`, `+`, grouping, named
  captures) — which a reader follows without training — NOT like regex, which needs it.  That is
  exactly why text matching stays in the regex *library*, out of `match` (one text-pattern language,
  opt-in).  **loft will pay a bit of extra PARSER LOGIC to keep that readable-grammar surface —
  readability of the written pattern beats parser simplicity.**

### Revisit when

Never, for tuple variants (a permanent non-goal).  The PEG readability bar is a live constraint on
@PLN35's per-operator syntax choices (e.g. how a repetition separator is spelled) — each decided
against "reads like grammar" in
[plans/35-match-peg/FORMAL-DESIGN.md](plans/35-match-peg/FORMAL-DESIGN.md).
## C90 — Each nullable scalar reserves ONE bit-pattern for null (the in-band sentinel residual; accepted, frozen)

**Catalogue:** @F1 (null / Optional), @F3 (scalar types), @F4 (width integers). Closes the @PLN102 pre-freeze [null-model keystone](plans/102-stability-contract/keystone-null-model.md) (option B); the sibling of [C85](#c85--overflow-arithmetic-types-non-null-the-game-keeps-running-dont-force-integer-on-every--) (overflow yields the sentinel) and [C80](#c80--the-spreadsheet-fault-model-nothing-stops-a-running-calculation) (the spreadsheet model).

### Question

`null` for a scalar is stored **in-band** — a single reserved bit-pattern in the value slot itself, not an out-of-band tag (formal/types.md § null-representation). This is zero-overhead (a `τ?` is the same width as `τ`; a `vector<integer?>` is as dense as `vector<integer>`), but it costs one representable value per type: the reserved pattern cannot also be stored as *data*. Before freezing the contract, should loft **retire** the sentinel — give nullable scalars a tagged representation (option A) so no value is lost — or **keep** it and confront the residual (option B)?

The reserved value per type (frozen by this decision):

| type | reserved null value |
|---|---|
| `integer` (8-byte) | `i64::MIN` (`-9223372036854775808`); the value range is `[i64::MIN+1, i64::MAX]` |
| narrow `u8`/`i8`/`u16`/`i16`/`i32` | the top stored width value (`u8?` → `255`), excluded from the `τ?` non-null range |
| `boolean` | `255` (three-state false/true/null — [C73](#c73--boolean-is-three-state-false--true--null--is-raw-truthiness-coerces)) |
| `character` | codepoint `0` (NUL, `'\0'`) |
| `float` / `single` | a reserved `NaN` (`+inf`/`-inf` are **real** values) |
| a reference | out-of-band `nullref` — **no** collision (a reserved `DbRef`) |
| a struct `S` in a `vector` | the tagged `__nullable<S>` enum — **no** collision |

### Evaluation

**Keep it (option B).** Option A (a tag bit / tagged rep for every nullable scalar) buys back the one lost value per type, but at a cost the whole stack pays forever: a `τ?` grows wider than `τ`, `vector<τ?>` loses its density, the store layout gains per-field tags (revising L-Null, the layout hash, @PLN97's golden layout, and the eval-stack/op rewrites on both backends), and materialization pressure returns — the exact machinery @PLN25 removed. The residual it removes is *bounded and cheap to live with*:

- **References and struct-in-vector — the aggregate cases — already pay ZERO cost**, out-of-band by construction (`nullref`, `__nullable<S>`). The residual is *only* the scalar leaves.
- **The lost value is one extreme, unreachable in normal data.** `i64::MIN` (use `long`/full-range or the sentinel is off by one from your data), NUL as a data character, a `u8?` needing to store exactly `255` — all rare; the everyday range is intact. `not null` reclaims the value where a field genuinely needs it.
- **The three silent hazards the sentinel *could* cause are now guarded**, which is what makes the residual merely a *storage* limitation rather than a correctness one: null comparison is uniform across every scalar (D-op-null-1 — `null == null` true, null orders low); an op whose result *lands on* the sentinel reports rather than silently masking (D-op-null-2 — shift/cast collisions raise `Shift/CastOutOfRange` like `÷0`); and the stdlib functions that can legitimately produce null on a reachable path are typed `τ?` so the type does not lie (`find`/`rfind`/`min_of`/`max_of` — keystone step 4).

This is the same taste as C85: pay a bounded, documented soundness edge to keep the common path zero-overhead, rather than tax every value to erase an extreme almost no program stores.

### Decision

**Each nullable scalar keeps its single in-band reserved value; the specific pattern per type (table above) is part of the contract-1 freeze.** A `τ?` is bit-identical in width to `τ`; the reserved value is observable and excluded from the base type's non-null range (`(E-Null)`). The residual — a nullable scalar cannot store its one reserved value as data — is an accepted, documented limitation, not a deviation to close. The collision sites that could *silently produce* a sentinel are guarded (D-op-null-1/2 CLOSED); reachable-fault stdlib returns are honest `τ?` (step 4). Golden pin: `tests/scripts/pln102-null-residual-golden.loft` freezes each reserved value as a boundary (the pattern reads null; an adjacent value does not) on both backends.

## C91 — `==` is value-by-value / reference-by-identity (bounded, never a reference-chase); `===` reserved for opt-in deep equality

**Catalogue:** @F1 (null / equality), @F2 (operators). Closes the @PLN102 pre-freeze **F7** ("`ref ==` is identity, not structural — a decision"). Sibling of [C86](#c86--whole-value-heap-binds-copy-aliasing-is-a-last-use-elision-the-rustc-rule) (whole-value copy) and [C90](#c90--each-nullable-scalar-reserves-one-bit-pattern-for-null-the-in-band-sentinel-residual-accepted-frozen) (the in-band sentinels).

### Question

`==` on two struct references is **identity**, not structural, and it composes with C86 (whole-value bind COPIES) into two footguns:

```loft
struct P { x: integer, y: integer }
a = P { x: 1, y: 2 };  b = P { x: 1, y: 2 };  c = a;
a == b   // false — two constructions, two records, identity says unequal
a == c   // false — `c = a` COPIED (C86), so c is a distinct record
```

Should `==` on a struct stay identity, become structural (deep content), or something in between? A **deep** `==` would be the intuitive answer but is a recursive crawl that can loop through many megabytes of nested vectors/structs on an operator a programmer reaches for constantly. A **shallow** `==` (compare the record's own fields, identity for nested references) is cheap but **internally inconsistent** — `P{1,2} == P{1,2}` is true yet `P{items:[]} == P{items:[]}` is false, for a reason that lives in the layout, not the language; a programmer can't state one rule for what `==` does, and that unpredictability is more dangerous than either honest extreme.

### Evaluation

The deciding constraints (owner, 2026-07-13): **`==` must be quick — never an automatic crawler over the data it points at — and it must not be internally inconsistent.** Both rule out the shallow hybrid *and* the deep crawl. What remains is one uniform, cheap rule, and loft already lives it: `5 == 5` and `"a" == "a"` are value/content, `P{…} == P{…}` is identity. The honest statement of that is not "`==` is identity" (text disproves it) but:

> **`==` compares values by value and references by identity.**

- **It is bounded, never a reference-chase.** A value is compared over its OWN storage — a scalar's slot, a text's bytes, a `value struct`'s flat record ([C101](#)/flat by construction). A reference is compared as its handle — O(1). Neither case follows a pointer into another store, so `==` is never the megabyte crawler; the only per-element work is text/`value struct`, which is the value's own bytes.
- **It is consistent.** One sentence describes `==` across the whole type system; the split tracks the type's value-vs-reference nature, which the language already exposes (`value struct` vs `struct`). `value P{1,2} == value P{1,2}` is `true` (matching its copy semantics); the reference `struct P` above stays identity.
- **Progressive disclosure is the ergonomic win.** A programmer reaches for the simplest syntax (`==`) first; when the cheap answer is not what they want, they escalate to the more semantically-complete but more expensive alternative — deliberately, by typing a third `=`. The simple tool is cheap and predictable; the correct-but-costly tool is opt-in and clearly marked.

### Decision

- **`==` / `!=`** — value types (`integer`, `float`, `single`, `character`, `boolean`, `text`, `value struct`, unit enum) compare **by value/content**, bounded by the value's own storage and **never chasing a reference into another store**; reference types (`struct`, `vector`, `hash`, `index`, …) compare **by identity**, O(1). One uniform rule; part of the contract-1 freeze.
- **`===` / `!==`** — RESERVED for opt-in **deep structural** equality (recurse through references, all the way down). Not shipped by this decision — the contract (that deep equality is a distinct, explicit, more-expensive operator, never the `==` default) is fixed now; `===` itself ships when a consumer needs it. When built it MUST be **consistently deep** (recurse into everything — a shallow `===` reintroduces the inconsistency this decision rejects) and **cycle-safe** (reference structs can form cycles; a naive deep walk loops forever).
- **Rejected:** a shallow/hybrid `==` (structural for a record's own fields, identity for nested references) — internally inconsistent and therefore dangerous, regardless of its speed.

## C92 — Compound assignment evaluates its place expression exactly once

**Catalogue:** @F2 (operators). Closes the @PLN102 pre-freeze **F2** ("compound-assign double-evaluates its place"). Same class as the F4 assignment eval-order item — an evaluation count/order that would otherwise freeze as impl-defined.

### Question

`w[idx()] += 5` today evaluates the place **twice** — the desugaring `w[idx()] = w[idx()] + 5` emits the index sub-expression `idx()` for both the read and the write (verified 2026-07-14, both backends; nested `m[i()][j()] += 5` evaluates it **four** times). For a pure index this is a wasted read; for a *side-effecting or divergent* index it is a live silent-wrong:

```loft
w[next()] += 5   // next() returns 1 then 2 → reads w[1], writes w[2]: lands on the WRONG slot
w[log()] += 5    // any side effect fires twice
```

Should a compound assignment evaluate its place once, or is double-evaluation accepted and frozen?

### Evaluation

Evaluation count of the place is observable semantics, so it freezes at contract 1 — the choice is forced pre-freeze. Double-evaluation is not merely a footgun: a divergent index makes the read and the write target *different* slots, a plausible-wrong value with no error — exactly the class the compat model resolves to correct function before the freeze ([COMPATIBILITY.md](COMPATIBILITY.md) § the error surface: "a would-be-error is first a rewrite-to-correct-function"). Every mainstream language (C, Rust, Python) evaluates an lvalue's addressing sub-expressions once. The once-eval lowering is **byte-identical** for the common case — a constant or variable index (an idempotent read) — so the fix converts only side-effecting-place programs, of which there are essentially none in-tree (measured at implementation).

### Decision

- A compound assignment `place op= rhs` (and its keyed-collection forms) evaluates the **addressing sub-expressions of `place`** — index expressions, container-producing calls — **exactly once**: bind them to hoisted temps, then read and write through the *same* temps. The stored result and the number/order of place-side effects are then well-defined. Part of the contract-1 freeze.
- Owner ruling 2026-07-14 ("evaluate the place once").
- **Rejected:** freezing double-evaluation — the divergent-index corruption makes it a silent-wrong, not a defensible edge.
- Fix scope + plan: [compound-assign-place-once.md](plans/102-stability-contract/compound-assign-place-once.md).

## C93 — A `par` worker's captured parent state is read-only; a write to it is a compile error

**Catalogue:** @F2 (operators) / threading. Instances the platform rule *no runtime errors, ever* (DESIGN_DECISIONS [C80](#c80--)): we do not fault at runtime — we either DISALLOW what cannot work (a compile error) or make it work in a lesser state (null). A `par` data race cannot be made to "work" as null, so it is DISALLOWED. Sibling of the sandbox host-data read-only model.

### Question

A `par` worker whose body writes state captured from the enclosing scope is a data race — N threads mutating one store with no synchronisation:

```loft
struct Shared { n: integer }
fn bump(s: Shared, x: integer) -> integer { s.n = s.n + x; x * 2 }   // writes shared s.n
s = Shared { n: 0 };
for x in [1,2,3,4,5,6,7,8] par(r = bump(s, x), 8) { total += r; }     // 8 threads write s.n
```

Today this does not race cleanly and it is not cleanly rejected: it falls through to a **codegen slot-panic** (`Incorrect var x[65535]`, `codegen.rs:3235`) — a runtime crash, the one thing the platform never does. The purity machinery exists (`Purity`, `ImpureCategory::ParentWrite`, `is_par_safe`, the phase-5b deep check) and even treats an un-annotated fn conservatively as `ParentWrite`, yet a user worker writing an *aliased reference/vector parameter* slips past it into the crash. How should an impure `par` worker be handled at contract 1?

### Evaluation

A data race is the single fault loft cannot resolve to a defined value: unlike an out-of-range read (→ null, C80) or an overflow (→ sentinel, C85), a race yields nondeterministic corruption that no null can stand in for. So the *"make it work in a lesser state"* half of the platform rule does not apply — the only faithful option is *"do not allow it,"* a compile error. And the signal is already present: the compiler knows which workers write captured state (`ParentWrite`). The cleanest expression is **data-centric, not analysis-centric**: the state a `par` worker captures from its parent is **read-only inside the worker, from the start** — the same principle by which host data and parameters are read-only unless explicitly update-linked (the sandbox model), applied to a second place. Then a write to it is a plain compile error *at the write* — `s.n = …` reports "cannot write `s` inside a `par` worker; it is read-only here" — and the race becomes **unexpressible**, not merely detected. The worker stays free to *read* captured state, read the element, compute, and *return* a value (folded sequentially); only writes to captured parent state are forbidden. This must land pre-freeze: it is an error-ADD, and the error surface can only shrink after contract 1 ([COMPATIBILITY.md](COMPATIBILITY.md) § the error surface).

### Decision

- **The parent state a `par` worker captures is read-only inside the worker.** A write to it — directly (`s.n = v`, `cap[i] = v`) or through a call that writes it (an aliased reference/vector argument bound to a writing parameter) — is a **compile error reported at the write site**, from the start. Part of the contract-1 freeze.
- **No runtime error, no runtime crash.** The current `codegen.rs:3235` slot-panic on an impure worker is a bug this closes: the rejection lands at parse/type time, before codegen.
- **The worker may READ captured state, the element, and locals, and RETURN a value.** Its own locals and the loop element stay mutable; only captured *parent* state is frozen read-only. Host I/O / PRNG (`HostIo`/`Prng`/`Io`) stay allowed — the host serialises them — matching the existing `ImpureCategory` split.
- **`par` is deliberately STRICT — the false-positive friction is accepted (owner, 2026-07-14).** `par` is a complex construct in itself, so strictness is the right default: a worker the purity analysis flags as capturing-and-writing is rejected even when the write might have been benign (disjoint slots), rather than widening `par` to admit "sometimes-safe" mutation. The programmer restructures to a pure fold (return the value; accumulate sequentially) — which is what `par` is for.
- **More capability, if ever needed, is a NEW inherently-safe construct — never a looser `par` (owner).** Broader shared-mutation parallelism would arrive as a *separate, inherently-safe* construct added additively (the compatibility ratchet — the reliable surface only grows), not by relaxing `par`'s read-only rule (which post-freeze is impossible anyway: loosening an error is fine, but re-tightening later is not, so `par` must be strict *now*). **Not currently envisioned** — the existing `par` already covers the need; this records the direction, not a planned construct.
- **Rejected:** leaving the race as undefined behavior (the one hole in a memory-safe, no-runtime-error platform), and "defining" it by silently serialising or copying (surprising, and it hides the programmer's mistake rather than surfacing it).
- Owner ruling 2026-07-14. Fix scope: [par-capture-readonly.md](plans/102-stability-contract/par-capture-readonly.md).

## C94 — Integer `/` truncates toward zero and `%` takes the dividend's sign; `floor_mod` is the wrap-around helper

**Catalogue:** @F2 (operators) / math. Fixes the sign convention for negative integer division at contract 1 — the one place a language must *pick* (C, Rust, Go, Java, JS truncate; Python, Ruby floor). Both are legitimate; the freeze needs one named default.

### Question

For integer `/` and `%` with a negative operand, two self-consistent conventions exist, differing only in how they round a negative quotient:

```loft
// truncate toward zero (C/Rust)      vs   floor toward -∞ (Python)
-7 / 2   ==  -3                             -7 / 2   ==  -4
-7 % 2   ==  -1  (sign of dividend)         -7 % 2   ==   1  (sign of divisor)
```

Truncation is the faster hardware default and makes `(a / b) * b + a % b == a` hold; floor gives a remainder that always lands in `[0, n)`, which is what circular indexing (`grid[(i - 1) % w]`) wants — but only under the floor convention, so under truncation that idiom silently reads a negative index. Which convention is loft's, and how does the *other* need get met?

### Evaluation

The two conventions are **two different operations for two different use cases**, and the split is decided by *which* the terse operator should serve — not by what other languages do.

- **Truncating `/` + dividend-sign `%` serve signed-magnitude decomposition:** splitting one signed quantity into (quotient, remainder) parts that both carry the sign and reconstruct — signed time (`-90s → -1min, -30s`), coordinates relative to an origin, round-toward-zero quantization (`a - a%b`), digit extraction after `abs`.
- **`floor_mod` serves cyclic domains:** a value on a ring of size `n` where you want the canonical representative in `[0, n)` and the input's sign is meaningless — array-index wrap, clock, angle, day-of-week, hashing a possibly-negative value into a bucket.

The load-bearing fact is that **`/` and `%` are a matched pair**: the identity `a == (a / b) * b + a % b` holds for the truncating pair *and* the floor pair **equally**, so the identity does **not** discriminate between them, and the two cannot be mixed (a truncating `/` with a floor `%` would make `%` imply a *different* quotient than `/` returns — an incoherence worse than either clean convention). So the choice collapses to *which `/` do you want*, and `%` follows. Integer `/` should **truncate toward zero** because that is "drop the fractional part" — `-7 / 2 = -3.5 → -3`; floor `/` gives `-4`, surprising for both signs, and division is used far more than negative-operand `%`, so the operator optimizes for un-surprising division. The cyclic case genuinely wants floor, but it is a **distinct operation**, and giving it the name `floor_mod` is a *feature*: at `(i - 1).floor_mod(w)` the reader sees the wrap, whereas a bare `(i - 1) % w` silently yields `-1` — an out-of-range index, exactly the kind of silent footgun loft's total/no-surprise model removes. Adding the helper is additive (a new stdlib name), so it lands cleanly pre- or post-freeze; the *operator* convention is what the freeze pins.

### Decision

- **Integer `/` truncates toward zero** (`-7 / 2 == -3`), and **`%` returns the remainder with the sign of the dividend** (`-7 % 2 == -1`, `7 % -2 == 1`) — the C/Rust convention. `a == (a / b) * b + a % b` holds for all non-zero `b`. Part of the contract-1 freeze.
- **`/` or `%` by zero is a null** (C80 — an undefined value is a null, never a fault), dischargeable with `?? default`. Unchanged; recorded here for completeness.
- **`floor_mod(both: integer, divisor: integer) -> integer?`** is the wrap-around helper: the remainder with the sign of the **divisor**, landing in `[0, divisor)` for a positive divisor (`(-1).floor_mod(3) == 2`). Pure stdlib (`default/01_code.loft`), integer-only, `floor_mod(x, 0)` is null like `%`. Use it for circular indexing (`grid[(i - 1).floor_mod(w)]`).
- **Rejected:** switching the operators themselves to floor semantics — makes `/` surprising (`-7 / 2 == -4`) to serve the cyclic case a helper serves cleanly, and a truncating-`/`-with-floor-`%` hybrid is incoherent (the two operators would disagree on the quotient). Also rejected: leaving wrap-around indexing to open-coded `((i % w) + w) % w` (the footgun stays un-named at every call site).
- Owner ruling 2026-07-15 — the split is decided by the use cases (signed-magnitude decomposition vs cyclic-domain representative), not by matching C/Rust; "keep truncate but add a floor-mod helper".

## C95 — A definition a same-named method would silently shadow is a compile error (no silent redefinition)

**Catalogue:** @F2 (operators) / naming. Instances the platform rule *no runtime errors, ever* ([C80](#c80--)) at its **compile-time valve**: what cannot work is *disallowed*, not run into a silent-wrong result. Surfaced by C94 (adding stdlib `floor_mod` collided with an existing library helper).

### Question

loft dispatches a method-or-free function (`fn foo(both:/self: T, …)`) on its **first-argument type**. A plain FREE function `foo(x: T, …)` whose name collides with such a method on the **same** first-arg type was **silently shadowed**: a call `foo(x, …)` resolved through the method dispatcher to `t_<T>_foo`, and the free `n_foo` was **unreachable, with no diagnostic**. Exact-signature redefinitions already errored (`Cannot redefine`), but a *different-parameter-name* redefinition slipped through the `Dynamic`-dispatcher exemption in `add_fn`. Repro — the definition compiles yet never runs:

```loft
fn floor_mod(ma: integer, mb: integer) -> integer { … }   // stdlib floor_mod(integer) wins; this is dead
pub fn clamp(val: float, lo: float, hi: float) -> float { … }  // stdlib clamp(float) wins; this is dead
```

Should such a definition be allowed (silently shadowed), or rejected?

### Evaluation

A definition the programmer wrote that can **never be called** is a latent bug — the author believes their `clamp` runs; the stdlib's does. Per *no runtime errors, ever*, the honest answer is the **compile-time valve**: disallow it, at the definition, naming the collision — rather than let a call silently reach a different function. The predicate must be **dispatch-exact**: reject a free function *only* when a method for its **first-argument type** already exists (`t_<len><sig>_<name>` — the function's canonical internal name under loft's first-parameter mangling), because only then does the call actually resolve to the method. A free function that merely **shares a name** with a method on a *different* receiver type stays legal — arg-type dispatch keeps it reachable (`scale(integer, …)` beside the trait method `scale(self: Self, …)`; `byte_at(integer, …)` beside `byte_at(self: text, …)`). This is an error-ADD, so it lands **pre-freeze** (the error surface can only shrink after contract 1). It immediately surfaced four in-repo latent shadows: two libraries redundantly re-defined a stdlib function byte-for-byte (`shapes` `clamp`, the doc viewer's `basename`), a test helper was vacuously shadowed (`83-return-in-if-expr`'s `clamp` was testing the *stdlib* clamp, not its own return-in-`if` body), and the `time` library's private `floor_mod` (resolved by adopting the new stdlib `floor_mod`).

### Decision

- **A free function `foo(x: T, …)` is a compile error when a method `foo` for first-arg type `T` already exists** — it would be silently shadowed (the call dispatches to the method, never the free `n_foo`). Reported at the definition: `Cannot redefine 'foo' (already defined at …)`. Part of the contract-1 freeze.
- **The predicate is first-argument-type exact** (loft's current mangling keys on the first parameter): a free function sharing a name with a method on a *different* receiver type stays legal. When mangling extends beyond the first parameter, this predicate follows it automatically — no separate rule to maintain.
- **This closes the different-parameter-name gap** left by the `Dynamic`-dispatcher exemption; exact `both:`/`self` same-type redefinitions already errored via the mangled-name check.
- **No silent shadow, no runtime error.** The previously-silent case becomes a clear compile error (valve a). A library must not redefine a stdlib function — it uses the stdlib one (or picks a distinct name).
- **Rejected:** silently shadowing (hides a definition that never runs); and making the user's definition *win* over the stdlib (an invisible footgun in the other direction — a program could silently re-point a stdlib call).
- Owner ruling 2026-07-15 — "that silent shadow is the problem, make it an error too"; "we are not yet under contract". Implementation: `src/data.rs::add_fn` (`shadows_a_method`).

## C96 — Library shipping is keyed on trust-root presence: a key-present machine ships autonomously, a key-absent one defers

**Catalogue:** @F/registry (publishing). Fixes the trust model of the file-based registry ([PKG_REGISTRY.md](PKG_REGISTRY.md), [REGISTRY_SUBMIT.md](REGISTRY_SUBMIT.md)) at contract 1: the reliable surface (a signed, immutable, append-only index) may only grow, and shipping into it must not bottleneck on one human.

### Question

The registry index is signed by the maintainer trust-root key, and `loft install` refuses an index whose signature doesn't verify — so a wrong/stale signature breaks **every** install. That key must therefore never enter CI, which makes signing inherently local and human-gated. But if the maintainer must sign every release, they are a per-release bottleneck on *all* library shipping. How does a library reach trusted-in-the-registry without a human-only step in every release?

### Evaluation

**Sign policy (rarely), not every release.** The bottleneck is that one key re-signs the *whole index* on every publish. Two structural changes remove the human from the per-release path without weakening trust:

1. **Key-presence is the tier boundary.** On a machine that HOLDS the trust-root (on `tuxedo` it is a file, `~/.loft/trust-root/registry-signing-key.bin`) the machine *is* the signer — ship is fully autonomous, no touch, the file key the default (no YubiKey attempt, no prompt). On a machine WITHOUT the key, it cannot sign and must not fake it: it validates locally and **defers** via a submission PR a key-holder folds in and signs. The maintainer is never a required step on a key-present machine; on a key-absent one the only human is a key-holder folding in — batchable, not per-release-per-author.
2. **Per-artifact signatures + an append-only, immutable index** make autonomous signing low-risk and retire the re-sign foot-gun structurally: each tarball is individually signed and independently verifiable against the trust-root; the index is untrusted metadata; a ship transaction can only APPEND (never alter a published version), and index + `.sig` are only ever written together. Concurrent lib landings are safe because appends of immutable versions commute — a single-writer ship transaction with a compare-and-swap push + a `submissions/` staging dir means a lost race just re-folds, never corrupts.

The maintainer's only irreducibly-human steps become **policy**: admit a namespace (first-time, especially `#native` — the V6 review) or revoke a key. Not per-release.

### Decision

- **Shipping is keyed on trust-root presence.** Key-present (the `tuxedo` default) → autonomous: validate → package → sign artifact + index with the local file key → append → push, no human step. Key-absent → **defer**: validate locally, open a submission PR to a `submissions/` staging area (NOT `index.json`), which a key-present machine's ship pass folds in and signs.
- On a key-present machine the **local file key is the default signer** (no YubiKey attempt, no prompt); `LOFT_REGISTRY_SIGNER` / `--yubikey` override.
- **Per-artifact signatures + append-only immutable index**; the index is untrusted, verified per-entry against the trust-root. The re-sign foot-gun is gone by construction (index + `.sig` written together, by the one transaction).
- The ship transaction is **single-writer with a compare-and-swap push loop** draining own-lib tags + `submissions/`, so concurrent landings can't race the signed index.
- The **V1–V6 validation gate** ([library-ship-validation.md](plans/102-stability-contract/library-ship-validation.md)) runs before signing; the signature attests validation.
- **Rejected:** the trust-root in CI (a leak breaks every install); a fully-automatic push→signed-release (can't be kept safe); a mutable single-signed index blob (the re-sign foot-gun — it broke installs once already). A scoped delegated CI key is a fancier key-absent tier, deferred until unattended CI publishing is a real need; the submission-PR defer is the MVP.
- Owner ruling 2026-07-15 (`tuxedo` default = local key). Proven in part: `scripts/registry_maintain.sh` + the local file key shipped shapes 0.3.0 + time 0.2.1. Freeze-gate companion: `.github/workflows/revalidate-libs.yml`.

## C97 — A library's public symbols live under its module, not the global namespace (so the stdlib can grow without breaking a shipped lib)

**Catalogue:** @F2 (operators) / modules + naming. A contract-1 compat decision — the precondition for a stdlib that is *both* absolute-compat *and* still growable.

### Question

A library's `pub fn clamp` registers as the **global** `n_clamp` — the same namespace as the stdlib (that is why [C95](#c95--)'s redefinition error fired on it). So library public symbols **leak into the global namespace**. The consequence surfaced at once: adding a stdlib `floor_mod` ([C94](#c94--)) **broke already-shipped libraries** that defined the same name — `shapes` (`clamp`) and `time` (`floor_mod`) both stopped compiling. Post-freeze, with an absolute-compat stdlib we still want to grow, this is a contradiction: any stdlib addition might collide with a name some published, immutable library already ships. Should library public symbols share the global namespace, or live strictly under their module?

### Evaluation

The `shapes`/`time` break is proof this is real, not hypothetical — and unshippable at contract 1. Two directions:

- **(a) Keep global-namespace library symbols.** Then the stdlib effectively **cannot grow** post-freeze: every addition is a potential compat break, so either it is forbidden (a frozen-forever stdlib) or it forces a coordinated republish of every colliding library — which we just did by hand for two libs, and which absolute-compat forbids post-freeze (a shipped program importing the old lib must keep working).
- **(b) Namespace library public symbols.** `shapes::clamp` never enters the global unqualified namespace, so stdlib `clamp` and `shapes::clamp` coexist and a stdlib addition can **never** collide with a library symbol. The stdlib grows additively forever, absolute-compat holds, and C95's error narrows to its correct scope — guarding a *program's own* top-level redefinitions, not library-vs-stdlib.

Only (b) is compatible with an absolute-compat stdlib that still grows. The stdlib stays the one shared namespace every program imports (it is the library every program uses); everything else is qualified. The cost: unqualified access to a library symbol now needs an explicit import (`use lib::name`) rather than leaking globally — a migration that must land **pre-freeze**, since the resolution rule freezes with the contract.

### Decision

- **A library's public symbols are addressed under its module** (`lib::name`, or brought into scope with an explicit `use lib::{name}`), and are NOT injected into the global unqualified namespace. The **stdlib is the sole global namespace** — the library every program imports.
- Therefore **the stdlib may grow additively without ever colliding with a shipped library**, and C95's no-silent-redefinition error applies to a program's own top-level definitions (its intended scope), not to a library duplicating a stdlib name.
- This retires the class of break C94→C95 exercised (a language addition retro-breaking shipped libs); `revalidate-libs.yml` is the pre-freeze guard that proves it while the change lands.
- **IMPLEMENTED (free-fn case) 2026-07-15** in `src/data.rs::add_fn`: a definition in a LIBRARY source (source ≥ 2 — not the STD prelude, not the user's MAIN program) is registered **scoped to its own source** (`source_nr(self.source, …)` instead of the STD-fallback `def_nr`), so a library name that exists only in the stdlib is not a redefinition — it coexists (`lib::clamp` beside the stdlib `clamp`). STD and MAIN keep the global-scope C95 check (a MAIN top-level def a stdlib method would silently shadow still errors). Verified both backends: a library `clamp` resolves as `lib::clamp` while the bare `clamp` stays the stdlib's; a library's OWN duplicate still errors; test `pln102_c97_library_may_define_a_stdlib_name` (`tests/imports.rs`). **Residual:** a library `both:`/`self:` METHOD colliding with a stdlib method still errors — methods register as attributes on a shared (global) type, which module-scoping can't cleanly cover; the real cases (`shapes` `clamp`, `time` `floor_mod`) are free fns, so this is a rare, accepted limitation.
- **Rejected:** global-namespace library symbols (makes stdlib growth a permanent compat hazard — proven by `shapes`/`time`; incompatible with the freeze); per-collision coordinated republish (does not scale, and absolute-compat forbids it post-freeze).
- Owner-directed 2026-07-15 (the direction reached across the shapes/time ship work). Trigger: C94 `floor_mod` broke shipped `shapes`/`time` via C95. Companion: [library-ship-validation.md](plans/102-stability-contract/library-ship-validation.md).

## C98 — `use lib;` binds only the `lib` namespace; unqualified access is an EXPLICIT `use lib::*` / `use lib::(…)`, where the imported name wins

**Catalogue:** @F2 (operators) / modules + naming. The name-resolution HOW that [C97](#c97--) deferred — the rule that makes "the stdlib can grow without breaking a program" actually hold.

### Question

C97 makes a library's public symbols module-scoped. But loft also has wildcard import. If a bare `use lib;` (or a wildcard) brought a library's names into the **unqualified** namespace, a later stdlib addition of the same name re-creates the C95 collision one level up — and *any* resolution of it (imported wins / stdlib wins / ambiguity error) either re-opens the silent-shadow class or **breaks an existing program when the stdlib grows** (violating absolute compat). How does unqualified access to a library's symbols work?

### Evaluation

Split the surface by **explicitness**:

- **`use lib;` binds exactly one name — `lib`**, the namespace handle; every function/type is reached as `lib::name`. It brings *nothing* unqualified, so a program that only `use lib;`s is **immune to stdlib growth** (it always qualifies) — the common case can never collide.
- **`use lib::*;` (wildcard) and `use lib::(a, b, c);` (selective)** are *explicit constructions* that pull names into the unqualified namespace. There the **imported name wins** over a colliding stdlib name — and that is *safe*, because the programmer explicitly asked for those names unqualified: it is their stated intent, not a silent shadow. A program that did this keeps its binding forever, so a later stdlib name of the same spelling **does not change its behavior**.

So absolute compat holds both ways — bare-`use` programs qualify (no collision), explicit-import programs keep their binding (no behavior change) — and the stdlib can grow forever with **no** program breaking and **no** ambiguity-error (which would itself be a break). The wildcard/selective import is the "you asked for it" opt-in; the bare `use` is the collision-proof default.

### Decision

- **`use lib;`** introduces exactly one name, `lib` (the namespace); members are `lib::fn` / `lib::Type`. **No** unqualified leakage.
- **`use lib::*;`** (wildcard) and **`use lib::(a, b, c);`** (selective) explicitly bind those names into the unqualified namespace; an explicitly-imported name **wins** over a stdlib name of the same spelling — the author's owned choice, so the C95 silent-shadow concern does not apply.
- Because bare `use lib;` never imports unqualified and an explicit import keeps its binding across stdlib growth, **no program breaks when the stdlib grows**, and no ambiguity-error is needed — the C97 guarantee holds.
- **Intended asymmetry with [C95](#c95--):** *defining* your own top-level free fn that a stdlib method would silently shadow is a C95 error (the def was silently dead); *explicitly importing* a library name unqualified is allowed and wins — the line is exactly **silent vs explicit**.
- **Collision resolver — aliased import (ALREADY in loft, @PLN22 P3/P4):** `use lib::(a as x, b);` imports `a` locally as `x` (plus namespace/type/fn aliases: `use lib as el;`, `use lib::Status as St;`, `use lib::make as mk;`). So any clash — two libraries exporting the same name, or an import you want to keep distinct — is resolved by binding it under a chosen local name. This also resolves the two-explicit-imports edge below: `use a::(x); use b::(x as bx);`. **Syntax — keep `as` (owner 2026-07-15):** the resolver already exists and `original as alias` is uniform across all four alias forms, so no new syntax is added (a proposed `alias = original` was declined — it would fragment the shipped alias grammar and break existing `use` statements for a cosmetic disambiguation; the `as` overload with the type-cast `as` is unambiguous inside a `use` group).
- **Open edges (not ruled here):** an explicitly-imported name vs the program's own top-level def — resolve when specced.
- Owner ruling 2026-07-15. Companion: [C97](#c97--). **Already shipped (@PLN22 P1–P4):** the whole `use` surface — bare `use lib;` = prefix-required namespace bind, `use lib::*;` = wildcard, `use lib::(a, b, c)` = selective, `use lib::(a as x, b)` / `use lib as el` / `use lib::T as St` = aliasing. So C98 needs **no new syntax**; the only residual is the C97 internal change (a library's `pub` symbol must stop registering as global `n_<name>` — the dual registration that still collides with the stdlib during the library's own compile, per C95 — and be module-scoped only, which the shipped `use` machinery already brings into scope). Import-wins precedence for an explicit `::*` / `::(…)` binding is the one resolution rule to confirm against that change.

⚠ **The shipped `use lib;` is NOT the bind this ruling describes — re-measured 2026-09-01.** A
bare `use lib;` wildcard-imports every `pub` name into the unqualified namespace, which is what
the parser says it does (*"Plain `use foo` (no spec) wildcard-imports all pub defs"*) and what
[LOFT.md § Library imports](LOFT.md) documents as deliberate under loft#1094 — the clash rule
there (declaring a name a bare import brought in is refused, naming both sites) only exists
*because* the bare form imports. So the "Already shipped" line above is a status claim that does
not hold, and the two documents have been describing opposite languages: the reference chapter
followed this ruling and told readers for as long as the corpus records that omitting the prefix
is a parse error, while `add(3, 4)` had always answered 7.

What that costs is exactly what the Evaluation section priced: a bare-`use` program is NOT immune
to stdlib growth, because its library names are unqualified. Whether to close the gap by making
the bare form bind only the namespace (this ruling, and a break for every program relying on the
current behaviour) or by superseding the ruling (loft#1094's position) is an owner call, and it is
the one thing here that is still open. The reference now describes the language that runs.

## C99 — A keyed collection's subscript is uniformly KEY-addressed (lookup / range / removal), never positional

**Catalogue:** @F8 (sorted) / @PLN102 arc-E lib-audit **H8** (INC#2). The freeze-time resolution of "the sorted key-range slice shares vector's positional-slice syntax."

### Question

`sorted<T[key]>` (and `index` / `hash` / `spatial`) accept subscript syntax that *looks* like vector's positional indexing: `s[i]`, `s[a..b]`. But on a sorted, `s[a..b]` is a **key-range** query — `s[15..35]` selects the elements whose KEY is in `[15, 35)`, not positions 15..35. The @PLN102 lib-audit flagged this as the "sharpest remaining trap": a `vector → sorted` port writing `s[1..3]` silently reads the wrong elements (an empty result when no key is in `[1, 3)`), and proposed *rejecting positional-shaped slices on sorted*. Should the range slice be rejected or made syntactically distinct before the contract-1 freeze?

### Evaluation

The proposed fix is **incoherent once you look at the whole subscript surface.** A sorted's subscript is key-addressed *everywhere*, not just for ranges (verified both backends, keys `10,20,30,40`):

- `s[20]` → the element with **key** 20 (a key lookup) — **not** position 20 (which would be null).
- `s[1]` → **null** (no key 1) — **not** position 1.
- `s[key] = null` → removes the element with that **key** ([C68](#c68--keyed-collections-dedup-on-insert--and-collkeyvalue), documented).
- `s[a..b]` → the **key-range** query (`sorted_range_positions`), the natural extension of the single-key lookup.

So `s[i]` single-subscript is *already* a key lookup — the core, documented sorted API nobody proposes removing. Rejecting only the *range* form `s[a..b]` while keeping `s[i]`/`s[key]` key-addressed would make the subscript surface **less** consistent, not more, and it would break `spatial<T[x,y]>`, which **deliberately reuses the same range-slice syntax** for proximity queries (`xs[(x,y)..(x2,y2)]`, @PLN48). The visual similarity to vector's positional `v[a..b]` is a **cross-type** footgun that is *inherent to having key-addressed collections at all* — it applies equally to `s[key]`, which is not up for removal. You cannot remove the footgun without removing key-addressing.

### Decision

- **Keep the uniform key-addressed subscript.** For every keyed collection (`sorted` / `index` / `hash` / `spatial`), `coll[k]` is a **key lookup**, `coll[k] = null` a **key removal**, and (where ordered — `sorted`/`spatial`) `coll[lo..hi]` a **key-range / proximity query**. None is positional. This is deliberate and matches [C68](#c68--keyed-collections-dedup-on-insert--and-collkeyvalue) (keyed insert) and the `spatial` proximity API.
- **Consciously ACCEPTED, not fixed** — the design is internally consistent and defensible; the audit's "reject positional-shaped slices" was based on treating `[a..b]` as a special case, but the single subscript is already key-addressed, so there is no inconsistency *within* sorted to fix.
- **Freeze guards:** the key-range semantics are golden-locked in `tests/expressions.rs` (`sorted_range_iteration` — `sum_range(2,4)` over keys `1..4` = `50`, i.e. keys 2,3, *not* positions 2,3 = `70`), and made un-missable by `sorted_subscript_is_key_addressed_not_positional` (keys far from positions). Documented as a Gotcha in `LOFT.md § Key-based collections` and `INCONSISTENCIES.md #2`.
- Owner-reviewable; reverses the lib-audit's H8 lean on the strength of the `s[i]`-is-already-key-addressed finding. Reversible at contract 0 if the owner prefers the reject-and-add-`.range()` path (which would then also have to re-home `s[key]` for coherence and carve out `spatial`).

---

## C100 — `print` stays text-only (no bare `print(value)` or variadic `print`)

**Catalogue:** @PLN13 (beginner-friendly scripts — step 5).

### Question

@PLN13 step 5 proposed making `print(42)` / `println(3.5)` work directly, so a
beginner need not wrap a lone value in a format string (`print("{42}")`). The
follow-on question (raised during the work) was whether to go further and adopt a
Python-style variadic `print(a, b, c)` for multiple values.

### Evaluation

**The any-value capability already exists.** `print`/`println` take `text`, and a
format string interpolates *any* `Printable` through its `to_text` — every scalar, and
a user type once it defines `fn to_text(self: T) -> text`. So `print("{x}")`,
`print("{a} {b} {c}")`, and `print("{p}")` all work today. Step 5 was only ever the
bare-call *sugar* `print(42)` vs `print("{42}")` — three characters.

**That sugar costs core-compiler surgery, because it forces `print` to overload.** loft
free functions cannot be redefined, and a generic `print<T: Printable>` fails to compile
*in the stdlib* (no call site to monomorphise the body — "missing built-in operation").
The only stdlib overload mechanism is concrete `self`-method overloads, which turn
`print` from a global free function into a method and thereby cause two real
regressions:

1. **REPL completion breaks** — `completion_names` excludes `t_…` methods ("not called
   by bare name"), but `print(x)` *is* a bare-name method call, so `print`/`println`
   drop out of tab-completion. Fixing it means broadening the completion model to list
   bare-callable methods (unclear scope).
2. **The @P376 poison-cascade returns** — `print(undefinedvar)` / `print(NoSuchType{})`
   emit a spurious *second* error ("Unknown function print — did you mean the method…"),
   because a poisoned `Never` argument cannot pick among the overloads. A single,
   non-overloaded `print(text)` silences it cleanly. Fixing it means touching the
   parser's call-resolution / error-recovery.

So the bare-call sugar fights three separate core systems (the global-vs-method model,
the completion model, and @P376 error recovery) — the "the fix wants more and more →
the structure is wrong" signal — for a three-character convenience whose capability
already ships via the format string.

**Variadic `print(a, b, c)`** is a further step: loft has no variadic functions (and
lists variadic tuples as a non-goal, [TUPLES.md § Non-goals](TUPLES.md)). It would also
duplicate what the format string already does — and *less* explicitly: `print(a, b)`
hides its separator (Python inserts a space you cannot see), whereas `print("{a} {b}")`
writes the separator in place. loft deliberately keeps ONE string-building tool (the
format string) that serves printing a value, separating several, and appending — rather
than Python's two (variadic `print` + f-strings).

### Decision

**Closed — declined (2026-07-20).** `print`/`println` stay `(v: text)`. The any-value
and multi-value need is met by the format string (`print("{x}")`, `print("{a} {b}")`),
which is loft's single, explicit string-building idiom; documented in
[STDLIB.md § Output and Diagnostics](STDLIB.md#output-and-diagnostics). No bare
`print(value)`, no variadic `print`. @PLN13 step 5 is closed as **delivered by existing
capability** (nothing to build).

### Revisit when

The owner wants the bare-call ergonomic enough to accept a print-specific arg coercion
(keep `print(v: text)` a single global; in the call checker auto-insert `.to_text()`
when the arg is a non-text `Printable`) — the one contained path that adds the sugar
without the method-overload regressions above. That is a deliberate type-checker
special case, not folded into step 5. Variadic `print` would additionally require
reversing the no-variadics stance and is not on the table for contract 1.

## C101 — `std`/`core` are reserved package names; `std::name` is the stdlib's qualified form (the shadow escape hatch)

**Catalogue:** @F2 (operators) / modules + naming. The pre-freeze completion of the
[C97](#c97--)/[C98](#c98--) namespace model — sealing the one global namespace's own
qualified name before the resolution rule freezes with contract 1. Closes @PLN13 phase 6.

### Question

[C97](#c97--) made the stdlib the sole global unqualified namespace and libraries
module-scoped (`lib::name`); [C98](#c98--) makes `use lib::*` / `use lib::name` bring a
library name into unqualified scope, where the imported name **wins** over the stdlib.
Two loose ends the freeze must close: (1) when an imported (or user-defined) name shadows
a stdlib name, is the stdlib symbol still reachable, and under what spelling? (2) nothing
stops a library from being *named* `std`, colliding with that spelling — reserve it?

### Evaluation

Both are already answered in the implementation and only need to be made *intentional*
before they freeze:

- **`std::name` already resolves to the stdlib** — bound in `data.rs`
  (`use_names["std"] = STD_SOURCE`), with qualified resolution falling back to it in
  `use_analysis.rs`. Verified: with a user `enum E` shadowing the stdlib `E`, `std::E`
  still reaches the stdlib ([LOFT.md](LOFT.md)); `std::max(3, 9)` → 9; `std::find("hello
  world", "wor")` → 6 even though a bare `find(a, b)` binds the stdlib *method*
  first-arg-as-self. So `std::` is the symmetric qualified form of `lib::name`, and the
  escape hatch whenever a bare name is shadowed by a user def or a `use lib::*` import.
- **`std` is special-cased in resolution** (`lib != "std"`, `data.rs`), so a library named
  `std` cannot override the built-in binding — but the name is not yet refused at
  creation, leaving a confusing, claimable name. Reserving it is pure pre-freeze
  insurance: no published package is named `std`/`core` (verified), and the resolution
  rule freezes with the contract, so a name that must never be a library is closed now.

### Decision

- **`std::name` is the stdlib's permanent qualified form** and the escape hatch for a
  shadowed bare name (a user def or a `use lib::*` import shadowing a stdlib name — the
  original stays reachable as `std::name`). It mirrors `lib::name`; bare-unqualified
  remains the beginner default ([C97](#c97--)) and `std::` stays opt-in, never required.
- **`std` and `core` are reserved package names** — refused by `loft new` and not
  admissible to the registry (`core` held for a possible future stdlib-core split).
  Canonical list + predicate: `libscan::RESERVED_PACKAGE_NAMES` /
  `is_reserved_package_name`; guard test in `tests/imports.rs`.
- **Not fixed by this:** the [C97](#c97--) residual stands — a library `self:`/`both:`
  *method* named like a stdlib method still errors at definition (methods register as
  attributes on a shared global type, which module-scoping and `std::` cannot cover).
  Free-fn collisions are the real cases and are covered. The path for use-free *library*
  calls is the existing lazy-load trigger surface (`derive_triggers` — method/type
  triggers, @I87), grown by adoption and by completing under-covered trigger kinds
  (e.g. operator overloads), **not** bare free-fn resolution (which C97/C98 declined).
- Owner-directed 2026-07-24. Companion: [C97](#c97--)/[C98](#c98--); closes @PLN13 phase 6.

## C102 — a release binary says nothing when it falls back to the interpreter

### Context

A downloaded release ships no native runtime, so every run without `rustc` on PATH
printed `Warning: native compilation unavailable (…); falling back to the interpreter.
To restore native, rebuild from source (`cargo build --release`) …`. The zero-install
path the README sells hardest therefore opened with a nag telling the user to install
Rust and rebuild — on a run that was about to succeed.

A per-user state marker (`~/.loft/native-notice-shown`) was proposed and rejected: a
fresh container has no `~/.loft`, so the marker fires on the first run of *every*
container — exactly the CI / demo / Docker path where the nag is most visible — and it
puts writable state on a path that must work with `$HOME` unset or read-only.

### Decision

- **Gate on intent, not on state.** The default path is **silent**: it falls back and
  runs. An explanation is printed only when native was explicitly asked for
  (`--native`), where it is an answer to a question the user actually asked.
- **Rationale, once:** someone who downloaded a release binary chose *not* to install a
  toolchain, so "install Rust and rebuild" is not an action they want — it reads as a
  defect report on a successful run. It is a non-sequitur, not a warning.
- **Nothing diagnostic is lost.** Every fallback still records
  `native_fallback_reason`, which `LOFT_REQUIRE_NATIVE` surfaces as a hard error for
  anyone who needs native to have happened. Silence costs no diagnosis.
- The full explanation stays in `--help` and the release `QUICKSTART.md`.
- **Not covered:** the source-checkout auto-rebuild notice (a tree with loft's source
  but no `rustc`) still explains itself — there the message is actionable.
- Sites: `src/main.rs` (rustc-mismatch, rustc-not-found, rustc-launch-failure,
  toolchain-not-usable). Owner-directed 2026-07-24; T0.1 of the first-contact work
  ([FIRST_CONTACT.md](FIRST_CONTACT.md)).

## C103 — `int` / `str` / `bool` are suggested, never legal (no cross-language type aliases)

### Context

Every Python, Rust, Go and C newcomer types `int` in their first hour. loft's type names
are full words (`integer`, `text`, `boolean`, `character`), and the undefined-type error
offered no suggestion. The fix could be a *suggestion* (a diagnostic) or an *alias* (a
language change); this records why it is the former.

Edit distance cannot reach this class at all — `suggest_similar_capped` returns `None`
at ≤ 3 characters (`int`, `str`, `i64`, `f64`) and caps distance at 2, while
`bool`→`boolean` is 3, `char`→`character` is 5 and `string`→`text` is unrelated. So the
suggestion is a table (`data::builtin_type_alias`), not a distance.

### Decision

- **Suggestion only.** `int` / `str` / `string` / `bool` / `i64` / `u64` / `f64` / `f32`
  / `char` / `double` / `long` stay **undefined**; the error names the loft type. The
  legal width types (`i8`/`i16`/`i32`/`u8`/`u16`/`u32`) are deliberately absent from the
  table — a legal name never reaches an unknown-type error.
- **Why full words, from the author (the two intrinsic reasons):**
  1. A newcomer *to programming* has fewer acronyms to learn.
  2. In normal loft code types are already rarely written (inference), and the weight of
     the full word is an **extra nudge not to introduce a type annotation where it has
     no purpose**. A cheap `int` would remove exactly that friction and invite the noise
     the language is shaped to discourage.
- **Supporting reasons this decision also rests on:**
  - An alias is **irrevocable**: compatibility is absolute at contract 1 (additive-only),
    so every alias is a permanent surface commitment bought for a one-time convenience.
  - The obvious-looking ones are **not even true**. `integer` is
    `Integer[i64::MIN+1, i64::MAX]` — `i64::MIN` is the null sentinel — so `i64` is not
    an identity, and `u64` is plainly wrong above `i64::MAX`. That is fine for a
    *suggestion* (which points and lets the type-checker do its job) and unacceptable
    for an *alias*, where the mismatch would be silent.
  - Two spellings tax every **reader** forever to save one **writer** once — the wrong
    trade for a language whose showcase is one readable file.
- **What would reopen it:** evidence from the dogfood loop or a real newcomer that the
  suggestion is not landing — people hitting it *repeatedly* rather than once. The
  suggestion is the reversible experiment; an alias is not.
- Owner-directed 2026-07-24; T0.3 of the first-contact work
  ([FIRST_CONTACT.md](FIRST_CONTACT.md)). Tests: `tests/parse_errors.rs` (`t03_*`).

---

## C104 — a slice on a lazily-bound collection never fetches; a range is an explicit call

**Catalogue:** @F108 (lazy store binding)

### Context

A collection bound to a database derives its query from its own kind: a `hash` gives
equality, a `sorted`/`index` gives a range. Wiring the range shape to the SLICE operation
is the obvious next step — write `events[100..199]` and the rows arrive. It is not built,
and the reason is a conflict inside the model rather than a missing week of work.

A lazily-bound collection answers **"what have I got"**, never "what exists"
([LAZY_STORES.md](LAZY_STORES.md) § `len` and iteration). `len` is the resident count and
iteration walks what has been touched. A slice that silently consulted the source would
make those two answers depend on HOW the collection was reached: `xs[0..10]` would see
rows that `for x in xs` on the next line does not, and `len(xs)` would disagree with both.
The honesty rule that makes a working set comprehensible is the thing the implicit form
breaks.

It is also a bytecode change on a path the interpreter and `--native` derive SEPARATELY
(`DATABASE.md § Adding or changing a collection kind`), where an omission does not read as
a missing feature.

### Decision

- **Closed 2026-08-06.** A slice reads the resident set, like every other read. The range
  is `store_lazy_range(c, lo, hi)` — explicit, visible in the source, and the batching
  claim is fully delivered by it: one query, many rows, asserted by count in
  `tests/lazy_sql_source.rs`.
- This matches the escape hatch beside it. `store_lazy_query` is explicit for the same
  reason — what the collection's kind cannot derive is written down where a reader sees
  the cost, and never inferred from an ordinary-looking read.
- **Revisit when** a consumer shows the explicit call is a real burden in practice AND the
  honesty question has an answer — for example a lazily-bound collection that is
  *streaming* rather than a working set, where iteration and `len` mean something
  different by declaration rather than by accident. Ergonomics alone does not reopen it;
  the model question is the blocker.

---

## C105 — a hash lookup keeps its TWO random reads (no hash-in-slot, no entries in the bucket table)

**Catalogue:** @F7 (`hash<T[keys]>` keyed collection).

### Question

`hash<T[k]>` lookups cost ~180 ns at 1M keys against a reported ~22 ns for .NET's
`Dictionary<long,int>` ([loft#809](https://github.com/loft-lang/loft/issues/809)). Two
designs were proposed against the gap: **arc D**, caching the key's hash in the bucket
slot, and the shape behind Q1's "approach Dictionary" — putting the ENTRIES in the bucket
table so a hit is one random read instead of two.

### Evaluation

@PLN135 arc H shipped the part that paid: entries moved into a chunked arena, insert 1.28x
and bytes/entry 27.67 → 18.6 (−33%). What the two remaining proposals buy was then
measured rather than argued.

**Arc D — cache the hash in the slot.** Its real value is not the lookup; it is removing
the from-scratch re-hash at every resize. Measured on 1M `integer` keys, alternating a
reserved fill against a grown one so the gap IS the resize: 92 ms, 43 ms and 28 ms out of
350–386 ms — a median ~12% of a GROWN insert and **0% of a reserved one**, because
`reserve(h, n)` already removes the resize entirely. The cost is a bucket slot of 8 bytes
instead of 4, doubling the table (5.3 MB → 10.7 MB at 1M), plus a SECOND persisted-store
format break. On a hit it skips nothing: the entry still has to be read to return it.

**Entries in the bucket table.** This is the one design that removes a random read, and
it is the one @PLN135's Q5 already refused: growth reallocates the table, so every entry
moves and every outstanding `DbRef` into the collection goes stale. That is a wrong READ
rather than an error, in the subsystem ranked weakness #1, and
[COMPATIBILITY.md](COMPATIBILITY.md) forbids it. Reference stability is a property callers
have today.

**The baseline is also not what it looks like.** The same 1M lookups cost 99–118 ns in
ascending key order against 175–179 ns shuffled — the ENTRY read becoming prefetchable,
since entries go into the arena in insertion order. Every @PLN135 figure before
2026-08-09 was taken in insertion order, so the reported ~20x compared two sides that may
not have used the same access order. A comparison whose sides differ in that is not a
comparison.

A layout-neutral hoist was written and measured too — `entry_ref` re-read the arena's
directory field and its size header on EVERY probe, both loop-invariant. Alternating A/B
over 1M shuffled lookups: median 214 ns before, 231 ns after. Inside the noise, because
the loop is two cache misses and those reads are hot. Reverted rather than shipped.

### Decision

- **Closed 2026-08-10.** A hash lookup reads the bucket slot and then the entry, and both
  stay random. Arc D is not worth a second format break and a doubled table for ≤12% of a
  case `reserve` already fixes; entries-in-table is not worth reference stability at any
  price. loft#809 is closed on the same evidence.
- `reserve(h, n)` is the supported answer for a fill whose size is known, and it is worth
  saying in a consumer's docs rather than optimising around its absence.

### Revisit when

- A like-for-like re-measurement — **same access order and same working set on both
  sides** — still shows a large gap. The number loft#809 was opened on cannot currently
  support that claim.
- Reference stability into a keyed collection stops being a property loft offers, at
  which point entries-in-table becomes available and would remove one of the two reads.
- A hash's bucket table stops being the memory it is today (e.g. a load factor or slot
  width change lands for another reason), so arc D's doubling is no longer the cost it is
  now.

---

## C106 — `#c` has ONE arity ceiling for both backends, and it was raised rather than lowered

**Catalogue:** @F92 (direct C binding), @F53 (native backend)

### Context

`#c` bindings were checked against `MAX_C_ARITY` on the interpreter ONLY — the check sat
behind `if !native_mode` in `main.rs`. The two backends therefore had different binding
capability, and nobody had measured it: `--native` bound and correctly called a 14-slot C
function while the interpreter refused the same declaration naming `0..=12`.

That is not a capability, it is a portability trap that fires late. The refusal fired at the
CALL site, so a library author could declare a 13-slot binding, build it under `--native`,
test it, and ship — and the failure landed on a downstream consumer who had not written the
declaration and could not edit it. `loft debug` *is* the interpreter, so the bindings you
could not debug were exactly the ones with no other way in.

### The rejected option

**Unify downward at 12.** This was the first recommendation, and it is what "one contract,
and it is the interpreter's" reads like until you check the promise. COMPATIBILITY.md § *The
error surface is one-directional* says adding an error is a break, that pre-freeze the
disposition inverts to "be strict now" — **but** that *"the first resolution of a would-be-error
is a rewrite to correct function, not an error. Erroring is the narrower choice, reserved for
what cannot be given a sane defined behavior."*

A functioning rewrite existed. The interpreter's ladder is one
`extern "C" fn(u64 × N) -> u64` per rung, and every rung above 7 is the same stack-passing
shape as 7 — mechanical, not a new risk per rung. Narrowing `--native` to 12 would have
removed working programs to fix a problem that a loosening fixes for free.

### Decision

- **Closed 2026-08-10.** `MAX_C_ARITY = 32`, enforced on **both** backends. The ladder was
  extended from 12 to 32; `--native` is held to the same number.
- **32 has a reason, not a roundness.** A ladder cannot be unbounded, so past 32 an ANSI-C
  shim is the answer — the stopping point is chosen, not accidental.

  > **The sizing arithmetic here was wrong, and C107 corrected it.** This entry justified 32
  > by "`dgemm_`'s 13 by-reference arguments cost TWO slots each, so it needs 26". The
  > premise was that a `vector` always carries a count. It does not, and more to the point a
  > Fortran routine *takes* no count — measured after this decision closed, `dgemm_` was not
  > bindable at ANY ceiling. Under C107 it costs **13** slots, not 26. The number 32 does not
  > change and neither does the decision; the margin is simply far larger than this text
  > claimed.
- **The tightening half is real but theoretical.** A binding of 33+ C slots that compiled
  under `--native` is now refused. No real C API approaches that; and pre-freeze is the only
  time this error can be added at all.
- **Two-pass enforcement**, mirroring `superseded_fold_diagnostics`: declarations are checked
  in code you OWN (stdlib or entry project), so the author sees it as they write it even if
  nothing calls it yet — while merely LOADING a dependency that declares an over-ceiling
  binding you never call does not fail your build, because a consumer cannot edit someone
  else's declaration. Call sites are checked everywhere, since a call that cannot work must
  be refused wherever it is written.
- The `c-binding-not-interpretable` code KEEPS its name though it no longer means "the
  interpreter specifically". A diagnostic code is a frozen public surface
  ([DIAGNOSTICS.md](DIAGNOSTICS.md)), so the row moved and the name did not.
- **Why this refuses the declaration when the wasm rule does not.** `#c` on the wasm targets
  is refused at the CALL only, deliberately, "so a declaration is still portable"
  ([PACKAGES.md](PACKAGES.md) § *The wasm and browser targets*) — a `#c` binding that wasm
  cannot reach is still perfectly good on native and interpret, so failing the declaration
  would break a legitimate cross-target library. Arity is not target-conditional: an
  over-ceiling binding can never work **anywhere**, so there is no target on which the
  declaration is valid and nothing is lost by saying so at the point it is written. The two
  rules differ because the underlying facts differ, not because the surface is inconsistent.
- **Revisit when** a real C API needs more than 32 integer-class slots — at which point the
  question is whether to add rungs or to generate the shim, and it moves for BOTH backends
  at once either way. Float-by-value does NOT reopen it: there is no cheap ladder move there
  (the trampolines are integer-class by construction), and arc B's pointer idiom covers the
  need.

## C107 — the C signature decides whether a `vector` carries a count, not the loft type

**Catalogue:** @F92 (direct C binding)

### Context

A loft `vector` crossed a `#c` boundary as an element pointer **and** a count, always, because
C carries no length. That is the right shape for a C library written for loft, and it is the
wrong shape for every numeric library that exists.

Fortran passes every argument by reference, so a BLAS or LAPACK routine takes a list of bare
pointers and learns the length from a separate `n` — itself a bare pointer. Measured against a
purpose-built `dgemm_` with the real 13-argument signature, both ways of writing the
declaration failed:

- the **honest** one, naming the thirteen pointers the header shows, was refused for arity
  ("the C signature takes 13 parameter(s), the loft declaration needs 26");
- the one loft **accepted**, with a count after every pointer, delivered each count where the
  callee expected the next pointer — SIGSEGV on the interpreter, nothing at all under
  `--native`.

So no real Fortran routine was bindable, at any arity ceiling. This was not visible from the
fixture, whose C functions were written to loft's shape and therefore held the one axis that
mattered fixed. C106's sizing arithmetic ("`dgemm_` needs 26 slots") is the same mistake
written down.

### The rejected options

- **A shim generator (@PLN128 arc D as originally scoped).** Generate an ANSI-C shim per
  routine that collapses the argument list. It works, but it makes every numeric binding
  depend on a C toolchain at build time, and it was scoped for "routines that overflow 32
  slots" when in fact *every* Fortran routine needed one.
- **C-owned buffers as opaque handles.** Measured working, and it needs no language change at
  all: allocate on the C side, hold each argument as an `integer`, one slot each. It stays the
  right answer for a **retaining** API (it also dodges the E6b use-after-free). It is the wrong
  default for numerics because it copies every array in and out across the boundary — an
  8 MB round trip per call on a 1000×1000 matrix, which is precisely what calling BLAS was for.
- **A new loft spelling** for "pass this without a count". Rejected as unnecessary surface: the
  declaration already carries a full C signature, and that signature is by construction the
  statement of what the symbol takes.

### Decision

- **Closed 2026-08-10.** A `vector` parameter crosses as a **bare element pointer** where the
  C signature has no integer for it, and as **pointer-then-count** where it does.
  `c_signature::plan` derives the assignment once; the declaration check, the interpreter's
  `dispatch` and the `--native` emission all read it, so a vector cannot carry a count on one
  backend and not the other.
- **This is a loosening.** Every declaration that compiled before still compiles and still
  means exactly what it meant — proven by a byte-identical `loft introspect` diff over the
  counted paths. What changes is that declarations previously refused now bind.
- **The assignment is unique when it exists, so inferring it is not a guess.** Take the
  leftmost parameter where two readings differ: it is a vector, counted in one and bare in the
  other, so from there one reading runs a slot behind. To end together some later vector must
  be counted in the reading that is behind — and its *pointer* then lands on the slot the other
  reading uses for a *count*. A slot cannot be both, so the two cannot both type-check.
  `every_reachable_shape_has_at_most_one_reading` searches 12k+ shapes rather than resting on
  the argument.
- **The matching looks ahead rather than walking greedily.** `f(v: vector, sel: integer,
  w: vector)` against `(const double*, int64_t, const double*, int64_t)` has the count on `w`,
  not on `v`, and a left-to-right walk that takes an integer whenever one follows a pointer
  gets it backwards. The fixture's `lc_split_` is position-weighted so that mistake answers a
  different number instead of the right one by luck.
- **What it costs.** A signature that omits a count the C function really takes is now accepted
  instead of refused, and passes a bare pointer where the callee wants a length. That is the
  same class as any other wrong signature — `#c`'s standing contract is that the declaration is
  the sole authority and nothing at runtime can check it (the probe called an arity-1 symbol
  through an arity-3 trampoline and got the right answer). It is not a new kind of hazard,
  but it is one fewer check, and it is the price of being able to name the signature the header
  actually shows.
- **Revisit when** loft grows an address-of, which would let a by-reference scalar be spelled
  without a 1-element vector and make the ergonomics question (@PLN128 Q1) moot.

## C108 — a `vector<T>` and the C pointee are two spellings of ONE layout

**Catalogue:** @F92 (direct C binding)

### Context

C107 made the C signature the authority on whether a `vector` carries a count.  It left the
other half of the same declaration unchecked: **what the pointer points AT**.  A `vector`
reaches C as a pointer into loft's own element bytes, with no conversion on either backend —
`dispatch` hands over the store address and the `--native` emission passes the same one — so
the loft element type and the C pointee are two spellings of a single layout.  Nothing
compared them.

Measured against real OpenBLAS on both backends, exit 0 and no diagnostic:

- `vector<integer>` (8-byte elements) against LAPACK's `int *` pivot array — `dgesv_` answered
  `ipiv = 8589934593, 0` where the pivots are `1, 2`;
- `vector<single>` (4-byte) against `double *` — `daxpy_` wrote **24 bytes into a 12-byte loft
  vector**, a write past the end of a store allocation from a declaration loft accepted;
- `vector<i8>` / `vector<i16>` — loft stores narrow SIGNED elements as `val - min`
  (`database/types.rs`), so a loft `0` is the byte `128` and C reads every value shifted;
- `vector<text>` — the elements are loft's own 4-byte heap handles, so C dereferences a store
  offset as an address: immediate SIGSEGV.  The fixture README had claimed this row worked;
  nothing had ever bound it.

The read-only cases are worse than the crashing one, because on little-endian a wrong width
reads the *right* answer for a one-element scalar and only goes wrong once an array is longer
than one — which is the shape of every bug that ships.

### The rejected options

- **Marshal a converted copy.** Well defined for a `const` pointer, and it hides the question:
  the LP64-vs-ILP64 split is a real property of the installed library that the author has to
  know either way, and copying is what calling BLAS was meant to avoid.  It also cannot work
  for a write-back pointer without a second copy back, and an 8→4 narrowing can overflow.
- **Check signedness too.** Rejected: signedness changes how a byte is read, never where the
  next one is, and `vector<u8>` against `const char *` is the ordinary byte-buffer idiom.
- **Refuse an unresolvable pointee** (`struct foo *`, a function pointer).  Rejected: the
  pointee is diagnostics-only everywhere else, and refusing on it would break `write(2)`'s
  `const void *`.

### Decision

- **Closed 2026-08-10.** A `vector<T>` binds to a C pointer only when the pointee has the same
  **width** and the same **class** (integer vs float) as one loft element.  `void *` — and any
  pointee this binding does not resolve — is **opaque** and always accepted: that is the author
  saying "these are bytes, I mean it", which is how `write(2)` takes a `vector<u8>`.
- **One home.** `CElement::of_loft` reads `data::element_storage_size`, the same function the
  store lays elements out with, so the table cannot drift from the layout; `CElement::of_c`
  resolves the pointee at parse time, where the target that decides C's widths is in hand.
  `c_signature::options` is the only site that matches them, so the declaration check, the
  interpreter's `dispatch` and the `--native` emission cannot form three opinions.
- **This is a TIGHTENING**, and the only one in @PLN128.  COMPATIBILITY.md reserves an error
  for what cannot be given a sane defined behaviour, and this qualifies: there is no correct
  program behind these declarations, only a wrong one that sometimes returns a plausible
  number.  Pre-freeze, and the refusal names the loft type to write instead.
- **What it costs.** `vector<i8>` and `vector<i16>` no longer cross at all (use `vector<i32>`,
  the unsigned sibling, or `void *`), and a numeric author must now write the integer width
  their build of the library uses.  Nothing in this repo or the registry declared any of the
  refused shapes.
- **What it does NOT catch.** The library's own build is still invisible: `daxpy_` declared
  with `int64_t` against an LP64 OpenBLAS is self-consistent and still reads the right answer
  on little-endian for a positive scalar, then corrupts every out-parameter.  Only the header
  the author is reading says which build is installed, which is why PACKAGES.md states it.
- **Revisit when** a `#c` declaration can name the library's integer model, which would let the
  ILP64 question be asked once per package instead of once per parameter.

## C109 — a float RETURN crosses `#c`; a float ARGUMENT still does not

**Catalogue:** @F92 (direct C binding)

### Context

@PLN128 Q3 settled "float by value" once, for both directions, and stayed with the refusal on
the grounds that relaxing it would mean "a second, SSE-aware ladder".  That is true of an
argument and false of a return, and merging them cost more than the decision was worth.

The interpreter calls a `#c` symbol through a fixed ladder of per-arity trampolines,
`extern "C" fn(u64 × N) -> u64`.  For an **argument**, the register file is chosen per
position, so supporting a float anywhere would need a rung per SUBSET of positions —
`2^arity`, genuinely impossible, and unnecessary because Fortran passes everything by
reference.  For the **return** there is one axis: the value is in `xmm0` or it is not.  It
costs one more expansion of the same arity list.

Arc E is what made the cost visible.  Bound against real OpenBLAS, every level-1 BLAS
*function* — `ddot_`, `dnrm2_`, `dasum_` — and every LAPACK auxiliary that answers a number
(`dlange_`, `dlamch_`) was refused, and the cure the refusal named was an ANSI-C shim per
routine: a C toolchain in the build of every numeric package, to work around a boundary that
can just be correct.  That is the trade C107 had already declined for arguments.

### Decision

- **Closed 2026-08-10.** A C `double` return binds to loft `float`; a C `float` return binds to
  loft `single`.  A float **argument** stays refused, with the pointer cure @PLN128 arc B
  wrote.
- **The pairing is exact, not widening.** A C `float` leaves a SINGLE in the return register,
  so binding it to loft `float` would read those bits as a double and get a denormal — hence
  two rungs (`call_at_arity_f64`, `call_at_arity_f32`) rather than one plus a cast.
- **This is a LOOSENING**, which the compatibility promise permits unconditionally: nothing
  that compiled stopped compiling, and the same declaration that was an error is now a call.
- **`--native` needed nothing.** It hands the signature to rustc, which already emitted
  `-> f64` / `-> f32` from `CType::rust_type`; only the interpreter lacked the rung.  The two
  backends were checked against the same hand-computed value (`lc_ddot_`, `lc_sdot_`).
- **The arity list is still written once.** `rung!` and `call_ladder!` are siblings — the list
  is spelled in `call_ladder` and the return type is what varies — because a rung whose
  function type and index list disagree transmutes to the wrong arity with nothing to catch it.
- **Revisit when** someone needs a float argument badly enough to pay for an SSE-aware ladder;
  it moves for both backends at once, as C106 requires.

---

## C110 — a generic's type variable stays in the FIRST parameter, and a keyed collection stays a record set

**Catalogue:** @F26 (interfaces & bounded generics) · @F94 (type-directed interpolation)

### Context

Two proposals arrived together and were evaluated together, because they had been filed as one
plan (@PLN137). They are not one feature, and separating them is most of the answer.

**(a) A type variable outside the first parameter.** `fn hole<T>(self: Acc, v: T)` — a generic
METHOD, whose receiver is concrete and whose type variable is an argument — is refused today:

```
Type variable T must appear in the first parameter — move T to the first parameter position
```

**(b) Multiple type variables**, for a freer keyed type such as `hash<K, V>` or `sorted<K, V>`.
loft does not parse `<A, B>` at all.

Filing them together implied that lifting (a) moves toward (b). It does not: (a) is about which
POSITION a single type variable may occupy, (b) is about how MANY there may be.

### Evaluation

**The first-parameter rule is the monomorph's identity, not a parser convenience.** A monomorph
is named `t_<LEN><Type>_<fn>` from the FIRST argument's type, and that name is load-bearing in
four places: `find_fn` locates a method by it, native emits it as the symbol, `original_name`
parses it back, and the H5 two-pass guard recognises a legal lazy bound stub by it. Lifting the
rule means the name must encode a second type and every back-parsing site follows. C95 already
anticipated this ("when mangling extends beyond the first parameter, this predicate follows it
automatically"), so the cost is understood — it has simply never been worth paying.

**The sole named consumer would be HARMED by (a).** @PLN125's A4 wanted the generic `hole<T>` to
collapse @PLN124's `hole_text` / `hole_int` / `hole_sql_ident` / … family. But the per-kind form
is a safety REFUSAL rather than an accident: *"a kind the target does not define is a compile
error naming the method to add — never a quiet fall back to text, which would put a value back on
the path this exists to close"*. A generic `hole<T>` accepts every type by construction, which
deletes the per-kind opt-in that makes "nothing but `lit` reaches SQL syntax" auditable. The
feature's only justification is a regression in the property @PLN124 was built to guarantee.

**(b) is a missing SPELLING, not a missing capability, and the existing spelling is better.** A
keyed collection is already a dictionary — `hash<Count[word]>` over `struct Count { word: text, n:
integer }` IS `hash<text, integer>`, measured. And the record-set model beats `hash<K, V>` on
three counts:

- the value CARRIES its own key, so the two cannot desync — `hash<K, V>` permits
  `h["a"] = Count { word: "b", … }` and nothing objects;
- a compound key is `[c, t]` by field NAME, not a tuple whose element order must be remembered;
- it is the SAME model across `hash` / `sorted` / `index` / `trie` / `spatial` — one concept
  behind five collections, per C99's uniform key-addressing.

Adding `hash<K, V>` would give one idea two spellings, which is the shape C103 already
refused for type names. The dominant real use — a keyed collection as an INDEX
into a store, pointing at records that hold their own key — is exactly what the record set is for.

### Decision

- **Closed 2026-08-11.** A generic's type variable stays in the first parameter, and keyed
  collections stay record sets keyed on a named field. @PLN137 is closed as declined.
- **The two halves are separable and stay separate.** A future case for multi-parameter generics
  must argue for itself; it does not ride in on generic methods, and neither rides in on
  associated types (@PLN125 arc A shipped those and moved neither, which is what this entry
  records so the conflation does not recur).
- **The `hole_*` family stays per-kind**, and the ugliness is priced correctly: it is paid ONCE
  per target type by a library author, never by a consumer, and it buys a compile error where a
  generic method would silently accept.
- **Revisit when** a consumer wants a generic method AND is not a safety-refusal surface — a
  builder or accumulator that legitimately takes any renderable value — or when a
  multi-parameter generic type has a use that a record set genuinely cannot express. New
  evidence means a real consumer, not a shape that reads more familiar from another language.

---

## C111 — a drop cascade reaches a container's death, not an element's removal

**Catalogue:** @F-drop (`OpDrop`, @PLN125 arc B) · loft#849

### Context

`OpDrop` runs where a value's own free runs. loft#849 is what happens when the value is put
INSIDE something: a struct field, an enum payload, a collection element. A field COPIES at
construction, so wrapping a droppable leaves two records holding one resource — and it is the
SOURCE that drops, while the container's copy is never dropped at all.

The answer chosen is **move-on-construction**: copying a droppable into a container transfers
ownership, so the source no longer drops and the container's death drops what it owns. The
refusal alternative (a type with `OpDrop` may not be a container member) was rejected because the
sanctioned workaround — `disown` after constructing the container — REQUIRES the droppable to be
a field, so a declaration-site refusal rejects its own cure, and with the collection case in
scope it would have to cover `vector<T>` as well.

**Extended to the plain whole-value copy (2026-09-04).** `t = s` is the same step with no
container around it — `@FR-B-Copy` makes it a second record holding one resource — and the
same reasoning gives the same answer: the copy owns, the source stops dropping, once per
resource on both backends.  Measured before the change: `h2 = h` ran the hook twice, `t = s`
twice, a struct-enum arm once only because it aliased (a bind-copy defect closed the same
day), `t = s; return t` three times.  `scopes::copy_moves_drop_from` is the one home; the
`double-move` warning reads it too, so `t = s; u = s` is reported as two owners.  A copy off
a parameter leaves the caller as the owner.  Guard:
`tests/scripts/a-whole-value-copy-of-a-droppable-releases-once.loft`.

**And to the reassignment (2026-09-05, loft#1362).**  A rebind is the owner's death for the
record it displaces, and the hook ran at scope end only — a struct literal assigned to a live
local is rebuilt IN PLACE, so the old record's bytes were gone before anything could read
them.  The release is taken on a null-safe snapshot copied before the statement and dropped
after it (`scopes::displaced_drop`), and the hand-off fact now follows the ASSIGNMENT: a
source copied and then rebound releases through its copy only, and a copy rebound releases
what it displaces.  The rule is `heap.md (H-Drop)`; the OLD value of an overwritten field or
element stays the boundary named above.  Guard:
`tests/scripts/1362-a-rebind-releases-the-droppable-it-displaces.loft`.

That leaves one question, and it is a question about the GUARANTEE rather than the mechanism:
which container deaths run the cascade?

### Evaluation

Every death site with a statically known type can be emitted: a named binding's scope end, a
temp's, a nested field's, a collection's own scope end. One class cannot: an element that dies
**without** its container dying — `v.remove(i)`, or `v[i] = x` overwriting the old element. Those
release through the runtime store cascade (`free_named` walking DbRef fields), so running a drop
there means the runtime calling back into user code — re-entering `State` for the interpreter and
calling a generated symbol for `--native`.

Two things argue against paying that:

- **A drop cannot fail and cannot be observed by its caller** (the @PLN125 arc B contract), so a
  drop invoked from inside a free has no way to report anything either. It would be the deepest
  re-entrancy in the runtime for the least observable effect.
- **The free cascade is the one path the heap invariant rests on** (priority #1). Making it call
  user code — which can allocate, free, and reach the same store — puts arbitrary loft execution
  inside the operation that maintains the invariant.

The cost of NOT paying it is a partial guarantee, which for RAII is the uncomfortable kind: it
looks like it works until someone removes an element. That is real, and it is why this is a
register entry rather than a footnote — the boundary has to be stated where a reader will find
it, not discovered.

### Decision

**Partial, 2026-08-11.** The cascade covers container deaths whose type is statically known at
the death site. **Removing or overwriting a collection element does NOT run the element's drop**
— the resource leaks and the program is otherwise correct. A droppable meant to be released on
removal needs an explicit call (`v[i].close()` before the overwrite), the same way a resource
whose owner closes inside the function body already does.

The rule to state in the contract is therefore not "a drop always runs" but:

> A drop runs when the value's OWNER dies. Taking a value out of its owner does not.

### Revisit when

A consumer meets it for real — a long-lived collection of live resources that is churned rather
than dropped whole (a connection pool that evicts, a cache of open handles). That is the shape
this boundary actually hurts, and none exists yet: @PLN138's registry wraps ONE cursor per
backend and drops it whole. New evidence means such a consumer, not the observation that Rust's
`Vec` drops its elements on `remove` — Rust can afford it because `Drop::drop` is an ordinary
monomorphised call, not a re-entry into an interpreter.

## C112 — a binding position mints a local whatever else carries that name; the function stays reachable as a call

**Catalogue:** @F2 (operators) / modules + naming · [C98](#c98--)'s consumer-facing half · loft#852, loft#756

### Question

A library's public functions occupied the **consumer's variable namespace**. With `use engine_host;`
in scope, `turn = 0` was a compile error anywhere in the consuming program — so adding one public
verb to a library was a breaking change for every consumer that already used that word as a local.
crawler's gate went red across 109 rows, on a commit crawler did not make, when `engine_host` gained
`pub fn turn()`.

[C97](#c97--) and [C98](#c98--) exist to make exactly this impossible for the *stdlib*: the stdlib may
grow additively forever because a library's symbols are module-scoped. The question C98 left is the
same hazard one level up, and it is worse there — a package ecosystem has no chokepoint weighing each
new name, and nothing announces "this release claims the word `turn`".

### Evaluation

**The refusal was never a rule loft held.** It held in exactly three binding forms — `name = …`, the
typed local `name: T = …`, and a tuple-destructuring element — all three of which route through one
path, the bare-function-reference fallback in `parse_var`. The other three binding forms never
refused, for the stdlib either:

```loft
for chr in 65..67 { chr(chr) }   // binds a local AND calls the function → "A", "B"
fn go(chr: integer) -> text { chr(chr + 1) }
struct S { chr: integer }
```

So loft already keeps **values and functions in separate namespaces**, and the parentheses already
pick between them. The three refusing forms were not enforcing a namespace; they were reporting a
parser-recovery problem (@P335/@P392: the function-ref left the `:` or `=` unconsumed and the author
saw a confusing `Expect token ;`). Binding is what supplies the `Value::Var` that recovery wanted.

The alternative — keep the refusal and let the consumer rename — is what crawler did, and it is a
clean workaround for the instance. It is not an answer to the class: the cost lands on the consumer
with no local change, and it is not available in advance. You find out which words are forbidden by
compiling against each new library release.

**This does not re-open [C95](#c95--).** C95 refuses a top-level *function definition* that a method
would silently shadow, because that definition is dead code the author believes runs. A local binding
is scoped to one function body, re-points no other call site, and is not silent — the author wrote
the name. The three forms that never refused are the proof that the language already accepted that
reasoning.

### Decision

- **A name at a binding position mints a local, whatever else carries that name.** All six binding
  forms agree: assignment, the typed local, a tuple-destructuring element, a parameter, a `for`
  variable, a struct field.
- **The function stays reachable as a call in the same scope** — `chr = 65` beside `chr(65)` — and a
  bare mention reads the local once one is bound. Parentheses pick the namespace.
- **A library's new `pub fn` therefore cannot break a consumer's locals**, which is C97's guarantee
  extended from the stdlib to packages, where the argument for it is stronger.
- **Rejected:** keeping the refusal (makes every short verb a library exports a word its consumers
  may not use, revoked on someone else's release); a diagnostic naming the collision instead (loft#756
  earned that message, and #852's point is that the cost is not the message — there is nothing the
  consumer can do in advance).
- Implemented 2026-08-11 in `src/parser/objects.rs` (`parse_var`: the function-ref fallback yields at
  a binding position; the pass-2 placeholder rescue beside it takes the same guard). Pinned by
  `tests/scripts/852-local-shadows-a-function-name.loft` — which carries the parameter / loop / field
  cells as CONTROLS, so a change that freed the three forms by breaking the three that already worked
  fails there — and `pln102_c98_a_local_may_shadow_a_library_function` (`tests/imports.rs`, the
  library half, both backends).

### Residual — C98's own half is still open

Measured while fixing this, and unchanged by it: **a bare `use lib;` still wildcard-imports every
public name into the unqualified namespace** (`src/parser/mod.rs`, `None => Some(ImportSpec::Wildcard)`),
so `turn(3)` is callable unqualified after `use mylib;`. C98 rules that it must bind only the `lib`
handle. That migration is mechanical for a consumer (`use lib;` → `use lib::*;`) but it is a
**breaking** resolution change, it must land pre-freeze, and it breaks out-of-repo consumers on the
day it lands — so it is owner-timed work, not a fix to fold into a bug. C112 is independent of it and
forward-compatible with it: once a bare `use` stops importing, a local named after a library function
is legal for a second reason as well.

## C113 — two `index` collections over one element type and key are refused, not made to work

**Catalogue:** @F9 (`index<T[keys]>`) · loft#902 · loft#843 (linked collection groups)

### Question

Two or more collections over one element type in one struct are auto-linked into several
routes to a SINGLE record set (loft#843). When two of them are `index` with the same key,
the first removal panics in `src/tree.rs` — *"Child of single-child node should be red"*.
Should each index FIELD get its own link storage in the element record, or should the
construct be refused?

### Context

An `index` is the one collection kind that keeps per-collection state **inside the element
record**: a red-black tree's `#left_N / #right_N / #color_N` triple is appended to the
element type when the index type is created, and `Parts::Index` records the triple's
offset. Every other kind keeps its state in storage of its own — a hash in its table, a
`sorted`/`ordered` in its slot list, an `array` in its slots — which is exactly why
`hash` + `hash` and `sorted` + `sorted` are two working routes to one record set and
`index` + `index` is not.

The triple is allocated **per index TYPE**, and an index type is named after its content
and its keys, so two fields declared `index<E[k]>` resolve to one type and therefore one
triple. They are not two trees: they are ONE tree reached through two roots. That is why
the FILL looked correct — both roots walk the same structure, so lengths and iteration
agree — and why the first removal, which rebalances through one root and leaves the other
stale, is where it comes apart.

Measured scope: only `index` + `index` **with the same key** collides. A different key is
a different type name and so its own triple (`index<E[k]>` + `index<E[n]>` fills and
removes correctly, both routes); two `index<E[k]>` fields in different structs hold
different record sets; and `index` beside `hash` / `sorted` / `vector` is correct as of
loft#900.

### Evaluation

**Per-field link storage** would keep the construct working. The layout machinery already
supports N triples per element type — the instance counter exists — so the storage is not
the obstacle. The obstacle is IDENTITY: the parser resolves a collection field to a db
type through its declared loft `Type`, and two fields with the same declared type are
indistinguishable there. Giving them separate storage means minting a db type per FIELD,
which changes what a schema id names — and schema ids are replayed by the native backend's
generated `init()` (`LOFT_STRICT_SCHEMA_IDS`), serialised into the IR, and round-tripped
through snapshots. That is a schema-identity change across four layers.

**What it would buy is nothing.** A second index with the same key over the same records
answers exactly what the first answers, in the same order. There is no query the pair
serves that one index does not, which is what separates this from `hash` + `sorted` (two
different orders / two different lookups over one record set — a real pairing, and the one
DATABASE.md documents by name).

**Compatibility** cuts the way it usually does, but not here. A program that declares such
a pair today and only ever fills it does get a consistent answer — from one tree seen
twice. It has never had a working removal, and a panic is the outcome loft's contract
("no runtime errors, EVER") ranks worst. Refusing at the declaration converts an unfixable
runtime panic into a compile error that names the cure.

### Decision

**Closed — refuse, 2026-08-14.** `Parser::reject_duplicate_index` rejects a second `index`
field with the same element type and the same key in one structure (or enum variant),
where the field is declared:

> `'b' and 'a' are both `index<E[k]>` in the same structure — two indexes cannot share
> records, because an index keeps its tree links in a field of the record and one field of
> links cannot hold two trees. Give the second route a different kind (`hash` or `sorted`
> over the same records), or index a different key.

Both cures are one word and both are already correct. The refusal's SCOPE — a different
key, a different struct, a different kind — is pinned by
`tests/scripts/902-duplicate-index-allowed.loft`, which must stay green beside
`902-duplicate-index-refused.loft`; a widened check that caught any of those rows would
fail there rather than silently reject working programs.

### Revisit when

A consumer needs two indexes over one record set with the same key and genuinely different
BEHAVIOUR — the only shape that would make the pair mean something. A partial-key or
filtered index (`index<E[k]> where …`) is that shape, and it would arrive with its own
type name anyway, so it does not reopen this: it lands as a new kind, not as two copies of
one.

## C114 — a keyed collection is refused as a vector ELEMENT, not given an element form

`vector<hash<E[k]>>` — and the same with `sorted` / `index` / `spatial` / `trie` — parsed,
and did nothing else that worked.

### What was actually there

The type could be declared and only declared. Every route to putting something in it
failed, each in a different way:

* a literal element typed as the CONTENT struct — `vh += [[E{k:1,v:10}]]` answered
  *"cannot store `vector<E>` elements in a `vector<hash<E,["k"]>>`"*, and there is no
  other spelling for a keyed literal;
* appending a keyed LOCAL (`h: hash<E[k]> = []; vh += [h]`) walked off the type table and
  panicked the compiler;
* `--native` never got that far. `emit_type_creation` has arms for `Struct` / `EnumValue`
  / `Enum` / `Vector`, and a keyed kind reached as a vector's CONTENT matched none of
  them, so the generated `init()` referenced a `t{N}` no line ever bound and rustc refused
  the program (E0425).

The interpreter's tolerance is what made it look like a feature: `len(vh)` answered `0`
and the program ran.

### Why not implement it

The obvious repair — give `emit_type_creation` a keyed arm — creates the keyed type at the
CONTAINER's position rather than its own, and a keyed type is created once. That moves
every runtime id after it, for **every program with a keyed field**, which is precisely
the loft#739 hazard the schema-id check exists to catch. So the cheap fix is not cheap; it
trades a refusal for a silent id drift across the whole language.

And the element form is the real question underneath. A keyed collection is a record set
with a spine; a vector element is a slot. Giving one an element form means deciding what a
vector of them OWNS, what a copy of the vector does to the spines, and what
`borrowed_spine` should free — the same lifetime surface loft#898 needed for a group. That
is a feature with a design, not a missing arm.

### Compatibility

Nothing to keep. No program could fill one, so no program depends on the behaviour; a
sweep of `default/`, `lib/`, `tests/` and `doc/` found no declaration of the shape outside
the issue that reported it. The refusal converts a compiler panic and a rustc error into a
compile error that names the cure — the same trade [C113](#c113--two-index-collections-over-one-element-type-and-key-are-refused-not-made-to-work) makes.

### Decision

**Closed — refuse, 2026-08-15 (loft#923 leg A).** The `"vector"` arm of
`Parser::sub_type_inner` — the one chokepoint every inline `vector<…>` element resolves
through, so local, parameter, return, field and nested all reach it — rejects a keyed
element where it is written:

> a `hash` cannot be a vector ELEMENT — a keyed collection has no element form anything
> can write, so `vector<hash<…>>` could only ever be declared and stay empty. Hold it in a
> struct and make a vector of THAT: the extra record is what the element would have been
> anyway.

The message names the KIND rather than `Type::name`, because a keyed type's registered
name carries its key list in the schema's spelling (`sorted<E,[("k", true)]>`) and that is
not what the author wrote. All five kinds are covered by
`tests/parse_errors.rs::every_keyed_kind_is_refused_as_a_vector_element` — naming three of
five is how loft#922's field-replace path left `spatial` and `trie` broken.

The prescribed cure is verified on both backends, not just asserted: a `struct Box { by_k:
hash<E[k]> }` in a `vector<Box>` fills, looks up and reads back identically under
`--interpret` and `--native`.

### Revisit when

A consumer wants a vector of keyed collections and the struct wrapper is measurably in the
way — which means the wrapper's extra record is the cost being complained about. That is a
layout question with an answer (an inline element region), and it arrives with the
ownership rules `borrowed_spine` would need, not as a new arm in the emitter.

---

## C115 — a closure cannot write through a captured `&` scalar parameter, and cannot rebind a captured heap parameter

Two composition edges, one wall, one answer. `(L-CapRef)` + `(F-ParamRef)` read together say a
write to a `&` scalar parameter from inside a closure reaches the caller; `(L-CapHeap)` +
`(F-ParamRebind)` read together leave it open whether a whole-value replace written inside a
closure stops at the callee. Both are REFUSED at compile time.

### What was actually there

Neither edge was a missing feature — each was a program that compiled and answered quietly wrong.

* **The `&` scalar (loft#1276).** `fn bump(p: &integer) { g = fn() { p += 1; }; g(); p = p + 10; }`
  on `n = 5` answered **15** where 16 is correct: the closure's increment dropped through a
  parameter whose whole purpose is the write-back.
* **The heap rebind (loft#1281).** `fn repl(p: vector<integer>) { g = fn() { p = [7,7]; }; g(); }`
  left the caller's `[1,2]` as `[7,7]`, on both backends, with nothing reported — while the
  identical rebind written *without* the closure correctly left it alone. Every heap kind did it
  (vector, keyed, struct) and every right-hand side (a literal, a call, another local).

### Why not implement it

The same wall, one rule apart, and it was measured rather than assumed.

A `&` scalar lives in the caller's slot and `(L-CapScalar)` gives the closure a **copy** of the
value, so there is no shared record for the write to land in. Making it mean what it says needs
the REF itself in the closure record plus a write-back — and the cell machinery, the mechanism
that gives a mutated captured *local* scalar exactly that channel, cannot supply it here: reads
in the enclosing body would then see the cell while the caller still sees its own slot.

The heap rebind hits that wall from the other side. The closure record holds a copy of the
parameter's DbRef, so the rebind lowered to a clear plus a refill of the store that copy names,
and `(F-ParamHeap)` makes that store the caller's. The obvious repair — repointing the capture
slot — was tried and **does not fix it**, because the callee reads its own slot directly
(`t_6vector_len(p(0))`, not a read through the record). Two readers, one binding: a repoint moves
the wrong answer from the caller to the callee instead of removing it.

So the cure in both cases is the binding reachable from inside the closure *plus* a write-back —
a capture mechanism the language does not have, not an arm someone forgot to write.

### Compatibility

Nothing correct depended on either shape: both compiled to an answer the rules call wrong, so the
refusal converts a silent wrong answer into a compile error. This is C79's
*decline-what-we-cannot-implement-safely* revisit applied twice, and its forward-compatibility
reason holds here too — an error can be dropped later, a silently different semantics cannot.

The refusals are narrow, and every neighbouring shape still works: a captured **local** (no caller
to reach past), a `&` parameter that is *read* (`(L-CapRef)`), a scalar or `text` parameter (the
cell machinery), and every mutation-**through** a captured heap parameter — `p += [x]`, `p[i] = v`,
`p.clear()`, a field write — which `(L-CapHeap)` and `(F-ParamGrow)` require to reach the caller.

### Decision

**Closed — refuse, 2026-09-02 (loft#1276, loft#1281).** Recorded here rather than in
[formal/closures.md](formal/closures.md) because
[formal/ROADMAP.md](formal/ROADMAP.md) says a row that turns out **spec-may-adjust** leaves
`formal/` and becomes a decided edge: no code change will ever close these, so counting them as
distance from the spec misreports the register. The rules keep their pointer here in place of a
deviation number, and `(L-CapRef)` states the carve-out in its own text.

* `&` scalar write — *"Cannot write to the `&` parameter 'p' from a closure"*; guard
  `tests/scripts/1276-reject-a-ref-parameter-a-closure-cannot-write.loft`, whose read-only cells
  are the control that says widening the write-walk to credit a lambda's writes did not also
  credit its reads.
* heap rebind — guard
  `tests/parse_errors.rs::a_closure_cannot_replace_a_captured_heap_parameter` (all three
  right-hand sides × vector, hash, struct). The shapes it must **not** reach live in
  `tests/scripts/1281-a-closure-cannot-replace-a-captured-parameter.loft`, which cannot hold the
  refused spelling because the fixed compiler will not parse it.

### Revisit when

A capture mechanism that carries the BINDING rather than a copy of its value exists — the same
piece of work `formal/closures.md` D-clo-7 / D-clo-14 and `formal/ownership.md` D-own-16 need
(QUALITY.md's per-execution ownership witness cluster). If that lands, both refusals become
implementable in one step rather than two, and this entry is the evidence that they are one
question. Nothing smaller reopens it: a repoint of the capture slot has already been measured
and moves the defect rather than removing it.

## C116 — a collection element holds a plain fn-ref, never a capturing closure

`vector<fn(integer) -> integer>` and every keyed collection over a fn type REFUSE a capturing
closure, at the literal and at an element assignment alike — a direct lambda, a local that
holds one, and a closure factory's return. A struct FIELD holds a capturing closure without
trouble, and that is the route the refusal names.

### What was actually there

The refusal has stood since loft#247, but its message said *"not supported **yet**"* and
pointed at a deferred layout plan (@P213/@P214), the same text lived in two parser sites, and
a canary (`tests/threading_chars.rs::par_vec_of_capturing_fns_t4`) sat `#[ignore]`d *"until the
underlying vector storage works"* — a promise of a future the register had never decided. The
user chapter (`tests/docs/26-closures.loft`) already stated the rule without the "yet".

### Why not implement it

A collection has ONE element layout, and a fn-ref element is the four-byte definition number a
call dispatches on. A capturing closure is that number PLUS an environment record whose shape is
its capture set — different for every lambda, even two of one signature (`|x| x * a` and
`|x| x * b` capture different cells). Giving the element slot a second half (the "co-located
closure record" the old message deferred to) would widen every fn-typed collection for the one
case that captures, and would make the environment a record the COLLECTION owns — freed on
element removal, copied on vector copy, moved on sort — a second ownership discipline for one
element kind. A struct field has none of that: its closure record is co-located in the struct
(C75), so the state travels with the value that reads it. The refusal converts a runtime crash
(*"Write to read-only store"*, the path the old comment recorded) into a compile error — C79's
*decline-what-we-cannot-implement-safely* again, and forward-compatible in the same direction:
an error can be dropped later, a silently different semantics cannot.

### Compatibility

Nothing that compiled changes: the refused shapes never ran. The message loses the word "yet"
and the plan pointer, and names the struct-field route. `#318` (a closure inside a struct that a
collection holds) is the same wall one layer up and stays refused; the route is a struct that is
NOT itself an element.

### Decision

**Closed — refuse, 2026-09-04 (loft#1358).** ONE home for the refusal,
`Parser::refuse_capturing_closure_in_collection` (raised by the collection literal and by the
element assignment); `par_vec_of_capturing_fns_t4` now guards the refusal instead of waiting
for a feature; [formal/closures.md](formal/closures.md) keeps its one-line note and cites this
entry. Per-element state goes in a struct whose field carries the closure:
`Stepper { advance: fn(x: integer) -> integer { x + step } }`.

## C117 — a linked group's members must share one element LAYOUT; the tag-aware dense read is declined

A struct declaring both a dense `vector<E>` and a `vector<E?>` beside a keyed member is
REFUSED.  The alternative — the group adopting the tagged layout, with every dense read
skipping the discriminant — is declined.

### What was actually there

Two rules that cannot both hold.  `(Col-Group)` said membership is *"not about whether the
element is dense (`vector<E>`) or nullable (`vector<E?>`)"*, so all three members are routes to
one record set.  `(N-Dense)` says a `vector<E>` stores `E` and its elements are non-null unless
the author wrote `vector<E?>`.  One record set that may hold absence cannot be read through a
non-null element type.

Both silent answers were measured before the refusal was chosen (loft#1385).  Left alone, the
dense member falls out of its own group: a write through it reaches nothing else, and `len` of
the member that never received the record is a legal `0`.  Made to join — by comparing the
element through the nullable peel (`Stores::key_owner`), which is the obvious fix and does make
the group form both ways — the dense member RECEIVES the record and misreads it: `a[0].n`
answered `7` and `a[0].k` answered `2`, the `Some` discriminant.  That is loft#1134's misread:
a zero turned into garbage, which is worse than the zero.

### Why the tag-aware read is declined rather than deferred

Making it work means the group adopts the tagged representation and every read through a dense
member skips the discriminant.  That gives `vector<E>` elements a layout the author did not
write, and `(N-Dense)`'s promise — that a `vector<E>` stores `E` — stops being true of the
bytes.  It is a feature nobody asked for, bought by making a written type mean something else.

So the refusal is the answer, not a placeholder: the message names both cures (`a: vector<E?>`
to join the group, or drop the keyed member that groups them), and `(Col-Group)` now states the
layout condition rather than leaving it to be re-derived.  Same direction as `D-bind-17`'s `&τ?`
decline, and what the freeze axis wants — both alternatives are silent, and a refusal is loud.

Guards: `tests/scripts/1385-a-group-cannot-hold-one-element-two-ways.loft` (the refusal, and the
note recording that all four declaration orders were measured) and
`1385b-a-group-agreeing-on-nullability-still-forms.loft` (the four shapes it must not cross —
no keyed member, all-nullable, all-dense, and a member's own `?`).

## C118 — an append to an ABSENT collection instantiates it empty and fills it; refusing or warning is declined

**Catalogue:** @F1 (null model), @F38.

**Question.** `c += [x]` where `c` is a nullable collection holding null compiles silently and
mints an empty collection before inserting.  loft#1434 asked whether it should instead be
refused with the discharge message a `for` over the same value gets — since `formal/types.md`
says there is no implicit `τ? ⤳ τ`, and this is the one write that performs a `(N-Default)`
discharge nobody spelled — or at least warn.

**Evaluation.** Where the absent collection comes from decides the weight of the question:
nearly every path starts a collection empty, and the one producer of a null collection FIELD
is decoding — a JSON document whose key is missing (owner, 2026-09-07).  The consumer of a
decoder then appends to the field it just read, and the append is the whole of what it wants;
a rule that made it write `s.items = s.items ?? []` first would bill every such program for a
distinction the insert cannot observe.  Three positions were measured on one nullable
collection (both backends):
an EXPRESSION propagates the absence into its result's `?` (`(N-Prop)`), a CALL warns at the
argument (`(N-Store)`), and a STATEMENT is refused because it has nothing to carry the null.
The append is the odd statement: unlike `for`, it binds no dense element, so no unwrap ever
happens — its only question is which store the record joins, and an absent collection has one
sensible answer.  Refusing would be a new error on code that compiles today, for a semantic
that is already the natural one; warning would bill the author for a default the language can
state.  `COMPATIBILITY.md`'s rule for contract 0 — a would-be error is first REWRITTEN to a
correct function, and erroring is reserved for what has no sane defined behaviour — points the
same way.

**Decision.** Accepted as the contract, 2026-09-07 (owner: *"a nulled field `+= [x]` is
instantiated empty and filled with the value"*).  Recorded as `(Col-Insert-Absent)` in
`formal/collections.md`, cross-referenced from `(N-Default)`.  No diagnostic.  The `for`
refusal and the `remove` refusal (same home, loft#1434) are unchanged: they bind or address an
element, and an absent collection has none.

**Revisit when.** A consumer shows an append that reached an absent collection it believed
present, and the silent instantiation is what hid the defect — that is the evidence a WARNING
would need, and it would be `advice`-tier (the result is what the language documents).

