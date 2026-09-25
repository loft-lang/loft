<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 165 — Flexible generics

Tracker: [loft-lang/plans#165](https://github.com/loft-lang/plans/issues/165) (@PLN165).

## Status

**FINISHED (2026-09-25).**  Every arc is on `main` (the generics line merged through #1667 and
#1672), and the dependency the table below names is met: C110 was revised by C126 (owner,
2026-09-21).  Verified at closing, on `main` + the branch's CI-only commits: the goal program
(`tests/scripts/the-goal-program-of-flexible-generics.loft`) 5/5 on both backends, and all 60
`@PLN165` corpus guards green on the interpreter (the native corpus that carries them is green on
`main`'s CI).  The 314 files in [`probes/`](probes/) record what each gave BEFORE the build —
most of them refusals — and are the design's evidence, not a gate.

**Phase 0 DONE (loft#1536–#1539, on `main`).  Arcs A and B BUILT on branch
`tuxedo-165-generics` (2026-09-21): A0–A7, B1–B7 and the added B3b, each measured against the
previous step's binary over the corpus; `G-Select` is in `formal/interfaces.md`.  Arc C
BUILT too (C1–C4; `G-Key` written).  Arc D BUILT (D1–D11; `G-Type` and `G-Regular`
written): a generic struct or enum, its instances, methods, self-reference and several
variables, across a library boundary, and the goal program — `map_grid<T, U>` with a short
lambda whose return binds `U`.  Arc E BUILT (E1–E7, 2026-09-22): `insert`, `sort`, `reverse`,
`reserve`, `filter`, `map` and `reduce` are stdlib `#builtin` METHODS on `vector` — one call
in either spelling, a program adds its own for its own types, and the special form stays the
lowering where it takes the call.  The whole plan is built; the branch is rebased onto `main`
@ `ffb66a58b` and its gate passes (5 256 tests).**

⚠ **What arc E cost, which is the part worth reading before the next arc of this kind.**
Declaring three stdlib generics changed answers no probe of the arc asked for, and each was
found by a gate or a peer rather than by the step that caused it: a generic's `-> T?` at a
tuple (wrong or leaking on every path but the single tail read), a keyed `a = add(a, x)`
reading back EMPTY on `main` too, a bare `[]` at a keyed type variable, `U` becoming a
reserved name wherever a parse shares the stdlib's source id, and a program's `<T>` failing to
reach the stdlib's generics wherever a parser CONTINUES another's Data — which had arc E's own
guard red in the corpus runner while green under the binary.  Each has its own commit and
guard.  The lesson for the next arc: a stdlib DECLARATION is a change to every program's
namespace and to every parse path, so the arc's verification has to include the paths the
binary does not take (the corpus runner's cached stdlib, the REPL, `<host>`).
The design is [DESIGN.md](DESIGN.md) and the ordered work is [STEPS.md](STEPS.md); both were
written against `9f5cf6a96` with every claim about today's compiler run, and the programs are
in [`probes/`](probes/), each with the answer it gave at the top.

| Arc | Delivers | State | Waits for C110 to be revised |
|---|---|---|---|
| phase 0 | today's generics hold `G-Mono` | DONE | no |
| **A** groundwork | one key decoder, one home for a template's variables, an instance key of its own, a lambda typed under its bindings | BUILT | no |
| **B** overload sets | a generic beside concrete definitions of its name, ranked | BUILT | no |
| **C** several variables | a variable in any parameter; `<T, U>` | BUILT | **all of it** |
| **D** generic types | `struct Grid<T>`, `enum Shape<T>`, methods, the goal program | BUILT | D9 (several variables on a type) and D11 only |
| **E** built-ins | `insert` / `sort` / `reverse` / `reserve` / `filter`, then `map` / `reduce`, as stdlib `#builtin` methods on `vector` | BUILT (E1–E7) | E6–E7 only (`map`, `reduce`) |

Every arc can start: step C0 is done.  The owner revised C110 as
[C126](../../DESIGN_DECISIONS.md) on 2026-09-21, after DESIGN.md § C110, evaluated measured
each of its reasons against the tree — two no longer held, one stood on narrower ground, one
held in full, and the consumer it asked for turned out to be the language's own `map` and
`reduce`.

**Why now** (owner, 2026-09-21): loft carries no restriction a reader cannot derive, and the
work under this one is better done before loft is called stable.  DESIGN.md § Why before
contract 1 names the steps that cannot wait — they move definition keys, the IR schema, which
definition a call reaches, or the set of refused programs — and measures the window: the
published libraries declare 0 generics and 0 free overload sets today, so the fallout is this
repository's corpus alone.  § How it lands keeps it off a long-lived branch: the
surface-moving steps land alone behind a switch, and a step whose corpus gate does not hold
yet lands opt-in and flips when it does.

## Goal

A generic may declare several type variables in any parameter position, a struct or enum may
be generic, and a generic definition is a member of its name's overload set — with every
instance answering exactly what a hand-written concrete twin answers.

```loft
struct Grid<T> { w: integer, cells: vector<T> }
fn map_grid<T, U>(g: Grid<T>, f: fn(T) -> U) -> Grid<U> { … }   // what the built-in `map` cannot do
fn show<T: Printable>(g: Grid<T>) -> text { … }
fn show(g: Grid<Tile>) -> text { … }                            // a concrete member of the same set

heights = map_grid(tiles, |t| { t.height });                     // Grid<Tile> -> Grid<integer>
```

The tracker issue's goal is `Map<K, V>` beside a concrete `insert`.  It is replaced here on
purpose: a keyed lookup is the one case where C110 still holds — a record set does it better
— so it cannot be the reason for anything.  `map_grid` is: it is `map` over a container the
program defined, which needs a generic struct AND a second type variable, and which no record
set, tuple or `τ?` expresses.

## Effort + design

- **Effort:** H across the plan; no single step above M.  A = XS·S·S·S·S·M·S,
  B = M·S·M·S·S·S·M, C = S·S·S·S, D = S·M·M·S·M·S·S·M·S·S·XS, E = seven parallel runs.
- **Design:** ✓ for A and B; ~ for C and D until C0; ~ for E (its emission equality is a
  prediction the first step measures).

## What designing it found

The design was probed before it was written down, and the probes moved it:

1. **The plan's headline cell cannot be keyed.**  Two overloads that differ only in a vector's
   element type are one key today — no generic involved.  B1 exists because of it.
2. **"Concrete beats bound" works by a key collision, not a rank**, and a method with a
   second parameter captures a call the generic should take — a wrong refusal on `main`.
3. **The key has one encoder and twelve decoders**, and the main one reads a two-digit length.
4. **A type variable is a global definition**: `fn g(x: T)` compiles with no header.
5. **An instance is keyed as a method, and takes a real method for itself** — a second wrong
   refusal on `main`, found by attacking this design's own cleanest claim, which it falsified.
6. **A short lambda passed to a generic is typed as the raw `T`**, not as what `T` is bound
   to — a third wrong refusal, and the reason `map` could not be a library generic even at
   one variable.

Three defects on `main` are therefore closed by steps here and are **not filed**: the
instance key meeting a method key ([a2](probes/a2-instance-key-meets-a-method-key.loft),
step A5), the lambda typed as `T` ([e3](probes/e3-short-lambda-under-a-generic.loft), step
A6), and the method that captures a generic's call
([b5](probes/b5-method-arity-captures-the-call.loft), step B4).

## The owner's open questions, answered by the design

| Question (plans#165) | Answer | Where |
|---|---|---|
| 1. Is this the C110 revisit? | Yes — recorded as C126: both restrictions lifted, for functions and types; "no `hash<K, V>`" and `hole_*` per-kind kept | `DESIGN_DECISIONS.md` C126 |
| 2. Explicit type arguments? | In TYPE position only; inferred everywhere else | `D-Infer` |
| 3. Bound versus enum? | Incomparable, so ambiguous — RULES.md already decided it | `D-Kind` |
| 4. Keyed collections over `T`? | Stay refused at the definition; no `hash<K, V>` | `D-Keyed` |
| 5. Feature tags? | One `@F` for flexible generics, reserved before B lands | catalogue check at the arc's close |

## See also

- [`../162-multiple-dispatch/RULES.md`](../162-multiple-dispatch/RULES.md) — `Disp-Specific`,
  `Disp-Ambiguous`, and the principle *refuse rather than choose* that every decision here
  takes from it.
- [`../../formal/interfaces.md`](../../formal/interfaces.md) — `G-Gen`, `G-Mono`, `G-Sat`;
  `G-Select` (arc B), `G-Key` (arc C), `G-Type` and `G-Regular` (arc D) joined it.
- [`../../DESIGN_DECISIONS.md`](../../DESIGN_DECISIONS.md) C110 and C126, which revises it.
