<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Multiple dispatch — design proposal

**Status: PROPOSAL.** Not decided, not scheduled, nothing in the tree. Written 2026-09-11
from a design discussion comparing Julia's constructs against Loft's commitments. Every
construct below is *proposed* surface until the owner accepts it. Every claim below about the
*current* implementation was made from reading the repository, not from running it — the
coding agent must verify each one against the tree before relying on it, and record what it
finds as deviations in the usual way.

## What this is, in one sentence

Let a function name have several definitions distinguished by the concrete types of **all**
its parameters, and have the language select the unique most-specific applicable one at
each call. Julia calls this multiple dispatch. This document gives (1) why Loft wants a
*restricted* form of it, (2) the exact form, (3) its semantics as formal rules, (4) how it
lands on the three backends and the two deployment profiles, (5) a worked gaming example
that doubles as the acceptance program, and (6) what **not** to take from Julia.

---

## Motivation

### It completes a notation Loft already has

Loft already lets a function be defined per enum struct variant — a pure `fn` notation in
which the variant a definition covers is stated in its signature rather than tested in its
body. That is single dispatch over a closed set of variants, and it was the original design
intuition for how discrimination should read in Loft. Multiple dispatch is that same
notation generalised along two axes at once: from **one** parameter to **all** of them, and
from a **closed** set of variants to **open and abstract** types. Nothing about the way it
reads is new to Loft; what is new is how far it reaches. Read this proposal as *finishing a
notation the language already has*, not as importing one from Julia — Julia is where the
generalisation is best worked out, which is why it is the reference, not the motivation.

### The problem it solves: behaviour that depends on two types

The canonical game case is pairwise interaction — what happens when *this* hits *that*.
Fire melts an ice wall but ice reinforces it; an arrow embeds in a crate but shatters on ice;
a fireball and an ice bolt cancel each other mid-air. None of that is a property of one type.
Single dispatch (a method on a receiver) structurally cannot express it; today Loft expresses
it as one central `match` on a pair of enum variants. That table has four costs that grow
with the game:

- It is N² in entity types, and every new type reopens the one central function.
- Fire logic, ice logic and arrow logic are interleaved in one body — "everything fire does"
  lives nowhere.
- The same fallback ("any projectile hits the player") is written once per projectile type.
- Adding a type is an *edit* to a shared file, not an *addition* of a self-contained one —
  which is exactly the shape the feature-tag / library modularity wants to avoid.

Multiple dispatch dissolves the table into only the pairs that do something, each placed
next to the type it concerns, with fallbacks supplied once by specificity.

### It closes INCONSISTENCY #6

"Plain enums cannot have methods." With dispatch, behaviour attaches to a variant by
defining a method over the variant's type — no `impl` block on the enum is needed.

### It fits both deployment profiles — differently

Loft ships two shapes, and they want opposite dispatch policies:

- **The slim game artifact** is closed-world: every method is known at build, whole-program
  dead-code elimination is preserved, no runtime code generation. Dispatch resolves at build.
- **The long-lived project** (server, persistent process) already adds scripts at runtime
  and promotes them to their optimised form. This is the *open* profile, and it is the one
  place where Julia's "specialise when a new type combination is first met" fits naturally —
  because the interpret-then-promote boundary it needs already exists.

The same construct, two policies, keyed to the profile. §"Where it lands" spells this out.

---

## The construct (proposed surface)

1. Several `fn` definitions may share a name; they differ in the types of their parameters.
2. A parameter is typed by a **concrete struct**, by an **interface** (abstract), or left
   **untyped** (accepts anything; least specific).
3. At a call, the *applicable* methods are those whose parameter types accept the argument
   types. The **unique most-specific** applicable method is selected.
4. Single-parameter dispatch is the degenerate case of this; a plain function today is the
   degenerate case of *that* (one definition).
5. **No new keywords.** The one piece of surface to confirm is using an `interface` as a
   parameter type for the abstract case — check this against `formal/interfaces.md`.

```loft
fn damage(f: Fireball)   -> integer { 3 }
fn damage(b: IceBolt)    -> integer { 2 }
fn damage(p: Projectile) -> integer { 1 }   // any other projectile — the fallback
```

Illustrative Loft; the dispatch (three definitions of one name) is the proposed part.

---

## Relationship to `match` — an alternative notation, not a replacement

**`match` is not limited, reduced, or discouraged by this proposal in any way.** It stays a
complete construct, fully available to a programmer who can wrap their head around a
decision better in that form. Dispatch adds a *second* notation for the same decision, one
that reads cleanly when a `match` would grow complex — many discriminants, many arms, arms
that want to live next to the types they concern. The programmer chooses; the two must mean
the same thing.

That equivalence is the load-bearing property, and it is checkable. For any set of dispatch
definitions of a name there is a canonical `match` over the same discriminants; the two must
select the same body for every argument, on every backend. This is written as a rule below
(**Disp-Match-Equiv**) precisely so that the two notations can never drift apart, and so the
differential oracle can hold them to it — a dispatch set and its canonical `match` are two
programs that must agree, in the same sense that the three backends must.

### What the fn notation can express — the fork this proposal takes

"Dispatch by definition" can mean two different things, and the difference sets how much of
a complex `match` has an fn-notation equivalent:

- **Type dispatch** (Julia): definitions are distinguished by the *types* of their parameters
  — which struct, which interface, which variant. Selection compiles to a jump the moment the
  types are known.
- **Pattern-clause dispatch** (Haskell, Erlang): definitions are distinguished by *patterns* —
  constructors **and values**, with guards — `f(0) = …`, `f(n) when n > 5 = …`, a nested
  destructure inline. Under this model clauses can express nearly any `match`.

This proposal takes **type dispatch**, and the rules in the next section assume it. The
consequence for notation coverage is exactly this: a `match` that discriminates on *what kind
of thing* has an fn-notation equivalent; a `match` that discriminates on a *value* — a literal,
a range, a guard, an inline destructure — is expressible in `match` alone. That is not a
limitation on `match`, which expresses both; it is a limit on how far the alternative notation
reaches.

The reason to draw the line at types is the property that motivated dispatch for the slim
artifact in the first place. Type dispatch resolves at compile time to a direct call. Value
dispatch cannot — there is no type to jump on — so it degrades to a runtime chain of tests,
which is a `match` again, only scattered across definitions and without the local
exhaustiveness check a single `match` gives. Extending the fn notation to values would widen
its coverage at the cost of the zero-cost static resolution that is the point of having it.
The owner decides where the line sits (open question 6); this proposal recommends types, as a
coverage decision, with `match` remaining the full notation for everything.

---

## Semantics as formal rules

Candidate file: `formal/dispatch.md`, following the house convention — named **Rules**,
numbered **Deviations**, and *the code changes to match the rules, not the reverse*.

**Disp-Applicable.** A method is applicable to a call iff every argument's type — static
where known, runtime where not — is accepted by the corresponding parameter type. An untyped
parameter accepts every type.

**Disp-Specific.** Method M1 is *more specific* than M2 iff every parameter type of M1 is a
subtype of, or equal to, the corresponding parameter type of M2, and at least one is strictly
a subtype. This is a partial order on the tuple of parameter types. A concrete struct is more
specific than any interface it implements; any typed parameter is more specific than an
untyped one.

**Disp-Select.** The selected method is the unique most-specific applicable method.

**Disp-Ambiguous.** If two applicable methods exist with neither more specific than the
other, and no third applicable method is more specific than both, the call is *ambiguous*.
In the closed-world profile this is a **compile-time error naming both methods**. (Open
profile: see Disp-World.)

**Disp-Fallback.** A definition whose parameters are all untyped is applicable to every call
and less specific than every other method of that name: it is the total default.

**Disp-Closed.** In a closed-world compilation the method set of a name is fixed at build.
Selection for a call site whose argument types are all statically concrete is performed at
compile time and lowers to a direct call; no runtime table exists for such a site.

**Disp-Dynamic.** A call site whose argument types are not all statically concrete selects
at runtime over the build-time method set. The selection is deterministic and equals what
Disp-Select gives for the runtime types.

**Disp-World** *(open profile only).* The method set of a name carries a monotonic *world*
counter; adding a method bumps it. Every promoted or compiled specialisation records the
world in which it was selected. A specialisation is **invalid and must not run** once a
method added in a later world would change Disp-Select for any call it serves. This is the
rule that makes "I added a script" never mean "an old specialisation now silently lies" —
the runs-right-once-wrong-later family the heap-correctness release was about, transposed
to code instead of storage.

**Disp-Match-Equiv.** For every set of dispatch definitions of a name there exists a
canonical `match` over the same discriminants — one arm per definition, in specificity order,
with the Disp-Fallback definition as the `_` arm. The two are observationally equivalent:
for every argument tuple, the body selected by Disp-Select is the body the canonical `match`
would run. This holds identically on all three backends. The rule exists so that dispatch and
`match` are two notations for one meaning and can never drift apart; it also makes the pair a
natural differential-oracle case — a dispatch set and its canonical `match` are two programs
that must agree. Note the equivalence is stated in one direction: every dispatch set has a
canonical `match`, but not every `match` has a dispatch equivalent (see §Relationship to
`match` — value-discriminating arms are `match`-only under type dispatch).

**Deviations:** none recorded. The feature does not exist; the first deviations are recorded
against the tree as implementation proceeds.

---

## Where it lands

### Static resolution — the common case

Loft is statically typed with near-total inference, so most call sites have concrete argument
types. There Disp-Select runs at compile time and the call lowers to a direct, monomorphised
call: zero cost, identical to today. The intended ordering is **select, then monomorphise** —
choose the method for the concrete types, then specialise its body for them.
*Confirm against the existing generics/monomorphisation pass; the reverse ordering may be what
the tree does today, and that is a design decision, not a detail.*

### Runtime resolution — heterogeneous collections

A `vector<Entity>` whose elements are of different types reaches Disp-Dynamic: a table lookup
over the build-time method set. Cost comparable to today's `match` on the pair.

### The two profiles

|                              | Slim game artifact                          | Long-lived project                                |
|------------------------------|---------------------------------------------|---------------------------------------------------|
| World                        | closed — Disp-Closed                        | open — Disp-World                                 |
| Method set                   | fixed at build                              | grows as scripts are added                        |
| Ambiguity                    | compile-time error                          | reported when the offending method is added; the *add* is refused |
| DCE / slimming               | preserved — unreferenced methods are dead   | not applicable (toolchain present)                |
| First use of a new type pair | pre-specialise before the hot loop          | interpret, then promote — the existing path       |

### Interaction with existing systems

- **Ownership / store.** Orthogonal. Selection chooses *which body runs*; each body obeys the
  same ownership rules as today. **No boxing, no GC.** The dynamism is in which code is live,
  not in the type discipline.
- **Monomorphisation.** Select-then-monomorphise (see above; confirm).
- **Enums (INCONSISTENCY #6).** Behaviour on variants via methods over the variant types.
- **Live-reload / promotion.** Disp-World is the invalidation rule the existing
  promote-to-optimised path needs once methods can be *added*, not only reloaded.
- **Differential oracle.** All three backends (interpret / native / wasm) must agree on
  Disp-Select for every call in the acceptance program. Add the program to the oracle corpus.
- **Slimming / closed world.** Disp-Closed keeps whole-program DCE intact. Open dispatch is
  confined to the profile that already ships the toolchain.
- **Formal rules.** The rules above are the basis of analysis; the implementation is judged
  against them, per the stability track.

---

## What NOT to take from Julia — non-goals

Each of these fights a Loft commitment and is explicitly out of scope:

- **The runtime type lattice / `Any` and boxed values.** Forces GC, breaks ownership, breaks
  the small sandboxed binary.
- **A tracing GC.** The store/ownership/zero-leak model is a differentiator, not a gap.
- **A JIT shipped in the artifact.** Loft already specialises ahead of time; the open profile
  uses the existing interpret-then-promote path, not a new JIT.
- **Homoiconic macros.** Non-local and hard to pin formally. If staged specialisation is
  wanted later, prefer a *generated-function* facility built on const-eval — a separate
  proposal.
- **Open-world dispatch in the slim artifact.** Hostile to DCE and to the sandbox's
  closed-world assumption.

---

## Worked example — pairwise interaction resolution (acceptance program)

An arcade game in Loft's wheelhouse: projectiles, hazards, an enemy, a player. Illustrative
Loft throughout; only the dispatch (several `fn hit` definitions) is proposed.

### Entities

```loft
struct Player   { hp: integer, pos: Vec2 }
struct Slime    { hp: integer, pos: Vec2, frozen: boolean }
struct IceWall  { hp: integer, pos: Vec2 }
struct Crate    { hp: integer, pos: Vec2, burning: boolean }
struct Fireball { pos: Vec2, vel: Vec2 }
struct IceBolt  { pos: Vec2, vel: Vec2 }
struct Arrow    { pos: Vec2, vel: Vec2, stuck: boolean }

interface Projectile { pos: Vec2, vel: Vec2 }   // Fireball, IceBolt, Arrow implement it
```

### Today — the central table (for contrast, not to be kept)

```loft
fn hit(a: Entity, b: Entity, world) {
    match (a, b) {
        (Fireball(f), IceWall(w)) => { w.hp -= 3; world.spawn(Steam { pos: f.pos }); world.remove(f) }
        (IceBolt(_),  IceWall(w)) => { w.hp += 2 }                        // ice reinforces ice
        (Fireball(f), Crate(c))   => { c.burning = true; world.remove(f) }
        (Arrow(ar),   Crate(_))   => { ar.stuck = true }                  // embeds, retrievable
        (Arrow(ar),   IceWall(_)) => { world.remove(ar) }                 // shatters
        (Fireball(f), Slime(s))   => { s.hp -= 6; world.remove(f) }       // fire weakness
        (IceBolt(b),  Slime(s))   => { s.frozen = true; world.remove(b) }
        (Fireball(f), IceBolt(b)) => { world.remove(f); world.remove(b) } // cancel mid-air
        (Fireball(f), Player(p))  => { p.hp -= 3; world.remove(f) }
        (IceBolt(b),  Player(p))  => { p.hp -= 2; world.remove(b) }
        (Arrow(ar),   Player(p))  => { p.hp -= 1; world.remove(ar) }
        _ => {}
    }
}
```

### With multiple dispatch

```loft
// fire.loft — everything fire does, in one place
fn hit(f: Fireball, w: IceWall, world)  { w.hp -= 3; world.spawn(Steam { pos: f.pos }); world.remove(f) }
fn hit(f: Fireball, c: Crate,   world)  { c.burning = true; world.remove(f) }
fn hit(f: Fireball, s: Slime,   world)  { s.hp -= 6; world.remove(f) }              // fire weakness
fn hit(f: Fireball, b: IceBolt, world)  { world.remove(f); world.remove(b) }         // cancel mid-air

// ice.loft
fn hit(b: IceBolt, w: IceWall, world)   { w.hp += 2 }                                // ice reinforces ice
fn hit(b: IceBolt, s: Slime,   world)   { s.frozen = true; world.remove(b) }

// arrow.loft
fn hit(a: Arrow, c: Crate,   world)     { a.stuck = true }                            // embeds
fn hit(a: Arrow, w: IceWall, world)     { world.remove(a) }                           // shatters

// combat.loft — the fallbacks, ordered by specificity
fn hit(p: Projectile, t: Player, world) { t.hp -= damage(p); world.remove(p) }       // ANY projectile vs player
fn hit(p: Projectile, q: Projectile, world) { }                                      // projectiles pass through
fn hit(a, b, world) { }                                                              // nothing else interacts

fn damage(f: Fireball) -> integer { 3 }
fn damage(b: IceBolt)  -> integer { 2 }
fn damage(a: Arrow)    -> integer { 1 }

fn resolve_collisions(world) {
    for (a, b) in world.overlapping_pairs() {
        hit(a, b, world)          // the language picks the most specific method
    }
}
```

The physics loop is unchanged. Adding a `Lightning` element is a new `lightning.loft` with
its own `hit` methods and **zero edits** to anything central.

### Expected results — make these the doc-test assertions

Starting from `Player{hp:10}`, `Slime{hp:10, frozen:false}`, `IceWall{hp:10}`,
`Crate{hp:10, burning:false}`:

| Pair resolved              | Expected                                                     | Rule exercised          |
|----------------------------|--------------------------------------------------------------|-------------------------|
| Fireball × IceWall         | wall.hp 10→7, one `Steam` spawned, fireball removed          | Disp-Select (concrete)  |
| IceBolt × IceWall          | wall.hp 10→12                                                | Disp-Select (concrete)  |
| Fireball × Crate           | crate.burning = true, fireball removed                       | Disp-Select             |
| Arrow × Crate              | arrow.stuck = true, arrow **kept**                           | Disp-Select             |
| Arrow × IceWall            | arrow removed                                                | Disp-Select             |
| Fireball × Slime           | slime.hp 10→4, fireball removed                              | Disp-Select             |
| IceBolt × Slime            | slime.frozen = true, ice bolt removed                        | Disp-Select             |
| Fireball × IceBolt         | both removed                                                 | concrete beats `Projectile×Projectile` |
| Arrow × IceBolt            | nothing                                                      | `Projectile×Projectile` fallback |
| Fireball × Player          | player.hp 10→7, fireball removed                             | `Projectile×Player` + `damage(Fireball)` |
| Arrow × Player             | player.hp 10→9, arrow removed                                | `Projectile×Player` + `damage(Arrow)` |
| Slime × Crate              | nothing                                                      | Disp-Fallback           |

Run on all three backends; the oracle must show agreement on every row.

### Ambiguity check — a program that must **fail** to compile

```loft
fn hit(f: Fireball,   d: Damageable, world) { }
fn hit(p: Projectile, w: IceWall,    world) { }
// a Fireball × IceWall call now has two applicable methods, neither more specific:
// Disp-Ambiguous ⇒ compile-time error naming both definitions.
```

This is a *feature* of the closed world: an ambiguity is caught at build, not at runtime.

---

## Open questions — the owner decides

1. Is `interface` the right abstract type for Disp-Specific, or does Loft want a distinct
   abstract-type notion? (Affects `formal/interfaces.md`.)
2. Select-then-monomorphise or monomorphise-then-select — which matches the existing pass
   structure, and which do we *want*?
3. Ambiguity in the open profile: refuse the **add** or refuse the **call**? Proposal: refuse
   the add — it keeps the running system consistent and surfaces the problem to the person
   who introduced it.
4. Must all methods of one name agree on return type? Proposal: yes, or the name is refused —
   keeps inference simple.
5. Reserve feature tags: one `@F` for the construct, one for Disp-World.
6. **Type dispatch or pattern-clause dispatch?** This is the question that sets how much of a
   complex `match` the fn notation can express (see §Relationship to `match`). Type dispatch
   covers discrimination on *kind* and resolves statically to a direct call; pattern-clause
   dispatch would also cover *values and guards* but resolves at runtime as a test chain.
   `match` remains the full notation either way — this decides the reach of the *alternative*
   notation, not the reach of `match`. Proposal: types. If the owner chooses pattern-clause,
   Disp-Applicable and Disp-Specific need value and guard cases, Disp-Closed's direct-call
   lowering no longer holds for value-discriminating definitions, and Disp-Match-Equiv becomes
   bidirectional.

---

## Suggested landing order

- **Phase 1 — closed world, statically-typed call sites.** Disp-Applicable / Specific /
  Select / Ambiguous / Fallback / Closed. Acceptance program green on all three backends.
  Ambiguity is a compile error. **Zero runtime table** — nothing dynamic yet.
- **Phase 2 — Disp-Dynamic.** Heterogeneous call sites; differential-oracle coverage.
- **Phase 3 — open profile.** Disp-World on top of the existing promote path. Test: add a
  script mid-run that defines a more-specific method; assert it is selected and no stale
  specialisation runs.
- **Phase 4 (separate proposal).** Generated/staged functions on const-eval; broadcasting /
  loop fusion over the internal collections.

---

## Provenance

Distilled 2026-09-11 from a design discussion weighing Julia's constructs (multiple dispatch,
generated functions, broadcasting, world age) against Loft's commitments: ownership-based
memory, three agreeing backends, slim sandboxed artifacts, and interpret-then-promote for
long-lived projects. The rule of thumb that produced the cut: **adopt the Julia constructs
that exploit compile-time type knowledge; refuse the ones that demand runtime type
flexibility.** Nothing here was verified against the tree by its author — the coding agent
must, and should treat any mismatch as a deviation to record, not a fact to assume.

Added the same day, after owner review: the framing that dispatch completes Loft's existing
fn-over-variant notation; the §Relationship to `match` section; the Disp-Match-Equiv rule; and
open question 6. One correction is recorded here deliberately so the agent does not
reintroduce it: an earlier draft of the discussion framed dispatch as *absorbing* `match`'s
type-discriminating uses so that `match` would shrink. The owner rejected that. `match` is not
to be limited; dispatch is a clean alternative notation for a decision that would otherwise be
a complex `match`, chosen by the programmer, and nothing more.
