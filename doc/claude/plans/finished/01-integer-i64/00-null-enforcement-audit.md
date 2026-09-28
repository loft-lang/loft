# Phase 0 — `not null` enforcement audit + G/G′ decision

Status: **done** — 11 probes run, 7 holes, 1 major surprise (`??` already
catches arithmetic overflow).  Decision: **ship C54.G (trap) as default**
with a possible **G-hybrid** extension captured as a Phase 1 design
choice.  See "Audit result" at the end.

## Why this is Phase 0

The C54 design (`QUALITY-history.md § 479-557`) offers two semantic fixes for
arithmetic overflow and divide-by-zero:

- **C54.G** — trap (runtime error) on overflow / div-zero.
- **C54.G′** — null on overflow / div-zero, composes with `??` and
  `?? return`.

G′ is strictly the stronger UX win when it works.  It only works if a
null cannot silently reach a `not null` slot.  Trap → null-on-overflow
is a backward-compatible relaxation later; null → trap is breaking.
So the audit MUST land before either implementation.

## Pre-audit findings (preliminary, documented in this file)

An initial exploration of the enforcement surface turned up concrete
gaps.  The audit converts these from suspicions into committed
verdicts via probe fixtures.

| Site | Current state | Hole? |
|---|---|---|
| `??` operator (`src/parser/operators.rs:592-670`) | Recognises field-marked `not null` only.  Does NOT see arithmetic-overflow-null.  Line 597: `if self.expr_not_null` gates warning, but `expr_not_null` is set from field-access analysis, not from arithmetic result types. | **YES — ??-composition doesn't fire on overflow null** |
| Struct field writes (`src/parser/objects.rs::handle_field:1078-1411`) | Compile-time `convert()` at line 1399; no runtime null-check at OpSetInt emission. | **YES — `v.field = nullable_expr` silently writes `i32::MIN`** |
| Struct constructor positional+named (`src/parser/objects.rs:1078-1140`) | Same `convert()` funnel; same miss. | **YES** |
| Function parameter passing (`src/parser/control.rs::parse_call:2680-2950`) | Compile-time `convert()` at line 2911; no runtime null-check at call entry. | **YES — `foo(a * b)` for `a*b` overflow silently passes `i32::MIN`** |
| Return-type narrowing — tuple (`src/parser/control.rs::parse_return:2419-2435`) | Explicit T1.7 error: "cannot assign null to 'integer not null' element" on tuple literals. | NO (held) |
| Return-type narrowing — non-tuple (`parse_return:2438-2439`) | `validate_convert("return", ...)` → `can_convert()`; no runtime narrowing op. | **YES — `return a * b` for a `not null` return silently returns `i32::MIN`** |
| Array / hash index (`src/parser/vectors.rs` + `src/parser/collections.rs`) | No null-check on index before `OpGetVector`. | **YES — `v[a*b]` silently reads an out-of-range slot or fails at runtime without clear diagnostic** |

Provisional conclusion based on these findings: **at least 5 holes are
open** → the design's fallback applies: **ship C54.G (trap) first**;
G′ becomes a later relaxation once the `not null` contract is
tightened.

The audit phase still needs to run the probes to turn "at least 5"
into an exact count and to catch holes not anticipated above (e.g.
method-style call receivers, iterator bounds, parallel-loop indices).

## Probe fixtures to build

Create `doc/claude/plans/01-integer-i64/probes/` with the following
fixtures.  Each probe constructs a program that would route a
nullable integer into a `not null` slot without explicit `??` or
narrowing.  Each fixture's expected status is listed; the audit
verdict is assigned by running the fixture.

| # | File | Shape | Expected |
|---|---|---|---|
| 00 | `probe_00_baseline_overflow_produces_null.loft` | `a = i32::MAX; a + 1` — confirm the sentinel mechanism is live | Produces `i32::MIN` / null |
| 01 | `probe_01_field_write_nullable_into_not_null.loft` | `struct S { v: integer not null }` + `s.v = a + b` where `a + b` overflows | **HOLE** if silent; CLEAN if error/trap |
| 02 | `probe_02_field_write_default_initialised.loft` | `struct S { v: integer not null = 0 }` + later `s.v = nullable_expr` | **HOLE** if silent |
| 03 | `probe_03_field_write_nested_path.loft` | `struct A { b: B }; struct B { v: integer not null }` + `a.b.v = nullable_expr` | **HOLE** if silent |
| 04 | `probe_04_call_arg_nullable_to_not_null.loft` | `fn foo(x: integer not null)`, call with `foo(a + b)` overflow | **HOLE** if silent |
| 05 | `probe_05_chained_call_returning_nullable.loft` | `fn bar() -> integer { ... }` (nullable), call `foo(bar())` where `foo` takes `not null` | **HOLE** if silent |
| 06 | `probe_06_return_narrowing_no_null_check.loft` | `fn foo(x: integer) -> integer not null { x }` called with null | **HOLE** if silent |
| 07 | `probe_07_return_nullcoalesce_discharges.loft` | `fn foo(x: integer) -> integer not null { x ?? 0 }` — happy case | CLEAN (held) |
| 08 | `probe_08_nullcoalesce_on_arithmetic.loft` | `x = a * b ?? default` where `a * b` overflows | **HOLE** — the decisive probe; `??` should catch but doesn't today |
| 09 | `probe_09_index_key_null.loft` | `v[nullable_expr]` and `v[i32::MIN]` | **HOLE** if silent |
| 10 | `probe_10_ref_forwarding_preserves_non_null.loft` | `fn foo(x: &integer) { }; fn outer(y: &integer not null) { foo(y); }` — does the forward preserve? | Likely CLEAN but confirm |

**Critical probe**: `probe_08_nullcoalesce_on_arithmetic.loft`.  If
`??` does NOT catch arithmetic-overflow-null, G′ is unsafe (the
`??`-composition argument from QUALITY-history.md § 537 evaporates).
Conversely, if by some miracle it does catch, G′'s case holds.

## Decision procedure

Run every probe.  For each, record verdict:

- **CLEAN** — the null is either caught (diagnostic or error) or
  propagates observably without silent coercion to `not null`.
- **HOLE** — the null silently reaches a `not null` slot and is
  treated as if it were non-null.

Decision rule:

- **0 holes** → G′ is safe.  Phase 1 ships `01-checked-arith.md`
  with G′ semantics.
- **≥1 holes** → G ships first.  File each hole as a follow-up in
  `00.1-hole-*.md` sub-phases for future tightening; do NOT block
  Phase 1 on closing them.  Phase 1 ships `01-checked-arith.md`
  with G semantics.

Given the preliminary findings above, expect the ≥1 branch.  That is
the SAFE default per the design.

## File + line references for the audit

The audit code-reading tour:

| Area | File | Lines |
|---|---|---|
| `??` op semantics | `src/parser/operators.rs` | 592–670 |
| `expr_not_null` field-access tracking | `src/parser/` (grep for `expr_not_null`) | — |
| Field write compile-time check | `src/parser/objects.rs::handle_field` | 1078–1411 (convert at 1399; set_field_no_check at 1409) |
| Runtime field write opcodes | `src/fill.rs` | `OpSetInt`, `OpSetShort`, `OpSetByte` handlers |
| Function call arg check | `src/parser/control.rs::parse_call` | 2680–2950 (convert at 2911) |
| Return narrowing — tuple case | `src/parser/control.rs::parse_return` | 2419–2435 |
| Return narrowing — non-tuple | `src/parser/control.rs::parse_return` | 2438–2439 |
| Null sentinel values | `src/data.rs::I32` | 32 |
| Storage read/write | `src/store.rs::{get,set}_{int,short,byte}` | 1115, 1124, 1153, 1167, 1184, 1194 |
| Arithmetic null-producing sites | `src/ops.rs` | 531 `op_add_int`; 313 `op_add_long`; the `checked_int!` macro at 33–52 |

## Deliverables

- `doc/claude/plans/01-integer-i64/probes/probe_00.loft` … `probe_10.loft`
- An **"Audit result"** appendix to this file:
  - Per-probe verdict (HOLE / CLEAN).
  - Hole count.
  - Decision: G or G′.
  - Rationale paragraph.
- `Status: done` flip on this file.
- `01-checked-arith.md` renamed or retitled to reflect the chosen
  option (e.g. "Phase 1 — C54.G trap on overflow").

## Budget

**120–180 minutes**.  Bail out after 180 min: ship G based on the
preliminary findings above; open holes as follow-ups.  Do not chase
an inventory past diminishing returns.

## Non-goals for Phase 0

- Implementing either G or G′.  Phase 0 decides; Phase 1 implements.
- Fixing any holes that surface.  Each hole becomes a follow-up
  sub-phase AFTER G ships.
- Touching `.loftc` format, storage layout, opcodes, stdlib.

## Success criteria

1. All 11 probes committed.
2. Each probe classified with a committed verdict.
3. Audit-result appendix committed.
4. Phase 1 opens with the chosen option named in its file + title.

---

## Audit result (2026-04-18)

All 11 probes built as `.loft` fixtures under
`doc/claude/plans/01-integer-i64/probes/`.  Each ran via
`./target/release/loft --tests <probe>.loft`; output parsed for the
`VERDICT: ...` line the probe prints.

### Per-probe verdicts

| # | Probe | Verdict | Observed |
|---|---|---|---|
| 00 | baseline overflow | **CONFIRMED** | `i32::MAX + 1` produces null — sentinel mechanism is live in release interpreter |
| 01 | field write nullable → not null | **HOLE** | `s.v = a + 1` (overflow) silently writes null into the `integer not null` field |
| 02 | field write default-initialised | **HOLE** | same silent write on a field declared `integer not null = 0` |
| 03 | field write nested path | **HOLE** | `a.b.v = overflow_expr` silently stores null at depth |
| 04 | call arg nullable → not null | **HOLE** | `fn foo(x: integer not null); foo(a+1)` passes null silently |
| 05 | chained call returning nullable | **HOLE** | `foo(bar())` where `bar` returns nullable and `foo` takes `not null` — null propagates |
| 06 | return narrowing | **HOLE** | `fn passthrough(x) -> integer not null { x }` returns null when x is null |
| 07 | happy-case `??` | **CLEAN** | `?? 0` correctly discharges null |
| 08 | `??` on arithmetic | **CLEAN** (unexpected) | `(i32::MAX * 2) ?? 42` returns 42 — `??` DOES catch arithmetic overflow null |
| 09 | array index null | **HOLE** | `v[null_idx]` silently returns null (no diagnostic, no bounds trap) |
| 10 | `&` forwarding not-null | **CLEAN** | `fn inner(x: &integer); fn outer(y: &integer not null) { inner(y); }` preserves and mutates correctly |

**Hole count: 7** (probes 01, 02, 03, 04, 05, 06, 09).
**Clean count: 4** (probes 00, 07, 08, 10).

### The decisive surprise — probe 08

The pre-audit analysis assumed the `??` operator only discharged
field-marked `not null` (based on the `self.expr_not_null` check at
`src/parser/operators.rs:597`).  Probe 08 disproves that: `(a * b) ??
42` returns 42 when `a * b` overflows.

Conclusion: `??` uses a runtime null-check — it evaluates the LHS and
if the result equals the type's null sentinel (`i32::MIN` for `integer`)
it returns the RHS.  This is orthogonal to the compile-time
`expr_not_null` tracking, which drives the deprecation warning for
non-null inputs, not the runtime behaviour.

**Implication for G vs G′.**  The design's G′ argument
(QUALITY-history.md § 537) — "the `??` composition is the decisive win" — is
already available *today* on overflow.  G would BREAK this idiom by
trapping before `??` runs.

### Decision: ship C54.G, but flag the `??` idiom regression

Per the design's fallback rule (§ 555), 7 holes require G.  However,
G introduces a user-visible regression: today `x = (a * b) ?? default`
works; under G it traps.  To preserve that idiom without giving up
G's safety, Phase 1 should consider a **G-hybrid** variant:

- **Bare arithmetic overflow traps** (the 7-hole safety argument).
- **Overflow inside a `?? default` context produces null instead of
  trapping**, so `??` can catch.

Compile-time detection: when codegen sees an arithmetic op whose
result is *immediately* consumed by a `??`, emit the
null-on-overflow variant.  Everywhere else, emit the trap variant.

This is a third option not spelled out in QUALITY.md.  The Phase 1
plan must decide between:

1. **Pure G** — trap everywhere, including inside `??`.  Simplest.
   Breaks the `?? default` idiom.  Users must write explicit guards
   (`if a > 0 && b > 0 && a < i32::MAX / b { a * b } else { default }`).
2. **G-hybrid** — trap by default, null-propagate inside `??`.
   More implementation.  Preserves the idiom.

Phase 1 opens on the G-hybrid option (path 2) as the primary
candidate, with pure G as the fallback if the compile-time
detection of the `OP ?? default` shape proves invasive.

### Follow-up holes (not blocking Phase 1)

Each of the 7 holes is a pre-existing null-enforcement gap,
orthogonal to C54.  Filed as a list for a future tightening effort
(probably Phase 7+ of this initiative or a sibling initiative):

- **H1 — field writes**.  Emit a runtime null-check when assigning
  into a `not null` field.  Covers probes 01, 02, 03.
- **H2 — function parameters**.  Emit a runtime null-check at call
  entry for `not null` parameters.  Covers probes 04, 05.
- **H3 — return narrowing**.  Emit a runtime null-check on return
  from a function declared `-> T not null`.  Covers probe 06.
- **H4 — array indexing**.  Emit a runtime null-check or bounds
  check on index before `OpGetVector`.  Covers probe 09.

These do not block Phase 1 — they are tighter-net-null-contract work
that can land incrementally.  They DO block a future G′ migration.

### Rationale

- **Why not ship G′ now?**  Probe 08 made G′'s case compelling
  (the `??` idiom works today), but probes 01-06 + 09 show that
  null can silently reach `not null` slots through 4 independent
  paths.  G′ under those conditions means overflow nulls flow into
  the same traps.  Safer to trap at the source until holes close.
- **Why G-hybrid instead of pure G?**  Pure G breaks an idiom that
  works today.  Migration pain without corresponding safety win
  (the holes are H1-H4, not `??`).  G-hybrid preserves the idiom
  at the cost of slightly more codegen complexity.  Phase 1's
  first task: confirm the compile-time detection is tractable.
- **What makes G-hybrid safe?**  Arithmetic WITHOUT `??` always
  traps.  Arithmetic WITH `??` produces a null that's immediately
  caught by the `??`.  No silent propagation either way.  The holes
  H1-H4 are the null-reaching-contract issues, but they are
  triggered by explicit null values (literals, explicit function
  returns), not by arithmetic overflow anymore.

### Phase 0 close

- `Status: open` → `Status: done` (this file header updated).
- 11 probe fixtures committed.
- Phase 1 plan file (`01-checked-arith.md`) to be updated with
  G-hybrid as primary, pure G as fallback, compile-time detection
  design sketched.
- H1-H4 hole list filed in the initiative README as future
  sub-phases (`07-enforcement-H1-field-writes.md` etc., opened only
  when prioritised).
