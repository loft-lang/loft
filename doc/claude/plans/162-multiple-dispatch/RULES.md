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

> **A VARIANT is more specific than its ENUM.  Otherwise specificity is type equality.**

⚠ This narrows the rule's reach and it is an honest narrowing, not a simplification: with
interfaces unavailable as parameter types, the OPEN half of DESIGN.md's two profiles has no
abstract position.  Either interfaces gain existential/parameter use — a language change well
beyond this plan — or the open profile dispatches over enums too, which is closed by
construction.  **This is a question for the owner and it is the one the verification opened
rather than closed.**

**Disp-Fallback** *(amended).*  The design says *a definition whose parameters are all
untyped*.  Untyped parameters do not exist and are not being added (README § Decision), so:
the fallback is the definition whose parameter types are the most general applicable ones —
the ENUM (an interface cannot occupy a parameter position — see `Disp-Specific` above).  It is less specific than every
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
