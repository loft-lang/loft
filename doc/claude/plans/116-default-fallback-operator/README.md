<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 116 — `x?` default-fallback operator

Tracks [`loft-lang/plans#116`](https://github.com/loft-lang/plans/issues/116) (`@PLN116`).

## Status

**DONE 2026-07-22 (both backends).** Reference for the operator itself now lives in
[`LOFT.md` § `x?`](../../LOFT.md); the enum-field non-null-soundness decision in
[`DESIGN_DECISIONS.md` § @PLN116](../../DESIGN_DECISIONS.md). The rest of this file is the
closure record.

**Landed (both backends).** The postfix `x?` operator ships as sugar for
`x ?? construct_default(T)`: it parses in the `parse_part` postfix loop (tightest
precedence, `parse_part` → `handle_default_fallback`), and reuses the whole `??` emission
path (`build_null_coalesce_default`) by handing it a pre-built default value
(`pending_default_rhs`) or, for the token-driven empty collection / record, a synthetic
default SOURCE parsed in-context (`pending_default_src` / `subparse_default`). No new
opcode — like `??`, it desugars to `if`/`block`/`set`/`call`, so both backends come free.
Proven **bytecode-identical to the hand-written `x ?? <default>`** for scalar, text,
enum, record and collection; the Stage-A matrix is graduated to
`tests/scripts/116-default-fallback-operator.loft` (green both backends + leak gate).

The single predicate is `Data::has_default` (`src/data.rs`) + `Parser::build_default`
(`src/parser/operators.rs`); `to_default`'s `character` gap (`Value::Null` →
`Value::Int(0)` = `'\0'`) was fixed on the way.

**Enum-field non-null soundness (arc B/D — RESOLVED, a contract-1 fix).** The open
question "may a record silently default an enum field?" is answered **no**. An enum's 0 is
its null/undefined value (variants are 1-based), so zero-filling a *non-null* enum field
put null into a non-null slot — a soundness bug the null model otherwise forbids, and one
that would freeze forever at =1. The one `has_default` rule now rejects a bare
(non-`Optional`) enum field with no `= expr`: **both** `x?` on such a record **and** `S{}`
omitting the field are compile errors (`object_init` enforces the `S{}` half). A *bare*
enum still discharges to its first variant (`x?` on `E?`); a genuinely `Optional` enum
field (`Color?`) still defaults `null`. This makes `S{}` and `x?` genuinely share one
predicate (the invariant the plan protects). In-repo defaults added where the old zero-fill
was relied on (all overwritten before read): `File.format = NotExists`, `Lexer.scanned =
Unknown`, `Definition.structure = Function`. Blast radius was tiny (1 stdlib field + 2
libs); full suite green.

**Inherited `??` gaps (not `x?`-specific — `x?` desugars faithfully).** `??` (hence
`x?`) does not discharge a nullable *struct field* (`p.field?` on a `Point?` field — the
`__nullable<S>` representation), and a *parenthesised float division* `(a/b)?` keeps
`inf` (the div→nullable rewrite doesn't reach a paren-wrapped op). Scalar/enum/text
nullable fields, vector-OOB, and integer division all discharge correctly.

## Goal

Ship `x?` as sugar for `x ?? construct_default(T)`, resolving null to the type's default
value, with a compile error wherever that default is not well-defined.

## Effort + design

- **Effort:** M
- **Design:** ✓ (converged)
- **Last touched:** 2026-07-22

## Why it exists

loft's own null-flow manufactures the boilerplate this relieves: DN3 makes float ops yield
`float?` ([@PLN25](../25-nullable-sequences/README.md) gate-2 null model), so `(a / b) ?? 0.0`
recurs; map/field/index reads are nullable by the @C80 spreadsheet model. `x ?? 0` is
explicit but repetitive, and it forces the author to *spell* a default the type already
knows. `x?` reads the type's default instead. The `x ?? v` / `x?` pair is the mnemonic:
**`??` = the default I give; `?` = the default the type gives.**

## The core rule — one predicate, two consumers

`construct_default(T)` (and its guard `has_default(T)`) is defined **once** and feeds **both**
`S{}` construction and `x?`. There must never be a second notion of "T's default" — that is
the invariant this plan protects ([Goal E](../../GOALS.md): one home per fact).

`has_default(T)` / `construct_default(T)`:

| `T` | has default? | default value |
|---|---|---|
| `U?` (nullable) | yes | `null` — *a nullable field never blocks its record* |
| scalar (`integer`/`float`/`boolean`/`character`) | yes | `0` / `0.0` / `false` / `\0` |
| `text` | yes | `""` |
| collection (`vector`/`hash`/…) | yes | empty |
| enum | yes | the **marked** default variant, else the **first-defined** variant |
| record | **iff every field has a default** (field declares `= expr` **or** its type `has_default`), recursively | `S{}` with each field defaulted |
| reference / non-null `DbRef` | **no** | — (compile error) |

**`x? : T`** requires `has_default(static-type-of-x)`; otherwise a **compile error naming
the first field that fails**, e.g. *"record `Order` has no default: field `customer:
Customer` has none — add `customer: Customer = <expr>`, make it `Customer?` (defaults null),
or give `Customer` a default."* This is a *compile* error (a static well-definedness check),
fully consistent with "no **runtime** errors ever" (@C80).

Cycles self-resolve: value-recursion is illegal (infinite size), and ref-recursion bottoms
out at a reference, which has no default → error. No special cycle handling.

## Composition matrix — Stage A

Written as `/tmp` probes on `--interpret` first, then graduated to
`tests/scripts/116-default-fallback-operator.loft`. The feature is done when **every cell is
green on both backends**, not when a demo runs.

| Axis | Cells |
|---|---|
| operand type | scalar (each of int/float/bool/char) · text · vector · hash · enum(marked) · enum(first-defined) · record(fully-defaulted) · record(under-defaulted → **compile error**) · nullable `U?` · reference (→ **compile error**) |
| operand runtime state | `null` → default · non-null → identity (**+ redundant-`?` warning**) |
| nesting / recursion | record with a nested fully-defaulted record field · record with a nullable field (defaults `null`) · `S{}` agreeing with `construct_default` on the same type |
| operator interaction | `x? ?? y` · `x ?? y?` · chained `a.b?` · `x?` in a non-null context (`T? → T`, assignable) · greedy lex `a??b?` = `a ?? (b?)` |
| backend | `--interpret` · `--native` (values identical) |

The load-bearing cell is **`S{}` vs `x?` agreement**: both must produce the identical value
for the same type — the proof that the predicate has one home.

## Sub-arcs

| Item | Concern | Status |
|---|---|---|
| **A** — `has_default` / `construct_default` | the single recursive predicate + value builder; `Data::has_default` + `Parser::build_default`, reusing `to_default` (char gap fixed) / `object_init` | **Done** |
| **B** — enum default | bare enum → first-defined variant via `emit_variant_value`; a bare enum FIELD in a record needs an explicit choice (non-null soundness). Marked-default-variant marker still open | **Done** |
| **C** — parse + desugar | postfix `?` in `parse_part` (tightest); lexer greedy `??` over `?` confirmed (2-char match); desugars via the `??` path; typing = `has_default` | **Done** |
| **D** — the compile error + `S{}` consistency | `x?` AND `S{}` share the one `has_default` rule — both reject a bare-enum-field record; error names the culprit + remedies | **Done** |
| **E** — diagnostics | redundant-`?` warning (same authority as redundant-`??`: `expr_not_null` + type) | **Done** |
| **F** — both backends + tests | bytecode-identical to `x ?? <default>` on both; matrix graduated to `tests/scripts/116-default-fallback-operator.loft` | **Done** |

## Phase ordering

1. **A** — the predicate is the foundation; build it as the *single* source `S{}` already
   (or will) consult, not a parallel path. Verify `S{}` and `construct_default` agree on a
   fixture of every type before any syntax exists.
2. **B** — enum default folds into A's `construct_default`.
3. **C** — parse + desugar on top of the working predicate.
4. **D** — the error messages + the `S{}`-tightening decision (see Open questions).
5. **E** — the redundant warning.
6. **F** — runs throughout; the matrix graduates as cells go green on both backends.

## Open design questions

1. **`S{}` tightening (verify, don't assume).** Does loft's `S{}` today zero-fill a field
   whose type has no default and no `= expr`? If so, the "record has a default iff **all**
   fields do" rule turns that into a compile error — a deliberate, register-recorded
   tightening (and it closes the `DESIGN_DECISIONS` construction-vs-parse tail, ~1188–1190).
   Confirm current behavior on a fixture *before* choosing to tighten.
2. **Enum implicit-first-default: on by default, or lint-nudged?** First-defined-variant
   fallback maximizes ergonomics but makes declaration order semantic. Recommend: allow it,
   plus an **advisory lint** when `?`/`S{}` relies on an enum's *implicit* (unmarked) default
   — nudging the protobuf convention (make the first variant a meaningful zero) without
   Rust's hard `#[default]` friction.
3. **Notation — RESOLVED to `?`** (record in `DESIGN_DECISIONS.md`): postfix `?`, not
   `?? _` or a named form. Rationale: loft has no exceptions / no early-return-on-null, so
   `x?` cannot carry Rust's hidden-control-flow hazard (local, total, value-in-value-out);
   and `.` already null-propagates (@C80), so Swift/Kotlin `?.` is redundant here — both
   neighbouring `?`-slots are semantically vacant, making `x?` the safer claimant.

## Cross-arc dependencies

- **[@PLN25](../25-nullable-sequences/README.md) null model / DN3 float null-flow** — the reason
  the operator exists (it manufactures the `?? <default>` call sites); the `x?` result type
  follows the same `T? → T` discharge rule as `??` (see `LOFT.md` § `??`).
- **The existing `S{}` construction-default + field `= expr` machinery** (`LOFT.md` field
  defaults) — reused, and the single-home invariant this plan must not violate.
- **`DESIGN_DECISIONS.md` construction-vs-parse `S{}` tail** — this plan resolves it (arc D).
- **[@PLN113](../113-contract-keyed-semantics/README.md)** — not required: `x?` is purely
  additive (new syntax), so it is post-1-safe with no contract-keying.

## See also

- `doc/claude/LOFT.md` — the `??` operator (§ null-coalescing) and field `= expr` defaults;
  where the `x?` operator reference lands on ship.
- `doc/claude/DESIGN_DECISIONS.md` — the notation decision (Q3) + the resolved
  construction-vs-parse tail.
- [@PLN25 gate-2 null model](../25-nullable-sequences/README.md) — DN1/DN3, the `??` discharge
  rule this extends.
- [`loft-lang/plans#116`](https://github.com/loft-lang/plans/issues/116) — the tracking issue.
