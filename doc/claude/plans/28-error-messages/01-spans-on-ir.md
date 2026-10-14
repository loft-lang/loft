<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 1 — Spans on IR

Status: **done 2026-07-07** — the error-message goal is met; the 5
"remaining" wraps (1.14-1.18) are unnecessary, see § Resolution
2026-07-07.
Wrap landings on `roadmap-lsp-eclipse`:

| Step | Commit | Coverage |
|---|---|---|
| 1.A — `Value::Span` variant | `b3b2da2` | enum + walker passthroughs |
| 1.B foundation — `with_span` / `unspan` | `4904669` | helpers |
| 1.B.0 — 12-site `unspan()` audit | `c63a169` | initial audit |
| 1.B.1 — binary `/` `%` wrap | `f6a3227` | wrap activated; 4 walker arms fixed |
| 1.11 + 1.12 — `[` and `.` wraps | `4c571aa` | vector index + field access |
| 1.13 — Call / CallRef wrap | `147a422` | every user fn-call site |
| 1.20 — pc → source-position table | `4324cc9` | codegen populates `source_spans` |
| Phase 3 hook + extended `+ - * << >>` | `a17eb55` | runtime panic prints `at file:line:col` |
| Phase 6 starter — type-mismatch messages | `09acf02` + `<this commit>` | "expected E, got G" + "cannot iterate over T" |

Remaining wraps (each small, mechanical, walker discipline already in
place — surface ≤1 new pattern-match site per wrap): 1.14 Set, 1.15
Iter, 1.16 Return, 1.17 struct lit, 1.18 narrow cast.

## Resolution 2026-07-07 — the 5 remaining wraps are unnecessary

Verified (each node tested for caret quality on the current tree): every
one of the five leftover nodes **already delivers a precise `file:line:col`
+ caret**, because the diagnostic sites evolved (phases 5-6) to capture
their own `Position` via `diagnostic_at!` rather than relying on the
`Span`/`current_span` fallback:

| Node | Diagnostic | Caret verified |
|---|---|---|
| 1.14 Set | `Variable 'x' cannot change type from integer to text` | `=`-assignment column |
| 1.15 Iter | `cannot iterate over integer; expected …` | the iterable |
| 1.16 Return | `expected integer, got text on return` | the return value |
| 1.17 struct lit | `Cannot assign text to field P.v of type integer` | the field value |
| 1.18 narrow cast | `narrowing cast … may not fit at runtime` (case 24) | the `as` expr |

And none of the five **faults at runtime**: the fault-prone ops that do
(div, mod, index, field, call — 1.9-1.13) are already `Span`-wrapped and
handled by phase 4's @C66 log-and-continue (verified: `100/d` with `d==0`
→ `null` + continue; `v[10]` → `null`).  A checked narrowing cast
`as i32?` returns `null` by contract (no fault to locate); loft has no
silent implicit narrow.

So wrapping Set / Iter / Return / struct-lit / cast would attach a
`Position` that **no consumer reads** — phases 2/3/6 already have precise
positions from direct capture — while wrapping `Value::Set` (matched
throughout the second pass) reintroduces exactly the `unspan()`
pattern-match churn + regression risk that the 1.B saga documents above,
for zero user-visible gain.  The wraps were speculative infrastructure
(the design listed Set/Iter/Return/struct-lit as position-carriers before
the diagnostic sites learned to capture positions themselves); that need
never materialised for @PLN28's error-message goal.

**Acceptance re-checked against reality:** every fault-prone-at-runtime
construction wraps in `Span`; every walker has its `Span` arm (`cargo
test` green); `Definition.source_spans` populated (1.20); the
`error_messages` goldens all carry `file:line:col`.  Phase 1's
error-message acceptance is met.  (If a future LSP/hover feature — the
`roadmap-lsp-eclipse` context — wants a span on *every* node regardless
of fault, that is new scope under an LSP plan, not @PLN28.)

The phase-3 hook (commit `a17eb55`) is the first user-visible payoff:
running a loft program that panics at runtime now prints
`  at file:line:col` before the panic message, sourced from the
codegen-populated `state.source_spans`.  Verified end-to-end on
arithmetic overflow (`big * big`), the `panic(...)` builtin, and
`assert(...)` failure.

## Resolution 2026-04-29 — four walker arms, not a wide audit

The 12-site `unspan()` audit from 1.B.0 was a red herring.  The
state-corruption bug ("sorted descending key produces ascending
order, then segfault") came from **four walkers in the second pass
that pattern-match `Value` exhaustively but treated `Span` as a
no-recurse leaf**:

| Walker | File | Symptom |
|---|---|---|
| `compute_intervals` | `src/variables/intervals.rs:199` | `last_use` for vars wrapped inside a `Span` was never updated → assign_slots aliased the slot, runtime read freed memory.  This produced the canvas demo's `3751172305368004 * 3751172305368004` overflow and the bounded-generic test's huge garbage values. |
| `Convert::scan_inner` | `src/scopes.rs:459` | `var_mapping` rewrites of `Var` references inside Span never ran → second-pass loops referenced the *previous* loop's `e` (slot 128 vs new slot 152).  This was the original "sorted descending → ascending" failure. |
| `substitute_type_in_value` | `src/parser/mod.rs:1418` | Generic specialisation re-resolves `Call` targets but skipped Span-wrapped calls → `t_5Score_halve` called the abstract stub `t_1T_OpDiv` instead of the concrete `t_5Score_OpDiv` (bounded-generic interface tests). |
| `place_large_and_recurse` / `walk_node` / `walk_frame_bases` | `src/variables/slots.rs`, `src/variables/slots_v2.rs`, `src/variables/validate.rs` | Defensive Span passthrough — current `/` `%` wraps don't put Set/Block under a Span, but adding the arm matches the discipline of every other walker and prevents future wraps from regressing. |

Each walker gets a one-line `Value::Span(b) => recurse(&b.1, ...)`
arm.  The `Value::unspan()` helper from 1.B.0 stays — it's the
right tool for sites that pattern-match `Call` shape (operator
optimisation, lock-call rewrite) rather than walk recursively.

Delivered:
- All 47 wrap-suite tests + 119 expressions tests + 539 issues
  tests + 3 spans_on_ir tests + 2 error_messages tests green.
- `binary_div_wraps_in_span` and `binary_mod_wraps_in_span`
  un-ignored.
- Canvas demo (`lib/graphics/examples/10-2d-canvas.loft`) saves
  PNG cleanly.
- Bounded-generic operators (interface `op /` / `op %`) compile and
  run correctly.

## Lessons learned 2026-04-28 — superseded by 2026-04-29

The 12-site `unspan()` audit (still useful for site-shape pattern
matches; see `src/parser/operators.rs:208` and
`src/parser/collections.rs:296` for examples) was based on the
false hypothesis that *all* second-pass consumers needed unspan().
The actual gap was four IR walkers missing a Span arm — see
"Resolution 2026-04-29" above.  Original notes preserved:

## Lessons learned 2026-04-28 — `Value::unspan()` is a prerequisite

The first activated wrap (binary `/` and `%` in
`parser/operators.rs::handle_operator`) broke 4 tests in
`tests/wrap.rs` (`collections`, `loft_suite`, `stress`, `vectors`).
All four failures traced to second-pass code that pattern-matches
`Value::Call(d_nr, args)` directly, e.g.:

- `src/parser/collections.rs:67-68` — vector compound-assignment
  optimisation peeks at the LHS shape.
- `src/parser/collections.rs:296-389` — `compute_op_code` and
  `call_to_set_op` extract the LHS def-nr and args.
- `src/parser/operators.rs:40-69, 749` — append-text and
  call-coercion both inspect `Value::Call(_, parms)`.
- `src/parser/expressions.rs:550, 966, 1331` — lock-call
  rewriting + lambda-arity checks pattern-match Call.

When a `%`-call is wrapped in `Value::Span(box (pos, Call(...)))`,
every one of these patterns silently misses, and the optimisation
falls through to a fallback that produces wrong runtime behaviour
(the `for v if v % 2 == 0 { v#remove }` filter went from "remove
even" to "remove all").

**Therefore** the order in the original landing sequence is wrong:
the per-site wraps in 1.9-1.18 must NOT land before a
`Value::unspan(&self) -> &Value` helper exists AND every Value-Call
pattern match in second-pass code is updated to call it.

### New step 1.B.0 — `Value::unspan()` helper + pattern-match audit

Insert before 1.9:

| # | Step | Test |
|---|---|---|
| 1.B.0 | Add `Value::unspan(&self) -> &Value` returning `b.1.unspan()` for `Span` and `self` otherwise.  Audit the ~12 sites listed above (and any others surfaced by the failing-test triage) and route their pattern matches through `unspan()`.  No wrap activated yet — this commit is pure infrastructure. | `tests/wrap.rs` stays green when run with a hand-injected `Value::Span` wrapper around the LHS Call of a compound assignment (one synthetic IR test in `tests/spans_on_ir.rs`). |
| 1.B.1 | Activate the binary `/` `%` wrap in `handle_operator`.  This commit thread `op_pos` through and wraps the call. | `tests/spans_on_ir.rs` un-ignores `binary_div_wraps_in_span` and `binary_mod_wraps_in_span`.  Full test suite green. |
| 1.11 | (renumber) Wrap vector index `v[i]` | … |

After 1.B.0 and 1.B.1, the rest of the wraps (1.11-1.18) follow
the original plan but each wrap commit must include a "pattern-match
audit" line in its description listing every site updated.

### Update 2026-04-28 (second attempt) — audit list is wider than 12 sites

A second activation attempt landed `Value::unspan()` and routed all 12
sites listed above through it.  All previously-failing wrap-suite tests
(`collections`, `loft_suite`, `stress`, `vectors`) STILL failed with
the same div/mod wrap active — but with a NEW failure shape:

> `tests/scripts/12-collections.loft:182` —
> `assert(rs_fwd == 321, "sorted forward desc order: {rs_fwd}")`
> printed `123`, not `321`

The descending-key sort was producing ascending order.  And after that,
the next sub-test crashed at
`src/database/allocation.rs:287` with an out-of-bounds access.

This is NOT a div/mod wrap bug — the failure surface is unrelated to
`%`.  It surfaces when a `for ... if ... %` loop runs over a `sorted`
collection with a descending key (`sorted<T[-key]>`).  Hypothesis: the
sorted-iteration codegen reads the if-condition's IR shape at a site
not yet in the audit list, so the wrap leaks state corruption into
the iterator's internal cursor.

**Therefore** the audit list grows in step 1.B.0:

- Step 1.B.0 must enumerate every `match val { … Value::Call(...) … }`,
  `if let Value::Call(...) = val`, `matches!(val, Value::Call(...))`,
  AND every `Value::Var`, `Value::Set`, `Value::If`, `Value::Iter`,
  `Value::Loop`, and `Value::Block` pattern that appears in the
  second-pass and codegen layers.
- A more conservative audit script: `rg -t rust -n
  'match.*Value::|if let Value::|matches!\(.*Value::' src/`,
  filtered to second-pass call paths.
- Each pattern that's expected to receive an unwrapped value gets
  `unspan()`/`unspan_mut()`.  An exhaustive audit ensures the list
  is complete BEFORE 1.B.1 re-enables the wrap.

### Why not Option B (side-table) instead

Option B's pros are exactly avoiding this pattern-match update:
the `Value` shape stays untouched.  Reasons to stay on A despite
this surprise:

1. **Cloning still works** — `derive(Clone)` covers Span; Option B
   would need every `Value` clone site to also clone the side-
   table entry.
2. **Pattern-match audit is one-time and mechanical** — each
   `if let Value::Call(...)` site becomes
   `if let Value::Call(...) = code.unspan()`.  ~12 sites, all
   listed above; cargo errors if a site is missed once unspan()
   exists and is exhausively used.
3. **The `Span` arm is strictly additive in walkers** — every
   walker already has its passthrough from 1.A.  Side-table
   entries would be invisible at the IR level and easy to drop.

So 1.B.0 is the cost; 1.B.1 onwards is straightforward.

## Goal

Every IR node that can fail at runtime, or that can produce a
diagnostic during the second-pass type-check, knows its source
`Position { file, line, col }`.  Today only `Value::Line(u32)`
markers are emitted — line only, no column, file inferred from the
enclosing `Definition.position`.  After phase 1, position is
attached to `Call`, `CallRef`, `Set`, `Iter`, `Return`, the binary /
unary fault-prone operators (div, mod, index, field, narrow cast),
and struct construction.

This phase is the foundation: phases 2, 3 and 6 all consume per-node
positions.  Phase 1 ships only the data path; renderer and pc→span
lookup land in phases 2 and 3.

## Decision 01.A — representation

Two viable shapes; pick one with a measured comparison on phase 0's
bench corpus.

### Option A: enlarge `Value`

Add a `Span` arm and wrap fault-prone constructs:

```rust
pub enum Value {
    …
    Span(Position, Box<Value>),     // wraps a single fault-prone node
    …
}
```

- **Pro:** zero out-of-band data structure; the IR remains a single
  tree.  Codegen pattern-matches `Span` and propagates the position
  to its line-number table without affecting other arms.
- **Con:** every IR walker (codegen, scope analysis, slot
  assignment, formatter, debug print) gains one match arm.  ~15
  files touched.
- **Memory:** one `Box<Value>` indirection per wrapped node;
  `Position` is `String + u32 + u32` ≈ 32 bytes plus the file
  string's heap.  File strings are interned (only ~3 unique values
  in practice — user file + `default/01_code.loft` + `default/…`)
  so the marginal cost is the box.

### Option B: side-table

Keep `Value` shape unchanged; allocate a stable node identity at
parse time (a `u32` tag baked into the `Box<Value>` allocation
address — sound because the parser builds and never moves IR
boxes during the second pass) and store `HashMap<*const Value,
Position>` per `Definition`.

- **Pro:** `Value` size unchanged.  No walker change.  Codegen and
  later phases query `definition.spans.get(node_ptr)`.
- **Con:** pointer-keyed maps are fragile — any clone of the IR
  invalidates entries.  We do clone IR in a few places (e.g.
  `Iter` rewriting in `parser/collections.rs`, lambda
  inlining).  Each cloning site needs a span-aware clone or
  the side-table loses entries.
- **Memory:** `HashMap` overhead per definition; ~200 entries per
  function in practice → ~10 KB per fn × ~500 fns = 5 MB.  Cheap
  but non-trivial.

**Recommendation (to validate in 1a):** Option A.  The walker churn
is one-time and mechanical; the side-table fragility under cloning
is a recurring tax.  A `Value::Span(Position, Box<Value>)` arm is
also strictly additive — old code that doesn't recognise it falls
through a default arm with no behaviour change.

The decision lands in 1a's PR description with the bench delta.

## Steps

### 1a — Choose representation

Implement both shapes against a single fault site (e.g. the binary
`/` operator) on a throwaway branch.  Run `make bench` against
phase 0's `0d-bench.txt`.  Pick the option that:

1. Stays within ±2 % of the baseline on `bench/11_par` and
   `bench/01_classic`.
2. Touches fewer than 20 files for the same fault-site coverage.

Commit the chosen path; throwaway branch is deleted.

### 1b — Parser: emit spans at every fault-prone construction

Sites (from phase 0's `0a-sites.md`):

| IR node | Source | What gets the span |
|---|---|---|
| `Value::Call` | `parser/control.rs::parse_call` | the call-site `(` token |
| `Value::CallRef` | `parser/expressions.rs` | the `.` or `(` introducing the indirect call |
| `Value::Set` | `parser/expressions.rs::parse_assign` | the `=` token |
| `Value::Iter` | `parser/collections.rs` | the `for` keyword |
| `Value::Return` | `parser/control.rs` | the `return` keyword |
| Binary `/`, `%` | `parser/operators.rs` | the operator token |
| Binary indexing `v[i]` | `parser/fields.rs` | the `[` token |
| Field access `s.f` | `parser/fields.rs` | the `.` token |
| Struct literal `S { … }` | `parser/objects.rs` | the type name token |
| Narrow cast (implicit) | `parser/operators.rs::coerce` | the assignment / arg-pass site |

Each site reads `self.lexer.position.clone()` *before* consuming the
relevant token, builds the inner `Value`, and wraps it with
`Value::Span(pos, Box::new(inner))`.

A helper on the parser:

```rust
fn at<F>(&mut self, build: F) -> Value
where F: FnOnce(&mut Self) -> Value
{
    let pos = self.lexer.position.clone();
    let inner = build(self);
    Value::Span(pos, Box::new(inner))
}
```

keeps the call-site noise low: `self.at(|p| p.parse_division())`.

### 1c — Walker fan-out

Every `match val { … }` over `Value` in the second pass and codegen
gets a passthrough arm:

```rust
Value::Span(pos, inner) => {
    self.current_span = Some(pos);    // for error reporting in this walk
    let result = recurse(inner);
    self.current_span = old;
    result
}
```

Files touched (verified by `grep -n 'match.*Value'`):

- `src/parser/expressions.rs` — type-check pass
- `src/parser/operators.rs` — operator dispatch / coercion
- `src/parser/collections.rs` — iterator materialisation
- `src/scopes.rs` — scope analysis
- `src/variables/slots.rs` — slot placement
- `src/state/codegen.rs` — bytecode generation
- `src/formatter.rs` — source formatter
- `src/state/debug.rs` — IR pretty-print

The walker's `current_span` is the cheap mechanism phase 2 uses
("the span for the diagnostic I'm about to emit is whatever Span
arm I most recently entered").

### 1d — Diagnostics consume `current_span`

`diagnostic!` macro is unchanged.  The lexer's
`pos_diagnostic(level, &pos, msg)` is the new preferred entry; sites
in second-pass walkers call:

```rust
self.lexer.pos_diagnostic(Level::Error, self.current_span.as_ref().unwrap_or(&self.lexer.position), msg);
```

A grep audit ensures no `diagnostic!(self.lexer, …)` site in the
second pass uses `self.lexer.position` blindly when a `current_span`
is available — the second-pass position has drifted to wherever the
walker happens to be, not where the user's offending token sits.

### 1e — Update `Value::Line(u32)` policy

Phase 1 does not delete `Value::Line` (it's still useful for crude
breakpoint-style line markers and for `LOFT_LOG=lines`).  But
codegen's line-numbers map (`src/state/codegen.rs:299`) gains a
sibling map:

```rust
pub line_numbers: HashMap<u32 /* pc */, u32 /* line */>,            // existing
pub source_spans: HashMap<u32 /* pc */, Position>,                  // new in 1e
```

`Value::Span(pos, inner)` recursing into codegen records the start
pc → pos mapping before generating `inner`.  Phase 3's pc→span
table is just `Definition.source_spans` plus a binary-search helper.

### 1f — Tests

- `tests/error_messages.rs` re-runs the phase-0 baseline corpus.
  Goldens that previously printed line-only now print line + col;
  `UPDATE_GOLDEN=1` re-captures.  Only **strict improvement** is
  acceptable: a baseline `.expect` losing a column is a phase-1
  regression.
- New `tests/spans_on_ir.rs` parses ~10 short programs and asserts
  the IR tree contains `Value::Span(...)` nodes around the expected
  constructs.
- `make bench` re-run; result table appended to `0d-bench.txt`
  under heading "phase 1".  Must stay within ±2 % of phase 0
  baseline on the 6 bench cases.

## Atomic landing sequence

Each row is one commit.  Steps 1.4 and later assume Option A (the
recommended path); on Option B the wrapping steps become side-table
inserts but the order is the same.

| # | Step | Test |
|---|---|---|
| 1.1 | Add `Value::Span(Position, Box<Value>)` arm; derive `Clone`/`Debug`; no parser wiring yet | Unit test in `src/data.rs`: construct `Span(pos, Box::new(Value::Int(0)))`, clone, debug-format; assert round-trip equality |
| 1.2 | Add `Span` passthrough arm to `state/codegen.rs::generate` (one walker only) | New codegen test: hand-crafted IR with `Span(pos, Int(7))` generates byte-identical bytecode to bare `Int(7)` |
| 1.3 | Add `Span` passthrough to `parser/expressions.rs` second-pass walker | All existing tests in `tests/issues.rs`, `tests/expressions.rs` still green |
| 1.4 | Add `Span` passthrough to `parser/operators.rs` | Same as 1.3 |
| 1.5 | Add `Span` passthrough to `parser/collections.rs` | Same as 1.3 |
| 1.6 | Add `Span` passthrough to `scopes.rs` | Same as 1.3 |
| 1.7 | Add `Span` passthrough to `variables/slots.rs` | Same as 1.3 (slot tests in `tests/frame_vars.rs`) |
| 1.8 | Add `Span` passthrough to `formatter.rs` and `state/debug.rs` | `tests/format.rs` green; debug-print test that `Span(pos, Int(0))` prints same as `Int(0)` |
| 1.9 | Wrap binary `/` in `parser/operators.rs` with `Span` via `self.at(...)` helper | Parser test: parse `let x = a / b`, assert IR contains `Span(_, Call(div, …))` at the `/` token's column |
| 1.10 | Wrap binary `%` | Same shape, asserts column at `%` |
| 1.11 | Wrap vector index `v[i]` | Parser test asserts column at `[` |
| 1.12 | Wrap field access `s.f` | Parser test asserts column at `.` |
| 1.13 | Wrap `Call` and `CallRef` | Parser test asserts column at `(` |
| 1.14 | Wrap `Set` (assignment) | Parser test asserts column at `=` |
| 1.15 | Wrap `Iter` (for-loops) | Parser test asserts column at `for` |
| 1.16 | Wrap `Return` | Parser test asserts column at `return` |
| 1.17 | Wrap struct literal type name | Parser test asserts column at type-name token |
| 1.18 | Wrap implicit narrow cast at coerce site | Parser test asserts column at the assignment / arg-pass site |
| 1.19 | Add `current_span: Option<Position>` to second-pass walkers; route `diagnostic!` through `pos_diagnostic` | Type-error fixture (case 5 from phase 0): `.expect` regen shows correct column; diff vs phase-0 baseline shows column added, nothing else changed |
| 1.20 | Populate `Definition.source_spans: HashMap<u32, Position>` in `state/codegen.rs` (mirror of `line_numbers`) | Unit test: synthetic IR with one `Span`, generate, assert `def.source_spans.contains_key(pc)` and value matches |
| 1.21 | Re-run `make bench`; append numbers to `0d-bench.txt` heading "phase 1"; regen all baseline goldens | Bench delta ≤ ±2 % gates merge; golden diff is "column added, nothing else" — any non-additive diff blocks |

## Acceptance

- 1a decision documented at the top of this file (option, why,
  bench numbers).
- Every fault-prone IR construction wraps in `Value::Span`.
- Every walker has a `Span` arm; `cargo test` green.
- `Definition.source_spans` populated; phase 3 will read it.
- Phase-0 baseline goldens regenerated with column information;
  every diff is strictly more informative.
- `make bench` ≤ ±2 % vs phase-0 baseline.
- `make ci` green.

## Risks

| Risk | Mitigation |
|---|---|
| `Value::Span` wrapper changes recursion depth and trips a stack-overflow test | The wrap is at most 1 level per fault site; total IR depth grows by ≤ 10 %.  If a test trips, increase the parser's recursion budget by the same factor; do not unwrap eagerly. |
| Cloning IR in `Iter` rewrite drops spans | Option A's `Span` arm is part of the enum, so `derive(Clone)` already preserves it.  No special handling needed. |
| Walkers that pattern-match exhaustively explode in arm count | Every match adds **one** arm — the `Span` passthrough.  Compile-time exhaustiveness flags any miss. |
| Bench regresses > 2 % | If A regresses, switch to B for the hot constructs only (e.g. `Call` keeps a side-table; everything else uses `Span`).  Hybrid is acceptable; "all-A" is not a hard requirement. |
