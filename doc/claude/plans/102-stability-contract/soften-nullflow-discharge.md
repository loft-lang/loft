<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN102 — Softening the `??` discharge: a domain lattice for fault ops

> **Status: DESIGN (2026-07-16).** DN3-Float types every float fault op (`/`, `%`,
> `sqrt`, `ln`, `log`, `asin`, `acos`, `pow`) as `float?`, forcing a `?? default`
> discharge. That discharge is the *concrete cost* of the uniform-null model. This doc
> measures where the discharge is **genuinely needed** vs **ceremony** (forced on a
> provably-non-null value), then designs a narrow, sound way to remove the ceremony —
> a **sign/lower-bound lattice** consulted when typing a fault op — and evaluates the
> specific cases. Trigger: the hex_terrain 0.1.1 migration, where **all four**
> arithmetic `??` I added were ceremony (`sqrt` of sums-of-squares / `max(_,positive)`,
> `pow` of a non-negative base — none can ever be null). Builds on
> [dn3-float-null-flow-design](../../DESIGN_DECISIONS.md) (DN3-Float, shipped default-on).

## The measurement (probed, not assumed)

loft's *only* current non-null proof is **shallow literal constant-folding**:

| Expression | Types as | Why |
|---|---|---|
| `sqrt(4.0)`, `10.0 / 2.0`, `pow(2.0, 3.0)` | **non-null** | the whole expression folds to a literal |
| `x / 2.0`, `x / SQRT3`, `x % 2.0`, `i / 2` | **non-null** | **divisor** proven nonzero (literal / file-const) — *division is already softened* |
| `x / d` (variable divisor) | `float?` | `d` could be 0 — **genuinely needed** |
| `sqrt(x * x)`, `sqrt(x*x + 1.0)`, `sqrt(abs(x))` | `float?` | **no domain proof** — forced despite a provably-≥0 argument |
| `sqrt(x)` (unknown sign) | `float?` | could be negative — **genuinely needed** |

Two conclusions fall straight out:

1. **Nonzero-divisor softening already exists** (literal / file-const divisor, and — verified —
   *qualified* literal/expression consts like `lib::CONST` already inline+fold). The only
   forced division is `x / <variable>`, which is correct. The one residual gap was a
   **call-valued const** (`PI = OpMathPiFloat()`): `x / PI` forced `?` because `const_eval`
   couldn't fold the nullary op — **fixed** by teaching `fold_op` `OpMathPiFloat`/`OpMathEFloat`
   (their exact `fill.rs` values), so a `PI`/`E` divisor or fault-op arg folds like a literal.
2. **The entire remaining ceremony is the domain ops** — `sqrt`/`ln`/`asin`/`pow` get
   *no* argument-domain analysis, so any variable-bearing argument forces `?`. In real
   numeric code (variables everywhere) that is effectively unconditional, and the
   discharged value is almost never actually null (distances, magnitudes, clamped
   inputs). This is the whole gap worth closing.

### Where `??` is *actually* needed

After this design, `??` remains **required** exactly where null is genuinely reachable:
`v[i]` / `hash[k]` on an unbounded index/key, `text as integer` (parse), a nullable
field/param, `x / <maybe-zero>`, `sqrt(<maybe-negative>)`. Everything else is ceremony
the type system simply can't yet see through.

## The unsound lint (why an inline fix wasn't possible)

The "Redundant null coalescing" lint (`src/parser/operators.rs:1444`, off the
`expr_not_null` flag) tracks whether the `??` operand *derives from a not-null name* —
but ignores that a fault op can **null a non-null input**. Verified: `sqrt(-4.0)` on a
**non-null** negative *is* null. So the lint fired on `sqrt(max(tt_steep,0.01)) ?? 0.0`
calling it redundant, **while the (correct) type system required the discharge** — a
contradiction with *no inline form that satisfies both*. In hex_terrain I had to hoist
to a `steep_root` local to dodge it. The lint is unsound independent of anything below,
and this design retires it on fault-op results.

## Design — one mechanism: a domain lattice

A bottom-up abstract interpretation over *pure* float sub-expressions, lattice
`{ Pos, NonNeg, Unknown }`, **default `Unknown`** (conservative). The nullability pass
consults `domain(arg)` when typing a fault op: if the argument is in the op's safe
domain the result types **non-null**; otherwise it stays `float?`, unchanged. One
recursive function with per-node transfer functions — not a bag of pattern matches (the
"fold the fact into the structure" move; every fault op then *reads* the fact).

Transfer functions — the specific softening cases, each with its soundness argument:

| Node | Fact | Sound because |
|---|---|---|
| literal `c` | `Pos` if c>0, `NonNeg` if c==0, else `Unknown` | exact value |
| `a * a` (operands structurally equal) | `NonNeg` | a square is ≥ 0; a **non-null** float is never the NaN sentinel, so no null leaks in |
| `a + b` | `NonNeg` if both `NonNeg`; `Pos` if either `Pos` and other `NonNeg` | monotone; overflow → +inf, still ≥ 0 and non-null |
| `abs(e)` | `NonNeg` | by definition |
| `max(e, k)`, k literal ≥ 0 | `NonNeg` (`Pos` if k>0) | lower bound is k |
| `min(a, b)` | `NonNeg` if both `NonNeg` | lower bound is min of bounds |
| `sqrt(e)` (nested) | `NonNeg` | sqrt result ≥ 0 |
| variable / unknown call / anything else | `Unknown` | can't see it → the op stays `float?` (correct) |

Each fault op then reads it: `sqrt(NonNeg)` → non-null; `ln`/`log`(`Pos`) → non-null
(strict, needs `Pos` not `NonNeg`); `pow(NonNeg, _)` → non-null; `sqrt(Unknown)` → `float?`.
Every hex_terrain site — `sqrt(ddx*ddx + ddy*ddy)`, `sqrt(max(tt_steep,0.01))`,
`pow(rad, 2.4)` — resolves to non-null.

### Compatibility rule (load-bearing)

Softening **only removes the error** (makes `??` optional); it must **never add a
warning**. A `??` on a now-provably-non-null value stays *silently accepted*, so:

- existing libraries stay warning-clean under `LOFT_DENY_WARNINGS=1` (no forced churn),
- new code may simply omit the `??`,
- and the unsound redundant-coalescing lint (case E) **stops firing on fault-op results
  entirely** — it can't soundly judge them, and the type now covers the safe cases.

This is additive under [absolute compat](../../COMPATIBILITY.md): a `?` becoming non-null
is a narrowing of the *result* type that no existing program can observe as a break (a
non-null value satisfies every `float?` consumer, and every existing `??` still runs).

## Evaluation of the cases

| Case | Value | Cost | Risk | Verdict |
|---|---|---|---|---|
| **E** — retire the unsound `expr_not_null` lint on fault-op results | removes a live bug (un-satisfiable sites) | trivial | none — removes unsoundness | **DONE** — both the `??` (`operators.rs:1444`) and `== null` (`:2160`) lints now gate on the operand's `Optional` type, not the stale flag; regression guards in `runtime_warnings.rs` |
| **B** — `sqrt`/domain non-negativity lattice (above) | high — *all* geometric `sqrt`/`pow`; the whole real gap | moderate — a bounded recursive analysis | soundness-critical, but the square/sum/max/abs rules are provably safe with `Unknown` default | **DONE, default-on** (opt-out `LOFT_NO_MATH_DOMAIN`) — `Sign` lattice + `domain_sign` in `parser/mod.rs` wired into `math_arg_in_domain`; the flip's redundant-lint churn closed by the `call_declares_nullable` grandfather; tests in `tests/math_domain.rs` |
| **C** — nonzero divisor | — | — | — | **DONE** — literal/file-const/qualified-const already folded; the residual call-valued const (`PI`/`E`) now folds via `const_eval` (`fold_op` + `const_f64`), so `x / PI` is non-null |
| **A** — fold through pure arithmetic before typing | low | low | none | **Falls out of B** (constants get exact facts) |
| **D** — `v[i]` bound-carry in `for i in 0..len(v)` | highest raw frequency (the `v[i] ??` sites) | — | — | **ALREADY SHIPPED** as `@PLN102 D1` (the null-flow flip, #559) — `fields.rs::index_provably_fit` types `v[i]` non-null for a for-loop iter var (and integer-arith indices over trusted leaves, `m[k*4+row]`), plus the `if i<len(v)` guard (pattern 5). It is a **trust model**, not a proof: a for-loop iter var is trusted for ANY vector (like a constant index — `v[100]` also types non-null), so a mismatched loop (`for i in 0..len(v) { w[i] }`, or a mid-loop resize) types non-null yet reads @C80-null at runtime. Tightening to a proof (range = `len(THIS vector)` + not-resized) would fix that but **break the ubiquitous `for i in 0..n { v[i] }` idiom** (`n` not a `len`) — a deliberate index-trust decision for the owner, not a softening this plan needs to add. **The audit half is now built**: a gated `LOFT_LINT_STRICT_INDEX` (default OFF, advisory — the type stays non-null) flags exactly the mismatched-vector case (`for i in 0..len(v) { w[i] }`, `w != v`), the highest-signal slice of the unchecked trust, leaving the trust intact (see below) |

## Soundness bar (non-negotiable for B, and D if ever attempted)

A wrong non-null proof is exactly the corruption DN3-Float exists to prevent, so per
[measure-a-flip-by-running-the-suite](../../STABILITY_METHOD.md) this ships **gated +
measured by running the full corpus on both backends**, never by counting compile-rejects
(a silent wrong-answer — a genuinely-null value landing in a non-null slot — is invisible
to a reject scan):

- **Positive controls that must STAY `float?`** (fail the build if any types non-null):
  `sqrt(x)`, `sqrt(x * y)` (distinct operands), `sqrt(x - 1.0)`, `sqrt(max(x, -5.0))`,
  `ln(x)` / `ln(max(x, 0.0))` (needs `Pos`, `NonNeg` is not enough), `x / d`.
- **Whole-corpus differential**: run the full suite + `native_scripts` under the analysis
  and confirm no value that is null at runtime now reaches a non-null slot.
- Graduate the controls to `tests/scripts/` as a permanent guard.

## Recommendation + scope

Shipped **E then B**, `sqrt`/`ln`/`log`/`pow` first (the sign lattice), then **`asin`/`acos`
DONE** via a small two-sided interval-bounds pass (`pm_bounds` in `parser/mod.rs`): the
`[-1, 1]` domain is proved for `sin`/`cos` outputs, `clamp(e, -1, 1)`, and the manual
`min(max(e, -1), 1)` clamp (unary-negation nodes handled so a literal `-1.0` reaches the
bound as a constant); an unbounded or one-sided arg stays `float?`. **Case C** also closed (the
`PI`/`E` call-valued-const fold). **Case D** turns out to be already shipped (`@PLN102 D1`, the
for-loop-iter-var index trust — see the table). Net effect: the `??` ceremony collapses to exactly
the genuinely-reachable faults — variable divisors, unbounded `v[i]`, parses, nullable fields —
and hex_terrain's four arithmetic `??`
disappear with nothing that could truly be null losing its guard.

## Implementation plan — small steps

Case E is shipped. The **case B** lattice (B0–B4 + the strict-op rule) is **implemented**
behind `LOFT_MATH_DOMAIN` (default off); only the default-on flip remains. Each rule is a
sound transfer function with a *positive control* that must **stay `float?`**, all pinned by
`tests/math_domain.rs`.

- **B0 — scaffold. DONE.** `enum Sign { Pos, NonNeg, Unknown }` + `fn domain_sign(&Value) ->
  Sign` (default `Unknown`) over pure float/single expressions; node kinds matched by EXACT
  stdlib def name (`OpMulFloat`, `t_5float_max`, …), never a suffix.
- **B1 — square rule. DONE.** `a * a` (structurally-equal operands) → `NonNeg`; `sqrt(NonNeg)`
  → non-null. Controls stay `?`: `sqrt(x)`, `sqrt(x*y)`.
- **B2 — sums + literals. DONE.** literal by sign; `OpAddFloat` of non-negatives → `NonNeg`
  (`Pos` if either `Pos`). Covers `sqrt(dx*dx + dy*dy)`. Control: `sqrt(x - 1.0)` stays `?`
  (subtraction is unmatched → `Unknown`).
- **B3 — clamps. DONE.** `max(e, k)` → stronger bound; `abs(e)`/`sqrt(e)` → `NonNeg`;
  `min(a,b)` → weaker bound. Covers `sqrt(max(tt_steep, 0.01))`. Control: `sqrt(max(x,y))`
  stays `?`.
- **B4 — nesting + `pow`. DONE.** nested `sqrt(e)` → `NonNeg`; `pow(NonNeg, _)` → non-null.
- **B5-strict — DONE.** `ln`/`log2`/`log10` require `Pos` (not `NonNeg`); controls
  `ln(max(x,0.0))`, `ln(abs(x))` stay `?`.
- **B5-flip — DONE.** Flipped default-on (opt-out via `LOFT_NO_MATH_DOMAIN`). The whole-corpus
  measure under the flag surfaced exactly ONE churn class — an inline `sqrt(max(field, k)) ?? d`
  newly warning "redundant" — closed by the **grandfather** `call_declares_nullable`
  (`operators.rs`): the two case-E lints never fire on a call to a fn DECLARED `-> τ?`, even
  when the domain lattice narrowed this site to non-null, since a `??` / `== null` guarding a
  declared-nullable op is a real defense (a bare `s.nn ?? d` still warns — distinct node). No
  runtime-null reached a non-null slot; hex_terrain 0.1.1 + the math fixtures stay warning-clean
  under `LOFT_DENY_WARNINGS`.

**Case D — the audit lint (DONE).** The softening (`v[i]` non-null under the for-loop trust) was
already shipped as `@PLN102 D1`; what this plan adds is the *audit* half — a gated
`LOFT_LINT_STRICT_INDEX` warning (default OFF) that surfaces the one unsound slice of the trust
without changing a type. It fires when a for-loop iter var is bounded by `len(<one vector>)` but a
DIFFERENT vector is indexed (`for i in 0..len(v) { w[i] }`, or `s.b[i]` under `len(s.a)`) — the
mismatched-vector silent-null hazard, highest signal / lowest noise. It is deliberately *not* a
default flip: tightening the trust to a proof would break `for i in 0..n { v[i] }`, so the trust
stays and the lint is opt-in.

- **Capture** (`objects.rs::parse_in_range_body`): a pure exclusive `for i in 0..len(X)` range
  records `X`'s `VecKey` on the loop (`Iterator.len_bound`); `0..n`, `0..=len(X)`, and slices do
  not qualify.
- **Emit** (`fields.rs`, at the `index_provably_fit` site): when a trusted loop-var index has a
  `len_bound` and the indexed vector's `VecKey` differs, warn — advisory, the element type is
  unchanged. Both `VecKey::Var` and `VecKey::Field` (field vectors) discriminate.
- **Gate + tests**: `keys::strict_index_lint_enabled()` (`LOFT_LINT_STRICT_INDEX`, default OFF);
  corpus `strict-index-corpus.loft` (3 FIRE + 4 OK), locked both backends by
  `tests/strict_index_lint.rs`. Emits no IR, so byte-identical for every program (full suite green).

Still out of this plan: whether to *tighten* the index trust to a proof (a separate compat
decision), and `pow(rad, 2.4)` where `rad` is a *variable* (inline-only analysis; variable-def
tracking is a separate follow-up).

## See also

- [DESIGN_DECISIONS.md](../../DESIGN_DECISIONS.md) — DN3-Float (the null model this softens) · @C80 no-runtime-errors (the uniform-null contract) · [COMPATIBILITY.md](../../COMPATIBILITY.md) (why softening is additive)
- `src/parser/operators.rs:1444` — the `expr_not_null` redundant-coalescing lint (case E)
- Corpus / trigger: the hex_terrain 0.1.1 migration (loft-lang/loft#579) — every arithmetic `??` it needed is a bucket-3 ceremony site this design removes.
