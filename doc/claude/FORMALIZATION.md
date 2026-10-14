<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# FORMALIZATION.md — a formal definition as a rough-spot lens

> **Thesis.** A full formal definition of loft (grammar + static + dynamic +
> ownership semantics) is **premature as a deliverable** — the ownership/`deps`
> model is still aspirational and mid-migration, so writing it down now would
> canonize the very holes the team is closing. But the *act* of formalizing is
> valuable on its own: a rule you cannot write cleanly marks a rough spot. This
> doc records what that lens reveals, layer by layer, so the rough spots are named
> once instead of re-discovered as bugs.
>
> Companion to [STABILITY_REDFLAGS.md](STABILITY_REDFLAGS.md) (the runtime/memory
> "re-derived fact" map) and [OWNERSHIP_MODEL.md](OWNERSHIP_MODEL.md) (the
> ownership beacon). This doc is the *front-end* counterpart: it adds the two
> rough spots the red-flag map structurally misses, because that map is scoped to
> runtime/memory/codegen, not syntax/types.

## Status (2026-07-05) — the lens's recommendations were carried out

> **The verdicts below were true when written; they have since been overtaken by the work
> they recommended.** The ownership model (rough spot #1, the blocker) closed — @PLN85 and
> @PLN87 landed — which unblocked writing the rest down. The strict formal definition now
> exists as [formal/](formal/README.md): every **static** area is at **0 open deviations**
> (grammar, types, binding, ownership, capabilities), and the **operational** contract is
> written across the sibling files (heap, iteration, coroutines, concurrency, calls, matching,
> tuples, closures, formatting, interfaces/generics), each at **0 own deviations**. The single
> remaining open deviation is the operational **D-op-1/2** meta-gap: conformance is
> *differential* (the @PLN89 oracle runs a corpus on both backends and asserts agreement),
> not yet a second executable definition.
>
> So this doc has done its job — the rough spots it named below became the formal/ files. It
> is kept as the **lens** (why the pass was worth it, and the reasoning that ranked the work),
> with each spot annotated with where it landed. For the live picture read
> [formal/ROADMAP.md](formal/ROADMAP.md); for the per-area rules read [formal/README.md](formal/README.md).

## Per-layer readiness (as-was → now)

| Layer | Verdict (when written) | Now |
|---|---|---|
| 1. Grammar | describable, in flux | **written** — [formal/grammar.md](formal/grammar.md), 0 open: the 12-level precedence ladder + associativity are pinned; the non-CFG surface + `&` overload are decided edges (@C81/@C82) |
| 2. Type system | describable, in flux | **written** — [formal/types.md](formal/types.md), 0 open: the bidirectional `⇒`/`⇐` judgment + range-containment conversion relation ([TYPING_RELATION.md](TYPING_RELATION.md) R1–R3 DONE); interfaces/generics in [formal/interfaces.md](formal/interfaces.md) |
| 3. Dynamic semantics | not ready (encoding sub-layer ready) | **written** — [formal/operational.md](formal/operational.md) scalar core + the sibling family (heap/iteration/coroutines/concurrency/calls/matching/tuples/closures/formatting/interfaces); conformance is the differential oracle (D-op-1), not "the interpreter is the spec" |
| 4. Ownership / `deps` | not ready | **written + closed** — [formal/ownership.md](formal/ownership.md), 0 open: D-own-1…5 all closed (@PLN85/@PLN90); the one total `deps` fact. [OWNERSHIP_MODEL.md](OWNERSHIP_MODEL.md)'s invariants became the soundness rules heap.md's free discipline rests on |
| 5. Inconsistencies | specifiable now | **decided edges** — the register is [DESIGN_DECISIONS.md](DESIGN_DECISIONS.md) (@C80–@C86) + [INCONSISTENCIES.md](INCONSISTENCIES.md) |
| 6. Stability | not ready (ownership) / ready (rest) | store-lifetime class **retired** — @PLN85 merged to main; the ownership deviations are closed |

## The rough spots, ranked by leverage

1. **Ownership/`deps` is fused into `Type` — the deep root.** Every heap-type
   variant carries a `Deps` list, computed by heuristics N times rather than as one
   fact. You cannot write a stable typing relation while the type's own contents are
   still moving. This is already the active work (red-flag clusters A/C, @PLN85;
   the `&`-binding law @PLN87). Everything below is partly downstream of it.
   **→ CLOSED:** `deps` is now a typed, total fact read once by the checker —
   [formal/ownership.md](formal/ownership.md), 0 open (D-own-1…5 all closed, @PLN85/@PLN90).

2. **The typing / conversion relation is a *table*, not rules.** There are no
   typing judgments; the closest artifact is the conversion trio + an 11-pair table.
   That ad-hoc-ness is where the integer-coercion bugs live. Evidence:
   - **#432** — an untyped vector literal kept 8-byte stride into a `vector<u8>`
     parameter; the fix had to add a *fourth* expected-type side-channel
     (`vector_hint`, alongside `lambda_hint`/`enum_hint`/`read_target_type`). Four
     parallel channels for "the expected type" is the smell: one rule should carry it.
   - **#433 residual** (reported 2026-06-23) — a local whose width is decided
     **across branches** (`arg = 0` … `arg = bytes[i]` … `arg = arg*256 + bytes[k]`)
     stays `u8` natively instead of widening to the integer join, so it overflows and
     mismatches the `i64` it is later used as (cbor `read_value`, ~10× E0308). The
     #433 seam fix (widen a narrow value-block at a return/assign) does NOT reach a
     variable whose *type* is inferred narrow across branches. A principled "the type
     of a multiply-assigned local is the join of its assigned types" rule is what is
     missing.
   **→ CLOSED:** written as rules in [TYPING_RELATION.md](TYPING_RELATION.md) (R1–R3 DONE) and
   [formal/types.md](formal/types.md), 0 open — the bidirectional `⇒`/`⇐` judgment replaced the
   four `*_hint` side-channels, `⤳` became range containment, and the `(I-Join)` rule closed the
   multiply-assigned-local case (guard `433-ijoin-multiply-assigned.loft`). Residual: the i64
   storage migration (D2 → @PLN88).

3. **No shared operational semantics → interp/native divergence is structural.**
   The interpreter is the spec; the native backend is a *separate* generator kept in
   agreement by tests. **#433** itself was an interp-vs-native divergence (native
   E0308, interpreter fine), and its residual still is (`--interpret` + `loft test`
   stay green; only the native binary breaks). A small-step semantics both backends
   must satisfy would make that divergence a definitional error, not a test-caught one.
   **→ ADDRESSED (the remaining open deviation):** the rules are written
   ([formal/operational.md](formal/operational.md) + family) and the chosen conformance model is
   a **differential oracle** (@PLN89, nightly-gated) that runs a corpus on both backends and
   asserts agreement — tracked as D-op-1/2, the one open formal deviation. A shared *executable*
   semantics is the later option; the rules are reused either way.

4. **The grammar under-specifies precedence and is not context-free.** The informal
   EBNF collapses all binary operators into one rule — real precedence lives only in
   `src/parser/operators.rs`. The lexer's Formatting mode and speculative backtracking
   (type-vs-variable, struct-init-vs-block) make the surface context-sensitive. Stable,
   but unwritten: reasoning about precedence today means reading the parser.
   **→ CLOSED:** [formal/grammar.md](formal/grammar.md), 0 open — the 12-level precedence ladder
   is written (also lifted into [LOFT.md § Operators](LOFT.md)); the non-CFG surface and the
   prefix/infix `&` overload are accepted as decided edges (@C82/@C81).

5. **Local irregularities.** `Enum(u32, bool, Deps)` overloads a bool for
   value-vs-reference; narrow-int nullability shrinks the value range to steal a
   sentinel; `const` silently does not apply to struct fields (the one genuinely open
   design gap, [INCONSISTENCIES.md](INCONSISTENCIES.md)).
   **→ MOSTLY DECIDED:** the sentinel/encoding choices are recorded as decided edges
   ([DESIGN_DECISIONS.md](DESIGN_DECISIONS.md) @C80–@C86, [formal/operational.md](formal/operational.md)
   E-Null); const-on-struct-fields remains the one open design gap in
   [INCONSISTENCIES.md](INCONSISTENCIES.md).

## The meta-insight

The formalization lens **confirms the red-flag map and adds to it**. Spots 1, 3, and
the null-codec are already in [STABILITY_REDFLAGS.md](STABILITY_REDFLAGS.md). But
spots **2 (the front-end typing/conversion relation)** and **4 (grammar precedence)**
are NOT — the red-flag map is scoped to runtime/memory/codegen. #432 is the proof: it
was not any red-flag cluster; it was an unwritten typing rule. So a formalization pass
is a *complementary* instrument to the red-flag sweep, not a duplicate.

## Recommendation — DONE (the pass was run layer by layer, as recommended)

Every recommendation below was carried out; each now points to where it landed. The order
held: ownership closed first (the blocker), then the layers it had been blocking.

- ~~**Highest leverage: write the typing / conversion relation as actual rules**~~ **DONE**
  (spot 2). The four expected-type side-channels are one `Parser.expected` (`⇐`) channel;
  `⤳` is range containment; the `(I-Join)` rule closed the #433-residual.
  → [TYPING_RELATION.md](TYPING_RELATION.md), [formal/types.md](formal/types.md) (0 open).
- ~~**Write a small-step operational semantics for the stable core**~~ **DONE** (spot 3) —
  [formal/operational.md](formal/operational.md) + the sibling family. Conformance is the
  differential oracle (@PLN89, D-op-1); a shared *executable* semantics is the later switch.
- ~~**Pin the grammar with precedence**~~ **DONE** (spot 4) —
  [formal/grammar.md](formal/grammar.md), 0 open (12-level ladder; non-CFG accepted, @C82).
- ~~**Defer the ownership-model formalization**~~ **now DONE** (spot 1) — @PLN85 / @PLN87
  closed, so [OWNERSHIP_MODEL.md](OWNERSHIP_MODEL.md)'s invariants became the soundness rules
  in [formal/ownership.md](formal/ownership.md) (0 open) that heap.md's free discipline rests on.

**Net:** the "premature as a deliverable" thesis held only until the ownership blocker closed.
It has, and the deliverable now exists as [formal/](formal/README.md) at 0 open static deviations,
one open operational meta-deviation (the oracle). The lens is retained as the record of *why*.
