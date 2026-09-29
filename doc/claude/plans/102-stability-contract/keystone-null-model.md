<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN102 keystone — the null representation model (the decision)

> **Status: DECIDED — B (owner, 2026-07-10). Step 1 (spec honesty) landed.** This is the
> deepest pre-freeze item — it recurs across [lib-audit.md](lib-audit.md) and
> [formal-audit.md](formal-audit.md) and is the @PLN102/@PLN25 boundary. Worked as a design
> decision per the audit disposition: the invariant, the alternatives *presented in full* with
> conversion cost, then a recommendation. The owner chose **B (confront + guard)**; the concrete
> work is under "If B" below.
>
> **Progress:** ✅ **Step 1 — spec honesty (docs).** ✅ **Step 2 — uniform null comparison
> (D-op-null-1 CLOSED).** The `Op{Eq,Ne,Lt,Le}{Float,Single}` `#rust` bodies now treat a NaN
> operand as null definitely — `null == null` true, `!=` its exact complement, null orders low —
> matching integer/char, verified on both backends; guard
> `tests/scripts/pln102-null-comparison-uniform.loft`; the conversion set (docs/tests on the old
> `x != x` NaN idiom → `== null`) migrated in the same change. ✅ **Step 3a — shift collisions
> guarded:** `1 << 63`/`1 << 100`/`1 << -1` now report + null (like `÷0`, new `ShiftOutOfRange`),
> not silently masked/nulled; both backends, zero conversion set, guard
> `pln102-shift-collision-guard.loft`. ✅ **Step 3b — cast collisions guarded (D-op-null-2
> CLOSED):** `float as integer` out-of-range (was: saturate to `i64::MAX`), `int as character`
> invalid code point (was: silent NUL), and `text as integer` parsing to exactly `i64::MIN` now
> report + null (new `CastOutOfRange`); both backends, guard `pln102-cast-collision-guard.loft`,
> one-assertion conversion set (`inf as integer` in `02-floats.loft`). ✅ **Step 4 — honest nullable
> returns:** `find`/`rfind` → `integer?`, `min_of`/`max_of` → `T?`; pure static type honesty (same
> runtime rep), **zero caller breaks** across the full suite, strict compile-rejection guard
> `pln102_stdlib_reachable_null_returns_are_typed_nullable` (proven non-vacuous). ✅ **Step 5 —
> residual frozen:** [DESIGN_DECISIONS.md § C90](../../DESIGN_DECISIONS.md) ratifies option B and
> freezes the per-type reserved-value table; golden pin `pln102-null-residual-golden.loft` (both
> backends). **KEYSTONE COMPLETE (steps 1–5).**

## The invariant we must be able to state (and today cannot)

> **A value of a non-null type `τ` is never observably `null`; a `null` of type `τ?` is never
> observably a real `τ` value; and `null`'s equality and ordering are uniform across every scalar
> type.**

The current model violates all three, silently.

## The current model (verified, both backends)

`null` for a scalar is an **in-band sentinel value** stored in the value slot itself
([types.md](../../formal/types.md) null-representation table):

| type | null is… | a real value it collides with |
|---|---|---|
| `integer` (i64) | `i64::MIN` | `1<<63`, `abs(i64::MIN)`, `"-9223372036854775808" as integer`, overflow |
| `float`/`single` | `NaN` | any computed `NaN` (`sqrt(-1)`); and `NaN != NaN` breaks `==` |
| `u8`/`u16`/narrow | the top value (`255`, …) | a real `255` in a `u8?` |
| `boolean` | `255` | — |
| `character` | codepoint `0` | a real `'\0'` |
| **reference** | **`nullref`** (a reserved DbRef) | **none — out-of-band, clean** |
| **struct in a `vector`** | **`__nullable<S>` tagged enum** (discriminant + payload) | **none — out-of-band, clean** |

**Key fact: loft already uses out-of-band nullness for references and struct-in-vector.** Only the
*scalars* use an in-band sentinel. So this decision is narrowly about the scalar representation.

### The three defects (all permanent if frozen)
1. **Silent-wrong collisions.** A legitimate computed value equal to the sentinel silently becomes
   `null` — the class the whole plan exists to kill.
2. **Cross-type inconsistency.** The sentinel differs per type, so `null == null` is **true** for
   int/char but **false** for float (NaN); `null` orders as **−∞** for int but is **incomparable**
   for float; the two formal docs even cite different sentinel constants.
3. **The spec denies it.** `operational.md` E-Null claims the encoding is "private/unobservable"; it
   is in-band and observable. Freezing E-Null locks an abstraction loft does not provide.

## The alternatives

### Alt A — retire the in-band scalar sentinel; represent `τ?` out-of-band (a tag), like structs already are

Give a nullable scalar an explicit null tag (a discriminant byte, or a wider tagged slot), exactly
as `__nullable<S>` already does for a struct element. A non-null scalar keeps the full value range.

- **Fixes all three defects by construction.** No collisions; uniform `null` identity/ordering; the
  spec becomes honestly out-of-band. Full correctness — a `u8?` can hold `255`, `integer` can hold
  `i64::MIN`, `character?` distinguishes `'\0'`.
- **Cost: HIGH — a runtime-representation rearchitecture for nullable scalars.** Every nullable
  scalar's storage widens (a tag byte), the eval-stack representation changes, every op that
  produces/consumes a nullable scalar changes, native codegen changes — and it **changes the frozen
  store byte-layout** (a nullable field grows a tag; L-Null's `layout(τ)=layout(τ?)` no longer
  holds). The zero-overhead in-band model was a deliberate loft strength (C79/C80). Confined to
  *explicitly-nullable* scalars (non-null stays zero-overhead), but still large.
- **Conversion cost:** internal (huge); user-program (LOW — programs don't see the representation,
  and every observable change is "a silent-wrong becomes correct").

### Alt B — keep the in-band sentinel (zero-overhead); confront and guard it

1. **Spec honesty:** state `null` is in-band, observable, reserved out of the value range (fix
   E-Null, reconcile the constants).
2. **Reserve the sentinel out of the non-null range** (mostly done: `IntegerSpec` already reserves
   `i64::MIN+1` as the min). A non-null `τ` never legitimately produces the sentinel bit-pattern.
3. **Guard the collision sites → FAULT** (this is the error-audit's one-way-door "add now" list):
   overflow / shift-out-of-range / `text as integer` / `float as integer` / `int as character` that
   would produce the sentinel value **faults** instead of silently nulling. A real value never
   silently becomes null. Conversion cost ~0 (nobody relies on `1<<63` being null).
4. **Uniform identity/ordering:** special-case the comparison ops so `null == null` is **true**
   uniformly (including the float null-NaN pattern), `null != null` is false, and `null` ordering is
   defined the same way for every scalar (either a fixed extreme or contagion → null — pick one).
5. **Accept the residual, consciously** (a DESIGN_DECISIONS entry + a golden test): a *nullable*
   scalar cannot hold the one sentinel value — `u8?` can't hold `255`, `integer?` can't hold
   `i64::MIN`, `character?` conflates `'\0'` with null, `float?` uses a specific NaN as null. These
   are **rare, documented, and — with the guards — loud, not silent.**
- **Fixes defects 1 and 2** (silent-wrong → loud fault; inconsistency → uniform). Defect 3 fixed by
  the spec rewrite. Does **not** reclaim the one-reserved-value-per-nullable-type (that is the
  irreducible price of in-band).
- **Cost: LOW–MEDIUM.** Add faults, fix the eq/ordering ops, rewrite the spec sections. No
  representation change, no store-layout change.
- **Conversion cost:** concentrated in the newly-faulting cases (~0) + the float `==` reflexivity
  fix (programs comparing float nulls) + the added faults' program set.

### The float sub-case (sharpest either way)
Float is the worst in-band case: `NaN != NaN` breaks reflexivity, and *any* computed NaN reads as
null. But note a computed NaN (`sqrt(-1)`) → null is actually **consistent** with the C80 spreadsheet
model ("uncomputable → null"), so it is arguably *correct*, not a collision. The real float defect is
just `null == null == false`, fixed cheaply under B by making `==` reflexive for the null-NaN
pattern. So B handles float without a rearchitecture; the only float loss is "can't have a non-null
NaN," which is rarely wanted.

## Probe (design-protocol — try to falsify the recommendation)

- *Claim: "B's guards make null never silently appear."* Load-bearing. The collision sites are
  overflow (arith/shift), the casts (float→int, text→int, int→char), and reading a stored sentinel.
  If producing the sentinel **value** faults, a non-null slot never holds it silently, and a nullable
  slot holding it *is* null (intended). The only residual is a nullable slot that *wants* the
  sentinel value as data — the accepted residual. **Holds.**
- *Claim: "the residual is rare."* `u8?` holding `255`, `integer?` holding `i64::MIN`, `char?`
  holding `'\0'`, a non-null NaN — each is an edge a real program almost never needs, and with the
  guard it is **loud** (a fault or a documented reserved value), not a silent wrong. **Holds** — and
  it is the difference between B (a documented conscious limitation) and A (no limitation) that the
  owner is really choosing.
- *Claim: "A's benefit is small vs its cost."* A reclaims one extreme value per nullable scalar type
  and removes the residual — at the price of rearchitecting the nullable-scalar runtime + breaking
  the frozen store layout. The benefit is real but narrow; the cost is the largest in the plan.

## Recommendation — **B (confront + guard), with the float `==` reflexivity fix**

The freeze-blockers are the **silent-wrong collisions** and the **cross-type inconsistency** — both
fixed by B at low cost, with a conversion set near zero. B keeps loft's deliberate zero-overhead
null model and does **not** disturb the frozen store layout. The residual it accepts (a nullable
scalar loses its one sentinel value) is *rare, loud, and documented* — which the compat doctrine
permits (a conscious limitation is fine; a silent-wrong is not). A's full correctness is real but
buys back only a handful of extreme values for a rearchitecture cost and a store-layout break — the
worst kind of thing to take on right before a freeze. **Pick B unless reclaiming `u8?=255` /
`integer?=i64::MIN` / `char?='\0'` as storable values is judged worth a core-representation
rewrite.**

## If B — the concrete work (each with its conversion set, land while contract 0 allows)

1. ✅ **Spec:** rewrote E-Null (in-band + observable + reserved), reconciled `types.md`/
   `operational.md`, stated the reserved value per width, fixed the `integer` range. *(landed
   2026-07-10; the two remaining code gaps are `D-op-null-1`/`2` in operational.md.)*
2. ✅ **Uniform `null` identity/ordering** — `null == null` true, `!=` its complement, null at the
   LOW extreme, uniform across scalars. Landed at the `Op{Eq,Ne,Lt,Le}{Float,Single}` `#rust` bodies
   (single source → both backends via `fill.rs` regen); guard
   `tests/scripts/pln102-null-comparison-uniform.loft`; conversion set (5 sites on the old `x != x`
   idiom) migrated. *(2026-07-10; D-op-null-1 closed.)*
3. ✅ **Guard the collision sites → report + null (like `÷0`)** so a real value never silently
   becomes null. **3a — shifts** (OOR amount / left-result == sentinel → `ShiftOutOfRange`, both
   backends, zero conversion set). **3b — casts** (option A, report-like-`÷0`): `OpCastIntFromFloat`
   OOR (was: saturate to `i64::MAX`), `OpConvCharacterFromInt`/`OpCastCharacterFromInt` invalid code
   point (was: silent NUL), `OpCastIntFromText` parsing to exactly `i64::MIN` → all `CastOutOfRange`;
   an unparseable text stays DN3-nullable → null (unchanged). Both backends, guard
   `pln102-cast-collision-guard.loft`, one-assertion conversion set (`inf as integer` in
   `02-floats.loft`). *(2026-07-10; D-op-null-2 closed — `default/01_code.loft` `#rust` bodies.)*
4. ✅ **Honest nullable return types** where a fault is reachable — `find`/`rfind` → `integer?`
   (null when absent) and `min_of`/`max_of` → `T?` (null when the vector is empty). The runtime
   representation is identical (in-band sentinel), so the change is purely static type honesty; it
   is observable only at a declared-non-null boundary (assigning the result to a non-null decl now
   errors). **Blast radius measured by running the full suite: ZERO caller breaks** — every caller
   already used inference / `== null` / `?? default` / a boolean context. Guard =
   `tests/issues.rs::pln102_stdlib_reachable_null_returns_are_typed_nullable` (a strict compile-
   rejection test — runtime tests can't distinguish `τ` from `τ?`; proven non-vacuous by injecting
   a revert). STDLIB.md updated. *(2026-07-11; `default/03_text.loft` + `default/01_code.loft`.)*
5. ✅ **DESIGN_DECISIONS entry + golden tests** for the accepted residual — the one reserved value
   per nullable scalar is now a conscious, frozen decision:
   [DESIGN_DECISIONS.md § C90](../../DESIGN_DECISIONS_VALUES.md#c90--each-nullable-scalar-reserves-one-bit-pattern-for-null-the-in-band-sentinel-residual-accepted-frozen)
   (option B ratified; per-type reserved-value table part of the contract-1 freeze). Golden pin
   `tests/scripts/pln102-null-residual-golden.loft` freezes each reserved value as a boundary (the
   pattern reads null; an adjacent value does not) on both backends. *(2026-07-11.)*

**Keystone COMPLETE** (steps 1–5). The null model is honest in the spec, uniform in comparison,
guarded at every silent-collision site, honest in stdlib return types, and its accepted residual is
a frozen design decision with a golden pin. D-op-null-1/2 CLOSED; C90 recorded.

## If A — the shape of the work (so the choice is informed)
A new tagged runtime representation for nullable scalars (extend the `__nullable<S>` machinery to
scalars), a store-layout revision (nullable fields grow a tag; revise L-Null + the layout hash + the
golden layout tests + @PLN97), the eval-stack + op rewrites for nullable scalars on both backends,
and a full re-validation (suite + native + poison + oracle + fuzz). Larger than any single item in
either audit; it would be its own multi-phase plan.

## See also
- [lib-audit.md § keystone](lib-audit.md) / [formal-audit.md § keystone](formal-audit.md) — the same
  model from the stdlib and formal/error angles.
- [types.md](../../formal/types.md) — the null model (DN1–DN6, the representation table).
- [DESIGN_DECISIONS.md](../../DESIGN_DECISIONS.md) C79/C80/C85 — the spreadsheet/zero-overhead model B preserves.
