<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN102 — Float domain-partial ops type `τ?` (the DN3 float extension)

> **Status: SHIPPED — FLIPPED DEFAULT-ON (2026-07-11, the @PLN102 null-flow cutover).**
> This doc was written spec-first; the implementation landed the same day and is now the
> default (`nullflow_enabled()` in `src/keys.rs` — true unless `LOFT_NO_NULLFLOW`; the old
> opt-in `LOFT_NULLFLOW` is a redundant no-op). Live on `main`: the domain functions in
> `default/01_code.loft` return `float?`/`single?` (`sqrt`/`asin`/`acos`/`ln`/`log`/`log2`/
> `log10`/`pow`), the float `/`/`%` div gate types `float?`, and (N-Prop)/(N-Warn)/(N-Cast)
> are enforced (verified: `return sqrt(y)` into a non-null `float` warns; `sqrt(y)+1.0`
> stays `float?`; both compile and run). The rest of this doc is the design record behind
> the shipped feature. A pre-freeze-only change (it alters a frozen SIGNATURE surface, so
> it landed while `contract` is still 0). **No runtime error added** — entirely a
> compile-time *type* change. Owner directive: *"a null is fine, errors never"* + *"use the
> types to give warnings about potential nulls"* (DESIGN_DECISIONS
> [@C80](../../DESIGN_DECISIONS.md), the spreadsheet fault model).

## The problem — the float type lies about null, the integer type does not

`x / 0` on integers types **`integer?`** (DN3): the type carries the potential null, so
storing it into a non-null `integer` field is flagged *today* as a hard error —
*"a nullable `integer?` cannot be stored into the assignment target of the non-null type
`integer` — discharge it first with `?? <default>` or `match`."* The programmer is nudged to
handle the null; nothing halts at runtime (the value is just null, @C80). (This doc relaxes
that hard error to a **warning** — see (N-Warn) below.)

The **float** side does none of this. Verified on both backends (2026-07-11):

| expression | runtime | declared type today | honest? |
|---|---|---|---|
| `10 / y` (int) | null on `y==0` | `integer?` | ✅ type carries the null |
| `1.0 / b` (float) | null on `b==0.0` | `float` (non-null) | ❌ **type lies** |
| `sqrt(-1.0)` | null | `float` (non-null) | ❌ **type lies** |
| `ln(-1.0)` / `asin(2.0)` / `acos(2.0)` | null | `float` (non-null) | ❌ **type lies** |

Because the float results are typed non-null, `f.g = 1.0 / b` and `f.g = ln(-1.0)` store a
null straight into a non-null `float` field with **no diagnostic at all** — the exact
silent-null the integer side already prevents. This is the `float`/`integer` asymmetry
called out in the pre-freeze audits (formal-audit E2, lib-audit numeric). Since return
types freeze at contract 1, the honest signature must be chosen *now*.

**This surfaces a latent hazard; it does not add one.** A `float` can already be `NaN`
*anywhere* — every IEEE operation can produce it, and `NaN` *is* the float null (@C90). The
`float?` typing + the warning do not create a new failure mode; they make **visible** a null
that was always possible but silent, for the many programmers unaware of it. Nothing new can
go wrong at runtime (the NaN was always reachable), which is why a **warning** is the right
instrument — inform, don't break — and why a well-written library, which should already
handle the NaN, simply gets an explicit nudge to do so.

## The invariant (one rule, both integers and floats)

> **A numeric operation types its result `τ?` exactly when it can yield the reserved
> null from inputs an ordinary program reaches** — its domain boundary is crossable by
> normal computed values. An operation whose only null-producing inputs are
> *extraordinary* (an integer overflow's ~3×10⁹ operands; a float infinity, itself only
> reachable through such an overflow) stays **non-null** — a decided edge, not a lie.

This is not a new principle: it is the **DN3-vs-C85 boundary already drawn for integers**,
read across to floats. DN3 (`÷0`, index, parse → `τ?`) and @C85 (overflow → non-null,
"the game keeps running") are the two halves; every float op lands on one side by the
*same* test. So the design absorbs floats into an existing invariant rather than inventing
one — and the classification below is where that absorption is checked, not assumed
(design-protocol step 4).

## Classification — which float/single ops become `τ?`

| Op(s) | Null from a *reachable* input? | Verdict |
|---|---|---|
| `/`, `%` | yes — divisor `0.0` | **`τ?`** (mirror of integer `/`; the existing `divisor_provably_nonzero` proof keeps `x / 2.0` non-null) |
| `sqrt` | yes — argument `< 0` | **`τ?`** |
| `ln`, `log`, `log2`, `log10` | yes — argument `≤ 0` | **`τ?`** |
| `asin`, `acos` | yes — argument outside `[-1, 1]` | **`τ?`** |
| `pow` | only base `< 0` **and** fractional exp | **`τ?`** (resolved 2026-07-11 — genuine domain error; `pow(x, 2)` is free, only a non-null store needs a discharge) |
| `sin`, `cos`, `tan`, `atan`, `atan2`, `exp`, `abs`, `ceil`, `floor`, `round` | NaN only from `±inf` input (extraordinary, overflow-reachable) | **non-null** (@C85-style decided edge) — no guard forced on ubiquitous ops |

`atan2` is total on all finite inputs (including `atan2(0,0)=0`); `sin`/`cos`/`tan` of a
*finite* argument are always finite (verified `cos(1000.0)` → finite). Their only NaN comes
from an infinite argument, and infinity is reachable only via a @C85 overflow — so forcing
`?` on every `sin` call would tax the common path for a fault the program never hits,
exactly the @C85 argument. They stay non-null.

## Mechanism — two type-level hooks, runtime untouched

Both hooks change only *types*. The runtime already returns null (NaN) and the nullable
representation is free (`float?`/`single?` share the base's NaN sentinel — types.md; the
`Optional` peel at `operators.rs:2076` already handles `float? == null`).

1. **`/` and `%`** — extend the DN3 division gate (`src/parser/operators.rs:2300`):
   ```rust
   let div_nullable = (operator == "/" || operator == "%")
       && matches!(ctp.base(), Type::Integer(_))          // ← add Float | Single
       && !self.divisor_provably_nonzero(&second_code);
   ```
   The nullable runtime peers already exist (`OpDivFloatNullable`, `OpRemFloatNullable`,
   `OpDivSingleNullable`, `OpRemSingleNullable` — phase 4f.5) and the `??`-swap already
   dispatches to them (`rewrite_outer_arith_to_nullable`). Extending the gate to
   `Float`/`Single` is the one missing piece, and it gets the `divisor_provably_nonzero`
   elision (so `1.0 / 2.0` and a guarded `if b != 0.0` stay non-null) **for free**.

2. **Domain-partial functions** — change the declared return type in
   `default/01_code.loft` from `-> float` / `-> single` to `-> float?` / `-> single?` for
   `sqrt`, `ln`, `log`, `log2`, `log10`, `asin`, `acos`, and `pow` (both the `float` and
   `single` overloads). `exp`/`ln`/`log2`/`log10` desugar through `log`/`pow`, so typing
   `log` and `pow` nullable flows to them automatically; verify the desugar carries the `?`
   (and that `exp`, which is total, does **not** inherit a spurious `?` — it desugars
   `pow(E, x)`, so if `pow` is `float?`, `exp` must re-assert non-null or be defined off a
   non-nullable power path). The underlying `OpMathFunc*` / `OpPow*` ops are unchanged.

**Enforcement is a WARNING, not an error** (`(N-Store)`, revised — see the null-flow model
below): an un-discharged `float?` into a non-null `float` field/return/typed-local *warns*
and directs to `?? d` / `match` / a `float?` target, but the program still compiles and
runs (the slot holds null at runtime — spreadsheet model). A warning, because (1) a hard
error would **break every existing program** that stores a `sqrt`/float-`/` result in a
non-null `float` slot — a compatibility violation — and (2) [Goal F](../../GOALS.md) reserves
*warnings* as the only channel that may bill the programmer. No new op, **no runtime fault**.

## Re-assertion sites (design-protocol step 2) — brittleness check

The invariant lives in **~9 centralized declaration sites** — the division gate (hook 1) and
the eight function return-type declarations (hook 2) — not scattered across call sites. Every
consumer reads the nullability *from the type*; there is no per-use re-assertion to forget.
An omitted signature makes that one op silently non-null (the type lies again), caught by the
conversion suite when a nullable-expecting case sees a non-null type — a bounded, auditable
risk on a small N, not a spray. (Contrast the reverted `DomainError` attempt, which needed
the guard re-stated at 12 runtime sites *and* carved out null-propagation.)

## Friction evaluation (2026-07-11, empirical) — and the resolved forks

`float?` does **not** create friction at *use* — only at a genuine **non-null
requirement**. Measured against the live `integer?` oracle (integer `/` already returns
`integer?`) and confirmed on a constructed `float?`, on both backends:

| context | result |
|---|---|
| assign / infer `x = sqrt(y)` | **free** — `x : float?`, no annotation needed |
| arithmetic `sqrt(y) + 1.0`, `* 2.0` | **free at the op** — result stays `float?` (N-Prop, below); the nudge rides through to a later non-null store |
| comparison `sqrt(y) > 1.0`, `== v` | **free** — uniform null comparison (D-op-null-1), yields `bool` |
| `if sqrt(y) > 1.0 { … }` | **free** |
| interpolation `"{sqrt(y)}"` | **free** — renders `null` |
| pass to any fn incl. `sin` / `sqrt` / `abs` | **free** — argument passing is permissive today |
| `?? d` discharge | **free** — yields `float` |
| **store into a non-null `float` field** | **WARN** — nudged to `?? d` / `match`; still compiles + runs |
| **return into a non-null `float` return type** | **WARN** |
| **assign to an explicit non-null local `m: float`** | **WARN** |

The nudge is exactly the three non-null **storage** sites — *"a place that doesn't allow
nulls"* — which is precisely where you WANT to be warned about what the null means. It is a
**warning, not an error** (N-Warn), so nothing breaks. Everything else flows, and (N-Prop)
carries the nullability *through* arithmetic to those store sites rather than laundering it
away. A thorough `float?` is therefore cheap, and it **resolves both forks**
(owner, 2026-07-11):

- **`pow` → `float?`.** Its domain error is genuine, and `y = pow(x, 2.0)` is free — only a
  *non-null store* of a pow result needs a discharge. Honesty wins with no broad friction.
- **Domain-proving → the constant/provable subset only.** No full range-tracking, but a
  **provably-in-domain argument blocks the `float?` typing** — a literal or known constant
  like `sqrt(4.0)`, `sqrt(PI)`, `ln(2.0)` stays **non-null** (the argument is a known `≥ 0` /
  `> 0`), exactly as a constant non-zero divisor keeps `x / 2.0` non-null
  (`divisor_provably_nonzero`). A *variable* argument (`sqrt(x)`) is `float?`. Symmetrically,
  a constant *out-of-domain* argument (`sqrt(-1.0)`) can warn at compile time — *"always
  null"* — the parallel of the existing constant-`/0` warning. The earlier worry that
  `sqrt(dx*dx + dy*dy)` forces a `??` was wrong regardless: `d = sqrt(…)` just infers
  `float?`, free — only a non-null *store* is nudged. Full per-function range-tracking
  (proving `dx*dx + dy*dy ≥ 0`) is deferred; the constant case is the cheap, high-value slice.

### The null-flow model this rides on — nullability PROPAGATES (no laundering), and warns

An earlier draft of this doc described a *partial* guarantee — arithmetic laundering the
`float?` back to non-null. That laundering is **wrong**, and the fix spans the integer side
too (owner, 2026-07-11). Verified: the runtime **already propagates null** through every
arithmetic op — `n+5`, `n-5`, `n*5`, `n%5`, `5-n`, `abs(n)` on a null `n` all stay null (the
integer ops check the `i64::MIN` sentinel; float NaN propagates by IEEE). Only the *type*
dropped it. The type must catch up to the semantics that already exist:

- **(N-Prop) — nullability propagates through arithmetic.** An arithmetic op with **any**
  nullable operand yields a nullable result: `integer? + integer → integer?`,
  `float - float? → float?` (either operand position). This already is how `text? + text`
  behaves (the @PLN25 nullable-concat propagate); it becomes uniform across
  `integer`/`float`/`single`. So `sqrt(y) + 1.0` stays `float?` all the way to the store,
  where (N-Warn) nudges — the guarantee is **no longer partial**.
- **(N-Warn) — the non-null store is a warning, not an error — EXCEPT narrow width types.**
  The relaxation applies iff the target's null pattern is still available in its *non-null*
  form: `integer` (reserves `i64::MIN` even non-null — a null stored there reads back as
  null, verified), `float`/`single` (NaN), `text` (out-of-band null). A **narrow width
  integer** (`u8`/`i8`/`u16`/`i16`/`i32`/`u32`) spends its whole width on real values (a
  non-null `u8` holds `255`), so it has **no bit-pattern for null** — a null there is
  unrepresentable and would silently corrupt to a real value. Those keep the **hard error**;
  the programmer must `?? d` or widen the target to the nullable form (`u8?`). The split is
  principled: *warn iff the null is representable-and-observable in the non-null slot* — the
  same in-band-sentinel property @C85 already relies on. Keeping the narrow error costs zero
  compatibility (narrow stores already error today, DN1/DN4/DN5).
- **@C85 is untouched and does not conflict.** Propagation is driven by an operand *already
  being nullable*, not by the possibility of overflow. `a * b` with **both operands non-null**
  stays non-null (@C85 — overflow silently → sentinel; no `?` forced on the ubiquitous
  non-null case). The moment a nullable enters, it rides through to the store. The two rules
  compose: non-null arithmetic stays non-null; a null, once present, stays *visible*.

**Scope note.** (N-Prop) and (N-Warn) refine **DN3 across the shipped integer model**
(@PLN25), not float alone: integer arithmetic gains the same propagation, and the
non-null-store diagnostic relaxes from a hard error to a warning. Both are
backward-compatible — relaxing error→warning never breaks a program, and propagation only
makes *already-nullable* values honest (a non-null program is unaffected). This is a larger
surface than the float return-type flips; it is the null-flow half of the same change.

## Casts are assertions, not operations — `as float` is never `float?`

A crucial boundary (owner, 2026-07-11): the DN3-Float nullability is for **fit-failing
operations** (`/`, `sqrt`, `ln`, …) whose nullability is *inherent*. An explicit **cast**
`as T` is the opposite — an **assertion** that the value fits — and it yields **non-null `T`**,
never `T?`. Every `as` cast follows DN4's narrowing model uniformly:

- **`as T`** → non-null `T`. Provable-valid → the value; not provable (a variable narrowing
  that might not fit, *or* a text parse) → **compile error** directing to `as T?` / `?? d` /
  `match`. It never silently becomes `T?`.
- **`as T?`** → the checked cast → `T?` (value or null; never a wrap/corruption — verified
  `511 as u8?` → null, not `255`).
- **`as T ?? d`** → assert-or-default → non-null `T` (the useful pattern: `text as float ?? 0.0`).

So `sqrt(x)` is `float?` (operation) but `x as float` is `float` (assertion) — *"as float
implies a situation where everything works."* This **revises DN3's N-Parse**, which today
auto-wraps `text as float` / `text as integer` to `τ?`: under the assertion model a bare
`text as float` on a non-provable text is a compile error (use `as float?` or `?? d`),
bringing parse casts in line with `as u8`. Distinct decisions, one rule — **the `?` on a cast
is always the programmer's explicit choice, never inferred.**

## Conversion set (enumerate + convert in the same change — the audit rule)

Every call site that uses one of these results in a non-null context needs `?? d` or a
`float?` target. Counted across `default/` + `tests/scripts/` + `tests/docs/` + `lib/` +
`examples/`: `sqrt` 16, `asin` 10, `acos` 10, and `ln`/`log`/`log2`/`log10`/`pow` in the
dozens (some counts inflated by comments / `.log()` Rust bodies — the real figure comes
from the flip). **Float `/` and `%` are the large unknown** — float division is common, and
grep can't separate it from integer division. The true conversion set is measured by
*running the full suite under the flip* (dev-gate on,
both backends) and reading the `(N-Store)` errors — not by grep. Land the golden-behavior
corpus first so each conversion's diff is visible; convert `default/*.loft` and the in-tree
consumers in the same change.

## Implementation plan — small steps, each with its verification

Grounded in the actual code points (2026-07-11). Three of the four laws land at existing
chokepoints; only Phase 5 touches the *un-chokepointed* surface (param passing, call results),
which is where "a lot of code" is. **Land Phase 1 first** — the warn/error split turns the
blast radius of every later phase from "build breaks" into "warnings," so 2–4 land
incrementally. Every step verifies on **both backends** (`--interpret` AND `--native`); gate
the lot behind a flag (`LOFT_NULLFLOW`, or reuse `pln25_dn3`) default-off → validate →
default-on.

**Chokepoint map:** `(N-Store)` = `n_store_violation` (`src/parser/mod.rs:2038`, 5 callers) ·
binary-op result = `handle_operator` near the `div_nullable` wrap (`src/parser/operators.rs:2300`/`:2339`) ·
cast/parse = the `as` handler (`src/parser/operators.rs:~2000`) · domain fns = return-type
decls in `default/01_code.loft` · narrow discriminator = `IntegerSpec::byte_width()` (`src/data.rs:178`, `<8` = narrow).

### Phase 0 — instrument + baseline

- **0.1** Add the gate (`LOFT_NULLFLOW`) defaulting to current behavior.
  *Verify:* flag off → full suite green, byte-identical `loft introspect` on a sample corpus.
- **0.2** Land the **cross-type golden corpus** — one `.loft` per type × each law (N-Domain,
  N-Prop, N-Cast, N-Store), capturing CURRENT behavior.
  *Verify:* runs clean on both backends; this is the before-baseline every phase diffs against.

### Phase 1 — `(N-Store)` warn-unless-narrow  ·  `mod.rs:2038` (chokepoint)

- **1.1** Add `is_narrow_store_target(tp)` = `matches!(tp, Type::Integer(s) if s.byte_width(false) < 8)`.
  *Verify:* unit — true for `u8`/`i8`/`u16`/`i16`/`i32`/`u32`; false for `integer`/`float`/`single`/`boolean`/`character`/`text`.
- **1.2** DN3 branch (`mod.rs:2084`): emit `Level::Warning` when not narrow (keep `Level::Error`
  when narrow), and **return `false` on the warn path** so the store still compiles (a warning
  must not block codegen — check the 5 callers treat `false` as "proceed").
  *Verify:* `s.f = 10/y` (`f: integer`) → warns + compiles + runs, `f` reads null; `n.x = 10/y`
  (`n.x: u8`) → hard error. Both backends.
- **1.3** DN1 bare-null branch (`mod.rs:2103`): same split.
  *Verify:* `f.g = null` (`g: float`) → warns + compiles (`g` null); `n.x = null` (`x: u8`) → error.
- **1.4** Confirm the warned store leaves codegen intact.
  *Verify:* after a warned store the slot holds the sentinel and reads null — no crash / garbage — on both backends (`LOFT_STORES=warn` / `LOFT_NATIVE_LEAK_CHECK` clean).

### Phase 2 — `(N-Prop)` propagate through binary arithmetic  ·  `operators.rs:~2339` (chokepoint)

- **2.1** Capture `operand_nullable` (either operand `Optional`) BEFORE `call_op` peels it.
  *Verify:* env-gated `eprintln` — for `n + 1` (`n: integer?`), `operand_nullable == true`.
- **2.2** Beside the `div_nullable` wrap: if `operand_nullable` && op ∈ `{+ - * / % & | ^ << >>}`
  && the result base is a scalar → `*ctp = Type::optional(ctp.base())`.
  *Verify:* `x = n + 1` (`n: integer?`) → `x: integer?`; `float - float?` → `float?`; storing
  either into a non-null slot warns (Phase 1). Both backends.
- **2.3** @C85 unaffected — non-null × non-null stays non-null.
  *Verify:* `a=3; b=4; s.f = a*b` (`f: integer`) → NO warning, clean.
- **2.4** Runtime value pin — a null through arithmetic stays null.
  *Verify:* `(n+1) ?? -7` with `n` null → `-7` on both backends (graduate to `tests/scripts/`).

### Phase 3 — `(N-Domain)` floats  ·  `operators.rs:2300` + `01_code.loft`

- **3.1** Extend the `div_nullable` gate: `Type::Integer(_)` → `| Type::Float | Type::Single`.
  *Verify:* `1.0/b` (var) → `float?` (warns at a non-null store); `1.0/2.0` (const) → non-null
  (`divisor_provably_nonzero`). Both backends.
- **3.2** Flip `sqrt`/`asin`/`acos` return types → `float?`/`single?` (`01_code.loft`).
  *Verify:* `sqrt(x)` → `float?`; `sqrt(x) ?? -1.0` → the value or `-1.0`. Both backends.
- **3.3** Flip `ln`/`log`/`log2`/`log10` → `float?`/`single?` (they desugar via `log`).
  *Verify:* `ln(x)` → `float?`; **`exp(x)` (total, via `pow`) stays non-null** — no spurious `?`.
- **3.4** Flip `pow` → `float?`/`single?`.
  *Verify:* `y = pow(x,2)` free (infer `float?`); `s.f = pow(x,2)` (`f: float`) warns.
- **3.5** Constant-in-domain elision (the one bit of *new* per-fn logic, mirrors
  `divisor_provably_nonzero`): a provably-in-domain constant arg → non-null.
  *Verify:* `sqrt(4.0)` / `sqrt(PI)` → non-null (store into non-null `float`, no warning);
  `sqrt(x)` (var) → `float?`. Both backends. (Lands after 3.1–3.4.)
- **3.6** *(optional)* constant-OUT-of-domain compile warning, like constant `/0`.
  *Verify:* `sqrt(-1.0)` → warning *"always null"*; value still null.

### Phase 4 — `(N-Cast)` parse folds into the assertion cast  ·  `operators.rs:~2000`

- **4.1** Delete the text→numeric auto-`Optional` wrap (`operators.rs:~2000`).
  *Verify:* `s as float` no longer types `float?` on its own.
- **4.2** Route bare `text as τ` through the DN4 assertion path (non-null; compile error for a
  non-literal text → "use `as τ?` / `?? d`").
  *Verify:* `s as float` (var) → compile error directing to `as float?`/`?? d`; `"3.14" as float`
  (provable literal) → non-null `float`. Both backends.
- **4.3** `as τ?` still checks → `τ?` (null on bad parse, never a wrap).
  *Verify:* `"abc" as float?` → null; `s as float ?? 0.0` → non-null `float` (assert-or-default).
- **4.4** Convert the corpus: every existing `s as float`/`as integer` → add `?` or `?? d`.
  *Verify:* full suite green on both backends after conversion; golden-parse corpus diff shows
  only the intended sites move.

### Phase 5 — close the chokepoint gaps (the large-impact surface)

- **5.1** Confirm `v[i] = x` (index store) routes through `n_store_violation`; add the call if not.
  *Verify:* `v[i] = 10/y` (`v: vector<integer>`) → warns (not silently laundered).
- **5.2** Param passing: call `n_store_violation` at arg binding (`call_nr` / arg conversion) so
  `f(nullable)` into a non-null param warns (narrow → error). *Bigger — touches call resolution.*
  *Verify:* `f(10/y)` with `f(a: integer)` → warns; `f(a: u8)` → error. Both backends.
- **5.3** Function-call `(N-Prop)`: propagate arg nullability to pure scalar fn results
  (`abs`/`min`/`max`). *Bigger — call-result typing.*
  *Verify:* `abs(n)` (`n: integer?`) → `integer?` (currently launders to non-null).

### Cross-cutting — the conformance matrix is the regression gate

After **every** phase: re-run the Phase-0.2 cross-type matrix + the full suite on both backends,
read the new warnings/errors, and convert the in-tree corpus in the same change (the audit
rule). A divergence in a type you didn't hand-check is a caught deviation — that is what "verified
throughout the stack" means operationally.

## Falsification probes (design-protocol step 3) — run before/with the build

- **"`float?` is representationally free"** → `float? == null` and a `vector<float?>` read
  correctly. *Confirmed* (the `2076` peel; @C90 pins NaN = the float null).
- **"the existing divisor proof applies to float"** → after the gate change, `1.0 / 2.0`
  (const) and `if b != 0.0 { a / b }` stay non-null. *Implementation-time.*
- **"sin/cos/… never null from finite"** → `sin(1e300)`, `cos(1000.0)` finite, not null.
  *Confirmed* for `cos`.
- **"runtime is already null, no error"** → `sqrt(-1.0)` / `1.0/0.0` → null, exit 0, no
  stderr, both backends. *Confirmed.*
- **Over-unification guard (step 4):** the cleanest claim is *"every float op fits the one
  DN3/C85 invariant."* The place it could be false is the `sin/cos-at-infinity` row — if
  those were forced `τ?` the rule would be *wider than the domain* (tax a ubiquitous op for
  an overflow-only fault). Keeping them non-null is the check firing, not a convenience.

## What this is NOT

- Not a runtime error. `sqrt(-1)` stays null + continue (@C80). No `DomainError`, no fault
  kind, no soft-halt entry. The type does all the work, at compile time.
- Not a new diagnostic mechanism. It reuses `(N-Store)` — the same check integer `/`
  already trips.
- Not a per-case DESIGN_DECISIONS entry restating @C80. The governing principle already
  exists; this doc records the *typing* refinement, not a new value decision.

## See also

- [types.md](../../formal/types.md) — the formal rule (DN3 float extension); the spec-first artifact this doc backs.
- [DESIGN_DECISIONS.md @C80](../../DESIGN_DECISIONS.md) (spreadsheet model), @C85 (overflow non-null), @C90 (in-band null sentinel).
- [formal-audit.md](formal-audit.md) E2 / [lib-audit.md](lib-audit.md) numeric — the audit rows this closes.
- `src/parser/operators.rs:2300` (the div gate) · `default/01_code.loft` (the function return types) — the two touch points.
