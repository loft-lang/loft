<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 162 — the rule set, as amended

[DESIGN.md](DESIGN.md) carries the owner's rules verbatim and is not edited.  **This file is
the working set**: the same rules with the decisions taken since, plus one rule the design did
not have.  It becomes `formal/dispatch.md` when the feature lands — not before, because that
register is *rules with their deviations driven to zero* and a rule with no implementation is
a 100 % deviation.

## Unchanged from DESIGN.md

`Disp-Applicable` · `Disp-Select` · `Disp-Ambiguous` · `Disp-Closed` · `Disp-Dynamic` ·
`Disp-World` · `Disp-Match-Equiv` — as written there.

## The principle the rules keep landing on

**Where the language could either refuse or choose, it refuses — because refusal is the only
reversible direction.**

A refusal defines a SUBSET of valid programs.  Broadening it later only ever ADDS programs, so
everything that compiles today keeps compiling and keeps meaning what it meant.  A rule that
silently chooses does the opposite: it fixes the meaning of programs that would otherwise be
refused, so revising that choice later leaves them compiling and **silently doing something
different** — a compatibility break that arrives as changed behaviour rather than as an error.

Four rules here are the same decision, and they should be read as one (owner, 2026-09-11):

| rule | could have | refuses instead |
|---|---|---|
| `Disp-Ambiguous` | picked by declaration order | names both definitions |
| `Disp-Specific` | ranked unrelated abstractions by kind | incomparable ⇒ ambiguous |
| `Disp-Exhaustive` | no-op'd an uncovered call at runtime | refuses at compile time |
| *(untyped parameters)* | a total default that swallows everything | not added; the fallback is a type |

⚠ This matters more at **contract 0** than it will later: `src/manifest.rs::CONTRACT_VERSION`
is `0`, the one era with no compatibility promise.  Every "choose" shipped now becomes
permanent at the flip; every "refuse" stays broadenable afterwards.

## Amended

**Disp-Key** *(new, replaces the design's silence on how a definition is found).*  A
definition's dispatch key is built from the TYPES of its parameters and from nothing else —
not from a parameter's NAME.  `self` and `both` keep their existing meaning, which is that the
definition is *also* callable as `x.f(…)`; they no longer decide whether it dispatches.

> **A name with ONE definition keys as `n_<name>`, byte-identical to today.  A name with
> SEVERAL keys each definition by its parameter types.**

The second sentence is what makes *no existing program changes* true by construction: a
program that compiles today has one definition per name, because two would not compile.

**Disp-Specific** *(amended — the abstract position is the ENUM, not an interface).*  The
design says *"a concrete struct is more specific than any interface it implements"*.  Measured
2026-09-11: **an interface cannot be a parameter type** — `fn describe(x: Shape)` is refused
with *"Expecting a type"* — so that clause has no surface.  An `interface` is a generic BOUND
(`fn describe<T: Shape>(x: T)` works), not a type.

The subtype relation loft actually has, and the one this rule needs, is **enum ⊃ variant**: a
variant argument already widens to an enum parameter (`take(e: Entity)` accepts `Fire{…}`),
and a variant-typed definition already coexists with the enum-typed one as a separate key.  So:

> **A VARIANT is more specific than its ENUM; a CONCRETE type is more specific than a BOUND
> that admits it; a bound is more specific than a strictly weaker bound.  Two abstractions of
> DIFFERENT kinds are INCOMPARABLE — and a call to which both apply is `Disp-Ambiguous`.**

**The primary abstraction is the ENUM** (owner, 2026-09-11).  Bounds are supported — the edge
below already works — but enum ⊃ variant is the way this language is meant to be written, and
that choice buys two things generics cannot.  The lattice is a **two-level chain per
parameter** (a variant has exactly one enum) rather than an open partial order, and it is
**closed and single-owner**, so the cross-library ambiguity that motivates orphan rules in
other languages largely does not arise: an enum is declared in one place, and a library adding
a variant edits that declaration.

⚠ Closedness is not a restriction accepted reluctantly — **it is what makes the check
possible**.  See the `dispatch-pairs-uncovered` note under `Disp-Exhaustive`.

Measured 2026-09-11: the concrete-beats-bound edge **already works** —
`fn kind(self: Sq)` beside `fn kind<T: Shape>(x: T)` selects `"square"` for an `Sq` and
`"some shape"` for a `Tri`.  So a bounded generic is the OPEN abstract position this rule
needs: any type satisfying the bound, including one a library adds later, typed, monomorphised
and free.  That closes the gap the interface finding opened — the open profile is not
enum-only after all.

⚠ The cross-kind case (an `Entity` variant that also satisfies `Projectile`) is deliberately
ambiguous rather than ranked.  Owner's decision, and the reason is the principle above: **we
can broaden later.**  Ranking by kind would be an invented precedence that a reader cannot
derive from the signatures, and changing it afterwards would move programs rather than admit
them.

⚠ This narrows the rule's reach and it is an honest narrowing, not a simplification: with
interfaces unavailable as parameter types, the OPEN half of DESIGN.md's two profiles has no
abstract position.  Either interfaces gain existential/parameter use — a language change well
beyond this plan — or the open profile dispatches over enums too, which is closed by
construction.  **This is a question for the owner and it is the one the verification opened
rather than closed.**

**Disp-Ambiguous** *(amended — the tie-break rule, and the escape hatch that already exists).*

Two clauses, and neither is new policy — the second is loft#788's, extended from NAME
collisions to SPECIFICITY collisions.

> **No tie is broken by generic-ness.**  A definition that is strictly more specific wins,
> whatever kind of abstraction it uses.  Where neither dominates, the call is ambiguous and
> the compiler does NOT reach for *"but one is generic"* to choose.

> **A BARE call that is ambiguous is refused.  A QUALIFIED call is not** — `lib::hit(a, b)`
> has already said which package it means, so its candidate set is that package's and it
> resolves within it.

The second clause needs no new syntax and no new mechanism: `lib::f(…)` is an existing
callable form, and `find_fn`'s `source` parameter already carries exactly this distinction —
`source == u16::MAX` *is* what makes a call bare (`parser/mod.rs:5932`).  loft#788 already
refuses a bare ambiguous call and already permits the qualified one, with the reason stated in
its own comment: *"two packages deliberately exporting one name and calling it qualified is a
shape that works today — refusing it would break a program for a collision it has already
resolved."*

⚠ **The first clause must not catch the concrete-vs-bound CHAIN**, which compiles today:
`fn kind(self: Sq)` beside `fn kind<T: Shape>(x: T)` selects `"square"` for an `Sq` and
`"some shape"` for a `Tri`.  One is strictly more specific, so it is not a tie and nothing
refuses it.  Refusing it would NARROW current behaviour — the one direction the principle
above forbids.

**The diagnostic offers both cures, and which one applies depends on what the reader owns:**

```
error: `hit` is ambiguous for (Fireball, IceWall)
  --> two definitions apply and neither is more specific:
      fn hit(f: Fireball, d: Damageable)   src/fire.loft:12
      fn hit(p: Projectile, w: IceWall)    lib/combat.loft:40
  = if you own one: give it its own name — these definitions are sparse, so the
    name is where the distinction belongs
  = if you own neither: qualify the call — `combat::hit(a, b)`
```

The rename cure is first because it is the right one in the common case; the Julia reflex
(*"add a more specific definition"*) is usually wrong here and is deliberately not offered.

**Disp-Fallback** *(amended).*  The design says *a definition whose parameters are all
untyped*.  Untyped parameters do not exist and are not being added (README § Decision), so:
the fallback is the definition whose parameter types are the most general applicable ones —
the ENUM for a closed set, or a BOUNDED GENERIC for an open one (an interface cannot occupy
a parameter position, but a bound can — see `Disp-Specific` above).  It is less specific than every
other definition of that name by `Disp-Specific`, with no special case, because it is a type
like any other.

## New

**Disp-Exhaustive.**  A call is *covered* when some definition of its name is applicable to
the STATIC types of its arguments.  **An uncovered call is refused at compile time**, naming
the argument types and the definitions that were considered.

Soundness: every runtime type is a subtype of the static type it is held at, and
`Disp-Applicable` is monotone under subtyping — so a definition applicable to the static types
is applicable to every runtime instance of them.  Covering the static types therefore covers
every run, and no runtime *"no applicable method"* condition can arise.

This is `match` exhaustiveness one level up, and it is the rule that makes the typed fallback
a **checked obligation** rather than a convention: forget the total case and the program is
refused, where an untyped fallback would have made every call trivially covered and turned the
same mistake into a silent no-op.

⚠ **With CLOSED enums the compiler can do strictly better than this rule requires, and should
— as ADVICE.**  Every reachable variant combination is enumerable at build, so it can report
*which pairs fall through to the fallback*: `7 of your 64 (Entity, Entity) pairs reach the
default — here they are`.  For a game that is the question actually being asked — *what have I
not handled?* — and it is decidable only because the enum is closed.

It must stay advice and never an error, or it is the N² table the feature exists to escape:
the enum-typed fallback legitimately covers every remaining pair, so a program with a fallback
is complete by the rule.  Proposed code: `dispatch-pairs-uncovered`.

⚠ It is also the rule that Julia's construct **structurally cannot have** — with no static
types there is nothing to check coverage against, which is why `MethodError` is a runtime
error there by necessity.  It is the sharpest statement of what the transposition buys, and it
should be read as the load-bearing one rather than as a convenience.

⚠ And it sharpens open question 6: **pattern-clause dispatch on VALUES would make coverage
undecidable again** (does some clause match every integer?), so `Disp-Exhaustive` would have
to be dropped or degraded to a runtime check.  That is a stronger argument for type dispatch
than the lowering cost the design gives.

## Consequences worth stating

- **No runtime "no method" path exists.**  `Disp-Exhaustive` at compile time plus
  `Disp-Ambiguous` at compile time means the only runtime work is choosing among definitions
  already known to cover the call.
- **`Disp-Dynamic` is a selection, never a search.**  The set is fixed at build and known to
  be non-empty for the static types, so the runtime step is a lookup with a guaranteed answer.
