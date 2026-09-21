<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 165 — Flexible generics

Tracker: [loft-lang/plans#165](https://github.com/loft-lang/plans/issues/165) (@PLN165).

## Status

**Phase 0 DONE (loft#1536–#1539, on `main`).  Everything else: DESIGNED, nothing built.**
The design is [DESIGN.md](DESIGN.md) and the ordered work is [STEPS.md](STEPS.md); both were
written against `9f5cf6a96` with every claim about today's compiler run, and the programs are
in [`probes/`](probes/), each with the answer it gave at the top.

| Arc | Delivers | State | Needs C110 revisited |
|---|---|---|---|
| phase 0 | today's generics hold `G-Mono` | DONE | no |
| **A** groundwork | one key decoder, one home for a template's variables, an instance key of its own | designed | no |
| **B** overload sets | a generic beside concrete definitions of its name, ranked | designed | no |
| **C** several variables | `<K, V>`, a variable in any parameter | designed | **yes** |
| **D** generic types | `struct Pair<K, V>`, `enum Opt<T>`, methods, the goal program | designed | **yes** |
| **E** built-ins | `insert` / `sort` / `reverse` / `reserve` as library generics | designed | no |

Arcs A, B and E can start now.  C and D wait on step C0 — the owner recording a revisit of
`DESIGN_DECISIONS.md` C110, for which DESIGN.md § The C110 revisit holds the material, the
evidence on both sides, and a proposed wording.

## Goal

A generic may declare several type variables in any parameter position, a struct or enum may
be generic, and a generic definition is a member of its name's overload set — with every
instance answering exactly what a hand-written concrete twin answers.

```loft
struct Map<K, V> { keys: vector<K>, vals: vector<V> }
fn insert<K, V>(m: Map<K, V>, k: K, v: V) { … }
fn insert(m: PhoneBook, k: text, v: integer) { … }   // a concrete member of the same set
```

## Effort + design

- **Effort:** H across the plan; no single step above M.  A = S·S·S·S·M, B = M·S·M·S·S·S·M,
  C = S·S·S·S, D = S·M·M·S·M·S·S·M·S·XS, E = four parallel runs.
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

Two defects on `main` are therefore closed by steps here and are **not filed**: the instance
key meeting a method key ([a2](probes/a2-instance-key-meets-a-method-key.loft), step A5) and
the method that captures a generic's call
([b5](probes/b5-method-arity-captures-the-call.loft), step B4).

## The owner's open questions, answered by the design

| Question (plans#165) | Answer | Where |
|---|---|---|
| 1. Is this the C110 revisit? | The owner's call; the evidence is laid out both ways | DESIGN.md § The C110 revisit |
| 2. Explicit type arguments? | In TYPE position only; inferred everywhere else | `D-Infer` |
| 3. Bound versus enum? | Incomparable, so ambiguous — RULES.md already decided it | `D-Kind` |
| 4. Keyed collections over `T`? | Stay refused at the definition; no `hash<K, V>` | `D-Keyed` |
| 5. Feature tags? | One `@F` for flexible generics, reserved before B lands | catalogue check at the arc's close |

## See also

- [`../162-multiple-dispatch/RULES.md`](../162-multiple-dispatch/RULES.md) — `Disp-Specific`,
  `Disp-Ambiguous`, and the principle *refuse rather than choose* that every decision here
  takes from it.
- [`../../formal/interfaces.md`](../../formal/interfaces.md) — `G-Gen`, `G-Mono`, `G-Sat`;
  `G-Key` and `G-Select` join it as their arcs close.
- [`../../DESIGN_DECISIONS.md`](../../DESIGN_DECISIONS.md) C110.
