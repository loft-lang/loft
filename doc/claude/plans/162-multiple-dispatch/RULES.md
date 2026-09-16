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
`Disp-World` · `Disp-Match-Equiv` — as written there (the last two have an implementation
form under § New, which says where each landed in the tree).

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

**Implementation form (IMPL.md step 6, 2026-09-14):** a name with several definitions is a
bare `Dynamic` DISPATCHER whose attributes are its overloads — the shape a `both` name has
always had — and each overload is keyed by its FULL parameter spelling, every declared
parameter's key name joined with `#`: a method overload as `t_10Fire#Crate_melt`, a FREE
overload as `f_10Fire#Crate_touch` — its own prefix, because `t_` means "a method" to every
reader of a key and a free overload has no receiver and no `x.f(…)` spelling.  A `both`/
`self` definition alone under its receiver keeps `t_<sig0>_<name>` and a free definition
alone keeps `n_<name>`.  When a free name gains a second definition, the incumbent is re-keyed to
its full spelling and `n_<name>` is retired, so the name offers no parse hint (`Disp-Hint`)
and every site that reads `n_<name>` as THE definition finds none, exactly as for a `both`
name.  Three consequences worth stating:

- **Scope is one SOURCE, and a library's set reaches its consumers whole.**  A MAIN definition
  under a stdlib name stays the C95 refusal, a library's names stay module-scoped (C97).  A
  library's overload set is reached by every import spelling — wildcard bare, selective,
  qualified, aliased, the method spelling, the library's own calls — through its bare
  dispatcher (measured 2026-09-14: `tests/scripts/a-library-exports-an-overload-set.loft` and
  its selective/aliased twin, both backends; the "untested" flag an earlier draft carried here
  was wrong in the safe direction).  A consumer's definition of ANOTHER spelling of the name
  joins the dispatch, kept live by its argument types, as a free function beside a stdlib
  `both` set always was; one whose spelling the set ALREADY CARRIES is a redefinition,
  refused naming the library's position — the collision an imported single `n_<name>` and
  `shadows_a_method` already refuse, which the first cut missed (a bare call answered the
  library's body while the consumer's own definition sat unreachable in silence).  Two
  packages exporting a set of one name stay loft#788's refusal: call it qualified.
- **The spelling is the key's, so it is as coarse as the key.**  Two `vector<τ>` spell alike
  (the element type is not in a key today), so `f(vector<integer>)` beside
  `f(vector<text>)` is a redefinition, as it was.
- **Selection today is EXACT:** a call reaches the overloads whose parameters spell its
  argument types position for position, a trailing parameter admitted when it has a default;
  one is the answer, none falls to the old ladder (`Disp-Exhaustive`'s message when that
  finds nothing), more than one is `Disp-Ambiguous`.  Today only a DEFAULT can produce more
  than one — a defaulted trailing parameter beside a shorter definition, or two defaulted
  parameters at one arity, for the call that omits the argument — and it is refused naming
  both, in both call spellings (IMPL.md step 7; the method spelling took the slot's routine
  in silence until then).  Ranking the exact-arity definition above the default-filled one is
  additive and may follow.

- **Selection over the enum lattice (IMPL.md steps 8–9, 2026-09-14), the static half:**
  `Disp-Applicable` is the parser's own `can_convert`, so a variant satisfies its enum
  (@FR-C-Var) and a present value a `τ?` slot; `Disp-Specific` ranks each position — exact,
  widened (variant to enum, `τ` into `τ?`), lossy (`τ?` into `τ`), converted — and a
  definition is more specific when no worse at every position and better at one;
  `Disp-Select` is the unique minimal one, a tie is `Disp-Ambiguous`, none is the ladder and
  then `Disp-Exhaustive`.  The `(F-Recv)` nullability routing runs before ranking.  What a
  value held STATICALLY at the enum reaches is the enum-level definition; its runtime
  variant is step 13's answer: `parser::dispatch::dynamic_dispatcher` synthesises, per
  (name, spelling) and on pass 2, the canonical `match` over every enum-held position, each
  leaf calling what `Disp-Select` picks for that variant tuple (`Disp-Dynamic`), a tuple no
  definition takes or two take without ranking refused naming it, and a set covering every
  tuple admitted without an enum-level definition.  And @F20's synthesised enum dispatcher
  yields to an author's enum-level definition of the name — `Disp-Fallback`'s most general type is the
  author's to write — and to a FREE overload set over variants, which is no method and gets
  no `x.f(…)` spelling (step 10 found the synthesiser hanging a dispatcher on one and the
  uncovered call reading as *did you mean the method*).

- **`Disp-Closed` holds by construction (IMPL.md step 11, measured 2026-09-14):** selection
  runs at parse time, so a statically-concrete site is a plain `Call` of the selected
  definition in IR, bytecode and native — byte-identical to a hand-monomorphised twin once
  the callee names are normalised (`tests/introspect_dispatch.rs`).  No runtime table exists
  for such a site — and so an overload nothing calls is unreachable and absent from the
  shipped artifact (`--native-release` emits only reachable functions; the semantics lane
  keeps every tier by design), the slim-artifact property step 12 pins in the same file.

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

⚠ It also settles what open question 6 would have cost: **pattern-clause dispatch on VALUES
would make coverage undecidable again** (does some clause match every integer?), so
`Disp-Exhaustive` would have had to be dropped or degraded to a runtime check.  That is a
stronger argument for type dispatch than the lowering cost the design gives.  **Q6 is
answered — type dispatch, and no other matching on a definition (owner, 2026-09-14;
[README.md § Decisions taken](README.md#decisions-taken-owner), item 5).**  The owner's own
reason is narrower than either argument: no more complexity on `fn` definitions, with
`match` kept as the home of every value-shaped decision.  A parameter DEFAULT is the one
optionality a definition already has and keeps.

**Disp-Hint** *(new — the hint circularity, decided at IMPL.md step 3, 2026-09-14).*
Parsing an argument is TYPE-DIRECTED by the definition being called: a `|x|` lambda takes its
parameter types from the parameter, a vector literal takes its element width from it, a named
argument is matched against its name.  That hint is the ONE definition of the name.  **A name
with several definitions offers no hint**: its arguments parse from their own spelling alone,
and an argument that cannot be typed without one is refused at the call, naming the
definitions considered and the cure — the typed lambda form (`fn(x: integer) { … }`), a
literal carrying its width, the argument spelled positionally.

Chosen over the alternative — hinting wherever every candidate AGREES on that parameter's
type — by [the principle](#the-principle-the-rules-keep-landing-on): refusal is the only
reversible direction.  Agreement-hinting can be added later and only ever admits programs;
shipping it first fixes a meaning for programs the refusal would have kept open.  It is also
the cheaper half: under `Disp-Key` a multi-definition name has no `n_<name>` key, and the hint
sites (`parser/control.rs`, the two `hint_d_nr` lookups) consult nothing else, so the refusal
falls out with no code and the message is the whole work.  A single-definition name — every
program that compiles today — keeps its hint unchanged.

**Disp-Exhaustive, implementation form (IMPL.md step 10, 2026-09-14).**  A bare call on an
overload set that no definition takes, and that today's ladder cannot resolve either (the free
`n_<name>` beside a `both` set, the operator map), is refused *no definition of `f` takes (τ₁,
τ₂) — declared: f(…), f(…)*.  The method spelling on a set whose receiver already carries the
method is refused by that method's own argument check, which names the parameter; the two
messages differ in wording and agree in verdict.  The enumerated-pairs ADVICE the rule text
above proposes (`dispatch-pairs-uncovered`) is not built.

**Disp-World, implementation form (IMPL.md step 14, 2026-09-14).**  The open profile is
`LOFT_LIVE_RELOAD=1`, and there is no promoter under it — the tree's "interpret, then
promote" is @PLN18's tier 0, a body swap of an existing named fn.  So the rule lands on that
boundary.  Under the open profile every call into an overload set is lowered to a
per-(name, spelling) synthesised function — the `__dyn_` dispatcher a dynamic site already
had (step 13), and a `__sel_` stub at a static site whose body is the one direct call
`Disp-Select` picked; the closed profile keeps step 11's direct call and is byte-identical.
That function IS the specialisation the rule speaks of, and the world counter is the reload
host's version: an overload added mid-run joins the shadow session's set, every
specialisation of the name is rebuilt in the new world and swapped in behind its old def
through tier 0's own patch (`fn_positions` + every recorded call operand), so the running
loop's next call selects in the new world and a body selected in an earlier world never runs
again — invalidation is eager, so no specialisation needs to carry the world it was built in.
The whole add is one transaction.  **Q3 answered as the design proposes, from the principle
above: the ADD is refused** — a rebuild that leaves a served tuple ambiguous, or one no
definition takes, is reported naming the tuple and the world is unchanged; a removed or
re-signatured overload is refused too (the world only grows, and a signature is the frame a
call site embeds); and a second definition of a name that was ONE function is refused,
because its sites were direct calls with nothing to rebuild.  Not reached: ADDING a `self`
overload mid-run — since D-disp-1's closure a `self` set over an enum with an enum-level
member is an ordinary set, and its values under the open profile are measured
(`a-method-at-the-enum-is-the-wildcard-…`, `35-dispatch-self-set` under
`LOFT_LIVE_RELOAD=1`), but no add was tried — and the native live-flip binary, whose compiled
callers keep the world they were built in until flipped — recorded, not closed.

**Disp-Match-Equiv, in the oracle (IMPL.md step 14, 2026-09-14).**  A dispatch set and its
canonical `match` are two programs of the differential-oracle corpus, `tests/oracle/34-…`,
and the set's side declares `@ORACLE_TWIN: <the match>`: the sweep holds the pair to one
stdout on top of holding each to its own three backends, with a positive control that a
differing line or exit is caught.

**One return type per dispatch (DESIGN.md Q4, answered — 2026-09-15).**  A
call decided by the runtime variant is one synthesised function with one return type, so the
definitions it chooses between must agree on it.  `val(f: Fireball) -> integer` beside
`val(e: Entity) -> text` read the text through the integer's frame on the interpreter (`[2]`
where the body says `e2`) and did not compile on `--native` — for a free set since step 13, and
for @F20's own dispatcher before any of this plan.  Such a call is now refused naming the two
definitions; a static call reaches one definition and is untouched (`width(integer) ->
integer` beside `width(text) -> text` compiles and runs).  **That is the whole rule (owner,
2026-09-15, answering Q4): the definitions of a NAME may return different types; only a
dispatch decided at run time needs the definitions it chooses between to agree.**  The design's
proposal — refuse the name — was declined: it would refuse the stdlib's own overload sets
(`abs` on an `integer` returns `integer`, on a `single` returns `single`) and working programs
like `width`, for a benefit, one result type to infer, that only a runtime dispatch needs.

## Deviations

OPEN: **0**.

- **D-disp-1 — CLOSED 2026-09-15** (opened 2026-09-14, narrowed the same day by step 13).
  What remained after step 13 was the `self` set over variants, in two facets, and the owner
  decided both in one sentence: *the refusal is the same shape as the match without `_`, and
  the enum-level definition is the `_` variant — every case with no more specific
  implementation.*
  - **The enum-level member is `_`.**  `kind(self: Fireball)` beside `kind(self: Entity)` — or
    beside a free `kind(e: Entity)` — lived under different keys (`t_8Fireball_kind`,
    `t_6Entity_kind`, `n_kind`), so nothing made them one set: no bare dispatcher existed,
    @F20 yielded to the enum-level definition, and a receiver held at the enum answered the
    enum-level body whatever its variant — on both backends, with no diagnostic, even with
    every variant implemented.  `Parser::join_enum_lattice_sets` now makes every definition
    of one name whose first parameter is an enum or one of its variants ONE overload set
    (once the file's definitions are known, before @F20 asks whether a set owns its
    dispatch), so `Disp-Specific` ranks the variant above the enum and step 13's dispatcher
    builds the `match`.  A group with no enum-level member stays @F20's, whose dispatcher
    already is that `match`.
  - **A missing variant is `match` without `_`.**  With `tag(self: Fireball)` and
    `tag(self: IceWall)` declared and `Crate` left out, a call through `Entity` was a
    WARNING at `Crate`'s declaration and an EMPTY value at run time.  It is now refused at
    the call, in `M-Exhaust`'s shape (*call of `tag` on `Entity` is not exhaustive —
    missing: Crate; add … or a fallback `fn tag(self: Entity)`*), by
    `Parser::refuse_uncovered_variants` at `call_nr`, where both spellings and every
    receiver spelling arrive.  A method only some variants have, called only on those
    variants, dispatches nothing and says nothing — the warning fired for it too.
  The warning was the leniency that let a half-built set run; at contract 0 turning it into
  the refusal is the owner's call, taken.  Measured over 1 554 corpus files and 3 331 more
  (the repo's fixture libraries, tools and docs, and nineteen sibling projects): the only
  programs refused are the two that pinned the old behaviour; `813-variant-bounded-generic`
  forms a set and answers unchanged.  Guards: `tests/scripts/a-method-at-the-enum-is-the-wildcard-for-variants-without-their-own.loft`,
  the refusals in `tests/parse_errors.rs`, and `tests/oracle/35-dispatch-self-set.loft` held
  to its `match` twin.  A `self: Fireball?` member beside `self: Entity` is `Disp-Ambiguous`
  for a `Fireball` held at the enum, exactly as the free pair is (nullability and the enum
  are incomparable abstractions).

- **D-disp-2 — CLOSED 2026-09-15** (opened the same day, found closing D-disp-1).  A
  NULLABLE enum position stayed static (step 13: *null has no variant*), so an `Entity?`
  holding a `Fireball` reached the definition its STATIC type selects — behind the `(N-Store)`
  warning where that definition takes a dense `Entity`, and in SILENCE where it takes
  `Entity?` — while the canonical `match` over the same value takes the `Fireball` arm
  (measured on both spellings, a trailing parameter, a loop over call results, and two
  positions: `hit(Entity?, Entity)` answered the fallback for a `(Fireball, IceWall)` pair).
  A nullable position is now dynamic whenever the call HAS a static selection: a present
  value's variant decides as a dense one's does, and a null — which has no variant — reaches
  that static selection, exactly what it reached before.  The dispatcher declares each
  nullable position at the nullability the static selection declares there, so the call into
  it is checked as the direct call was (the same `(N-Store)` warnings at the same sites with
  the same text, measured), and a `Disp-World` rebuild recovers what the call routed from the
  specialisation's spelling.  A call with no static selection keeps its nullable positions
  static and is answered as before.  Guards:
  `tests/scripts/a-nullable-enum-argument-is-dispatched-on-its-variant.loft`, the nullable
  cells of the two step-13 guards (which pinned the static answer, now flipped), and
  `tests/live_world.rs` (`a_nullable_dynamic_site_keeps_its_null_leaf_across_a_rebuild`,
  which fails with the spelling step removed).

## Consequences worth stating

- **No runtime "no method" path exists.**  `Disp-Exhaustive` at compile time plus
  `Disp-Ambiguous` at compile time means the only runtime work is choosing among definitions
  already known to cover the call.
- **`Disp-Dynamic` is a selection, never a search.**  The set is fixed at build and known to
  be non-empty for the static types, so the runtime step is a lookup with a guaranteed answer.
