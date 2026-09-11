<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 162 — Multiple dispatch

## Status

**PROPOSAL — design written, nothing accepted, nothing in the tree.**  The design is
[DESIGN.md](DESIGN.md), carried verbatim as the owner wrote it.  Six open questions in it are
the owner's to answer and at least one (question 6) changes the rules, so **no phase below
starts until questions 1, 2 and 6 have answers**.

⚠ **The design says so itself and it governs this plan: every claim in it about the CURRENT
implementation was written from reading the repository, not from running it.**  Phase 0 exists
to check those claims before anything is built on them, and a mismatch is a deviation to
record — not a fact to assume.

**One such claim has already been checked, and it does not hold (2026-09-11).**  The design's
motivation section says the feature "closes INCONSISTENCY #6 — *plain enums cannot have
methods*".  Measured:

- [INCONSISTENCIES.md](../../INCONSISTENCIES.md) has **no entry 6** — the live list is 2, 8,
  18, 26, 27, and the resolved table (33, 34, 2, 3, 8, 9, 12, 17, 18, 26, 27, 28, 29, 30, 31)
  has nothing about enums and methods either.
- A second claim fails the same way: **untyped parameters do not exist** — `fn twice(a)` is
  refused with *"Expecting a clear type, found unknown"*.  So `Disp-Fallback` as written has
  no surface, and the worked example's `fn hit(a, b, world) { }` would not parse.  See
  [IMPL.md § Step 6](IMPL.md#step-6--disp-fallback--s-and-it-needs-a-surface-decision-first).
- A second claim fails the same way: **untyped parameters do not exist**, so `Disp-Fallback`
  as written has no surface — see § Decision below.
- More to the point, **a plain enum can already have a method.**  `fn describe(self: Colour)
  -> text` over `enum Colour { Red, Green, Blue }` compiles and `c.describe()` calls it,
  printing its result on `--interpret`.

So this is not a motivation for the plan and must not be carried as one.  The design is kept
VERBATIM as the owner wrote it — the correction lives here, in the plan, because that is the
document that is answerable to the tree.  If enums-with-methods is still wanted as a
motivation, it needs restating as whatever gap actually remains (perhaps: a method cannot be
defined per VARIANT of a plain enum, which is a different claim and is untested).

## Goal

A function name may carry several definitions distinguished by the types of all its
parameters, with the language selecting the unique most-specific applicable one; resolved at
compile time to a direct call wherever the argument types are statically concrete.

## Effort + design

- **Effort:** H — six rules, a new selection pass, a lowering, three backends, two profiles.
- **Design:** ~ (partial) — the rules are written; three open questions gate the first phase.
- **Last touched:** 2026-09-11

## Composition matrix — Stage A

The feature adds an *operation* (selection at a call), so the matrix is required before the
implementation phases.  Write these as `/tmp` probes on `--interpret` first; they graduate to
`tests/scripts/162-*.loft` as the regression suite.  Axes the change actually touches:

| Axis | Domain to cover |
|---|---|
| **Parameter count** | 1 (the degenerate case = today) · 2 (the motivating case) · 3+ |
| **Parameter kind** | concrete struct · interface · untyped · **mixed within one signature** |
| **Argument staticness** | all statically concrete (Disp-Closed) · some dynamic (Disp-Dynamic) · all dynamic |
| **Specificity shape** | a total order · a partial order with a unique minimum · **ambiguous** (must refuse) |
| **Arity** | same-arity definitions only, vs. a name carrying two arities — *is that one name or two?* Not answered by the design; the matrix is where it gets decided |
| **Return type** | all definitions agree (open question 4's proposal) · they disagree (must refuse) |
| **Method count** | 1 definition (must be identical to today's emission) · 2 · many |
| **Backend** | `--interpret` · `--native` · `--native-wasm` — every cell, all three |

⚠ The **1-definition** row is the one that is easy to leave out and the one that protects
every existing program: a name with a single definition must emit byte-identically to what it
emits today.  If that cell moves, dispatch has changed the cost of code that does not use it.

## The finding that reshapes this plan

**Single dispatch already works, keyed on the FIRST parameter only** (measured 2026-09-11):
`fn hit(self: Fire)` and `fn hit(self: Ice)` coexist, and both `f.hit()` and the bare
`hit(f)` resolve by argument type.  Add a second parameter and they collide —
*cannot redefine method `hit` on `Fire`*.

So this plan is **widening an existing key**, not building a dispatcher.  The key has two
homes — `Data::get_fn` (write) and `Data::find_fn` (read) — and `Data::bound_stub_name`
already folds a marker and an arity into a key, which is the shape the widening follows.
[IMPL.md](IMPL.md) is the step-by-step design; it is written against the tree with a file and
line for every claim.

## The findings that reshape this plan

Measured 2026-09-11, before any design was written.  Both change what the feature IS.

**1. Single dispatch already works — the syntax exists.**  `fn hit(self: Fire)` and
`fn hit(self: Ice)` coexist today, and both `f.hit()` and the bare `hit(f)` resolve by
argument type.  Add a second parameter and they collide — *cannot redefine method `hit` on
`Fire`*.  So the user-visible change is **an error message going away**: nothing new to
learn, no keyword, no construct.  The plan is widening an existing key, not building a
dispatcher.

**2. Dispatch is tied to the parameter NAME `self`, and that is the wart to remove.**  The
key is only built when `arguments[0].name == "self"` / `"both"` (`src/data.rs:7568`), so
ordinary parameter names collide:

```loft
fn hit(f: Fire) { … }
fn hit(i: Ice)  { … }     // error: Cannot redefine 'hit'
```

A programmer who just writes the function for a specific case therefore meets a redefine
error whose cure is to rename a parameter.  **Decision (owner, 2026-09-11): key on the
parameter TYPES, whatever the parameters are named.  `self` keeps its current meaning and
only that — the definition is also callable as `x.f(…)`.**  Dispatch and method-call sugar
become independent.

**And existing programs are safe by construction, not by testing.**  145 sites in `src/` look
a free function up as `n_<name>`, so the rule is: *a name with ONE definition keys as
`n_<name>`, exactly as today; a name with SEVERAL keys by parameter types.*  A program that
compiles today has one definition per name — if it had two it would not compile — so no
existing key moves.  The only programs whose behaviour changes are ones currently refused.

**3. The hard part is not the key — it is the ORDER.**  Both spellings resolve the definition
*before* the argument types exist: `find_fn` at `parser/fields.rs:617` runs before
`parse_method` at `:622`, and `def_nr("n_<name>")` at `parser/control.rs:16602` runs ~180 lines
before `types.push(t)` at `:16785`.  `find_fn`'s own doc says it — *"a method call's arguments
are not parsed when its receiver is resolved."*  Multi-parameter dispatch needs the opposite
order, so resolution becomes two-phase — a candidate SET at the name, SELECTION once the
types are known — with today's single answer as the degenerate case.  That is why this is `H`
effort and why phase A of the implementation is five behaviour-preserving steps before
anything changes.

[IMPL.md](IMPL.md) is the step-by-step design, with a file and line for every claim;
[RULES.md](RULES.md) is the rule set as amended.

## Decision — no untyped parameters

DESIGN.md's `Disp-Fallback` is *a definition whose parameters are all untyped*.  **Untyped
parameters do not exist** (`fn twice(a)` → *"Expecting a clear type, found unknown"*), so the
worked example's `fn hit(a, b, world) { }` would not parse — and after a search for use cases,
they should not be added.  Both things they would be wanted for already have typed, zero-cost
spellings:

| Wanted for | Already served by |
|---|---|
| the dispatch total fallback | the most general TYPE — `Entity`, or an interface; already a working key |
| "accepts literally anything" | `fn dump<T>(x: T)` — an unbounded generic, **measured** accepting a struct, another struct and an integer, monomorphised |

Two reasons beyond redundancy:

- **Efficiency splits the feature in two and neither half pays.**  An untyped parameter that
  can never be READ is a wildcard — zero cost, but it buys exactly what the typed fallback
  buys.  One that can be read must hold a value of unknown type, which is `Any` plus boxing —
  the first entry on DESIGN.md's own non-goals list.
- **It would make the exhaustiveness check unfireable.**  With a typed fallback, *a call must
  have a definition covering its STATIC argument types* is sound (every runtime type is a
  subtype) and decidable at compile time — `match` exhaustiveness one level up.  A total
  untyped default makes every call trivially covered, so a forgotten pair becomes a silent
  no-op instead of a refusal.  And it stops `Disp-Specific` being a pure subtype relation.

That check is now a rule — **`Disp-Exhaustive`** in [RULES.md](RULES.md), with its soundness
argument and the note that it is the one thing Julia's construct structurally cannot have.

Step 6 of [IMPL.md](IMPL.md) is therefore expected to be a no-op.  What would reopen this: a
case where an argument's type is genuinely unknowable at the call site *and* the body must use
the value — that is `Any`, and it deserves its own proposal weighed against the ownership and
artifact-size commitments, not a side door through dispatch.

## Sub-arcs

`Verify` names the comparison that would go RED if the phase were done wrong.  The phases are
cut finer than the design's four-step landing order, because several of its steps have no
half-done state to compare against — see [§ Phase cutting](#phase-cutting-why-these-and-not-the-designs-four).

| Item | Source | Verify | Status |
|---|---|---|---|
| **0** — verify the design's claims about the tree; record the canonical-`match` answers | [DESIGN.md](DESIGN.md) §Where it lands, §Worked example | the 12-row expected-results table, measured through a hand-written `match`, identical on all three backends — **before** dispatch exists | Started — the INCONSISTENCY #6 claim checked and struck (see Status); §Where it lands' select-then-monomorphise claim still unchecked |
| **1** — `Disp-Applicable` / `Disp-Specific` / `Disp-Select` / `Disp-Fallback` as a pure function over a method table, no lowering | DESIGN.md §Semantics | a unit test over synthetic signatures asserting the selected definition per argument tuple, ambiguous tuples included | Open |
| **2** — `Disp-Ambiguous` as a compile-time refusal | DESIGN.md §Ambiguity check | the ambiguity program fails to compile, and the message names **both** definitions | Open |
| **3** — `Disp-Closed` lowering for statically-concrete sites | DESIGN.md §Static resolution | `loft introspect` byte-identical to the hand-monomorphised equivalent; and the 1-definition matrix row byte-identical to today | Open |
| **4** — the acceptance program through dispatch | DESIGN.md §Worked example | the same 12 rows phase 0 recorded, now via dispatch, three backends | Open |
| **5** — DCE / slim-artifact property | DESIGN.md §The two profiles | an unreferenced method is absent from the stripped artifact; artifact size unchanged vs. the `match` form | Open |
| **6** — `Disp-Dynamic` | DESIGN.md §Runtime resolution | a heterogeneous `vector<Entity>` reproduces phase 0's rows; plus a control that a concrete site still emits a direct call and no table | Open |
| **7** — `Disp-World` (open profile) | DESIGN.md §Disp-World | add a method mid-run; the new selection is taken AND a marker in the stale specialisation's body never appears | Open |
| **8** — `Disp-Match-Equiv` in the differential oracle | DESIGN.md §Disp-Match-Equiv | a dispatch set and its canonical `match` compared as two programs, per the oracle's existing shape | Open |

## Phase cutting — why these, and not the design's four

The design's landing order is right about sequence and too coarse to validate.  Its phase 1
bundles six rules, a lowering and the acceptance program: half-way through there is nothing
exact to compare against, which is the upper bound the plan workflow forbids.  The cut above
splits it where a comparison exists.

Two of the splits are load-bearing:

- **0 before 1.** Recording the canonical-`match` answers *before* dispatch exists is what
  makes phase 4 a comparison rather than an inspection.  It is also the cheapest possible
  falsification of `Disp-Match-Equiv`: if the three backends do not already agree on the
  `match` form of the acceptance program, the rule is in trouble before a line is written.
- **1 before 3.** Selection as a pure function can go red on its own — a wrong partial order
  picks the wrong definition and a unit test says so.  Folded into the lowering it could only
  be tested by running programs, where a selection bug and a lowering bug look alike.

Phase 5 is separate from 3 for the opposite reason: it is the only phase whose failure is
invisible in behaviour.  A dispatch implementation that quietly retains every method passes
every value test and silently costs the slim artifact its whole point.

## Phase ordering

1. **0** — pre-flight.  Gated on open questions 1, 2 and 6 being answered.
2. **1 → 2 → 3 → 4** — the closed-world core, in order; 4 is the first phase with a
   user-visible feature.
3. **5** — the artifact property, once 3 lands and there is something to strip.
4. **6** — dynamic selection.
5. **7** — the open profile, last: it is the only phase that depends on the promote path.
6. **8** — the oracle pairing, any time after 4.

## Open design questions

The six in [DESIGN.md §Open questions](DESIGN.md#open-questions--the-owner-decides), unchanged
and unanswered.  Three of them gate phase 0:

- ~~**Q1**~~ **ANSWERED 2026-09-11 — no, and it opens a bigger question.**  An interface
  cannot be a parameter type (`fn describe(x: Shape)` → *"Expecting a type"*); it is a generic
  BOUND.  The subtype relation loft has is **enum ⊃ variant**, and that is what
  `Disp-Specific` now uses.  ⚠ **But that leaves the OPEN profile without an abstract
  position** — enums are closed by construction, so a library adding a type must edit the
  central enum, which is the "addition not edit" motivation partly back.  Either interfaces
  gain parameter use or the open profile is enum-only.  **RESOLVED the same day by a further
  measurement: a BOUNDED GENERIC is the open abstract position, and it already works** —
  `fn kind(self: Sq)` beside `fn kind<T: Shape>(x: T)` already selects the concrete one for an
  `Sq` and the generic for a `Tri`.  Open, typed, monomorphised, no new type form.
- **Cross-kind specificity — ANSWERED: incomparable ⇒ ambiguous** (owner, 2026-09-11).  An
  `Entity` variant that also satisfies a bound matches two definitions of different kinds;
  neither is ranked.  The reason is the general one now stated in
  [RULES.md § The principle](RULES.md#the-principle-the-rules-keep-landing-on): **refusal is
  the only reversible direction** — broadening later adds programs, while a silent choice
  revised later moves programs that already compile.
- ~~**Q2**~~ **ANSWERED — select-then-monomorphise, as wanted.**
  `try_generic_instantiation(first_id, &types)` already takes the argument types and REPLACES
  the chosen `def_nr`, so a resolution getting revised once types are known is an existing
  shape, not a new one.
- **Q6** (type dispatch or pattern-clause dispatch?) — if pattern-clause, `Disp-Applicable`
  and `Disp-Specific` grow value and guard cases, `Disp-Closed`'s direct-call lowering stops
  holding for value-discriminating definitions, and `Disp-Match-Equiv` becomes bidirectional.
  Every phase below 1 changes shape.

Q3, Q4 and Q5 can be answered later — they gate phases 7, 1 and the catalogue entry
respectively, not the start.

## Cross-arc dependencies

- ~~**INCONSISTENCY #6**~~ — **struck; see Status.**  There is no entry 6 and plain enums
  already take methods, so nothing in
  [INCONSISTENCIES.md](../../INCONSISTENCIES.md) depends on this plan.
- **`formal/interfaces.md`** — Q1 decides whether the abstract-parameter case is `interface`
  or a new notion; either answer edits that doc.
- **The promote path** — phase 7 is an invalidation rule *on top of* live-reload, not a new
  mechanism; it cannot be designed before that path's current shape is confirmed (phase 0).
- **The differential oracle** — phase 8 adds a pairing rather than a backend.

## See also

- [DESIGN.md](DESIGN.md) — the proposal, verbatim.
- [IMPL.md](IMPL.md) — the implementation design in small steps, measured against the tree.
- [RULES.md](RULES.md) — the rule set as amended: `Disp-Key`, the restated `Disp-Fallback`,
  and the new `Disp-Exhaustive`.  Becomes `formal/dispatch.md` when the feature lands.
- [IMPL.md](IMPL.md) — the implementation design in small steps, measured against the tree.
- [`loft-lang/plans` #162](https://github.com/loft-lang/plans/issues/162) — `@PLN162`, the
  issue this plan IS.
- [INTERFACES.md](../../INTERFACES.md) · [`formal/interfaces.md`](../../formal/interfaces.md)
  — the abstract-parameter question.
- [INCONSISTENCIES.md](../../INCONSISTENCIES.md) — checked; carries nothing this plan closes.
- [GOALS.md](../../GOALS.md) — the slim-artifact and closed-world commitments phase 5 defends.
