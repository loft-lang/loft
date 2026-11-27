<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 231 — Restrictions that bite: what causes each, and can we lift it

[loft-lang/plans#231](https://github.com/loft-lang/plans/issues/231) · an investigation plan

## Status

| Stage | Status |
|---|---|
| A — Which restrictions bite | 🟡 the loft tracker and four consumers' loft notes read (dryopea, crawler, moros, routing); R10 bites, the rest show no consumer evidence yet |
| B — Cause and lift per biting restriction | 🟡 R10 feasible (probes 20–30); `map`/`filter`/`reduce` feasible (R5); the others carry a first reading |
| C — Out of the plan | 🟡 R10 → loft#2659 + D-call-32; R5 → loft#2658 + D-iter-18.  The rest stay verdicts here until the owner picks one |

## Goal

Find the restrictions that BITE us — the ones a real program written in loft runs into — and,
for each, investigate whether we can lift it.  A restriction that only reads badly on a
comparison page is not this plan's work.

Per restriction:

1. **Does it bite?**  Evidence from a consumer: a workaround in its code, an entry in its loft
   notes, an issue it filed.  No evidence, no work.
2. **What causes it** — the decision (`@C…`), rule (`@FR-…`) or mechanism, by name.
3. **Can we lift it** — what the lift costs, and what it would break.  A lift that reopens a
   decision is the owner's call.

The probes in [`probes/`](probes/) are the evidence for the verdicts, not tests; tests are
written where a lift needs them.  A lift that removes a restriction also removes its refusal and
the doc lines that describe it.

## The restrictions

Verdicts: **real** (a program cannot do it) · **partial** (it can, with a named workaround) ·
**not one** (the probe does it; the doc is wrong) · **by design** (a decision chose it, with a
revisit trigger).

| ID | Restriction | Bites? | Verdict | Cause | Probes |
|---|---|---|---|---|---|
| R11 | Consumers work around restrictions loft has lifted | **yes** — dryopea's notes cite 13 loft issues, all closed | lifted; the notes are stale | nothing tells a consumer an issue closed | 42–52 |
| R10 | A function cannot return a place (`-> &T`) | **yes** — dryopea's lost writes through an accessor (loft#894, LOFT_GOTCHAS.md § Values, copies and tail calls) | real, and the rules allow it | the refusal at the `&` return type; `(F-Ret)` names a `&T` return as its exception | 18, 19 |
| R1 | No proof against aliasing or data races | one loft-found case (loft#1605); no consumer | partial; the data-race half is not one | @C79, @C86; `par` isolation | 03, 04, 13, 14, 16 |
| R2 | No moves, no consuming `self` | through R10: copies are what accessors lose writes to | by design | @C86 | 03 |
| R3 | No dynamic dispatch on interfaces, no default methods | no consumer evidence | partial | INTERFACES.md § What is out of scope | 05, 06, 09 |
| R4 | No metaprogramming | no consumer evidence | real; no decision recorded | none written down | 17 |
| R5 | `map`/`filter`/`reduce` take only a vector | no consumer evidence; kept for habit users (owner) | a deviation from `(I-Map)`/`(I-Filter)`/`(I-Reduce)` | their own parsing in `src/parser/` | 31–41 |
| R6 | Closures: not in a collection; `par` takes no reference | loft-found only (loft#1358, #2492) | partial | @C116, @C74, @C115; `par` isolation | 07, 08, 09, 14, 15 |
| R7 | Slower than `rustc -O` | yes — every consumer's speed; @C136 owns it | real, and a defect past 3× | @C136; the keyed and alloc-temp classes | perf portal |
| R8 | No Rust ecosystem, a small own one | not measured | by design | @C170, @C78 (@PLN225) | — |
| R9 | The compatibility promise is not in force | not a code restriction | real until 1.0 | contract 0 (@PLN102) | — |

### Which layer each restriction lives in

loft has a SEMANTIC layer — what a program means, stated by the formal rules — and an
OPTIMISATION layer — what the C does, decided by the rewrites (`formal/rewrites.md`) and the
runtime's representation.  A restriction is lifted differently depending on where it lives:

- **Semantic:** lifting it changes what a program can say or what it answers.  A surface
  change; the owner's call, and a rule edit.
- **Representation:** the semantics would allow the program; the refusal or the cost exists
  because of how the implementation stores or runs it.  Liftable without touching the
  contract, and where a rule already allows the program, the refusal is a deviation.
- **Cost:** nothing is refused; the C does more work than the semantics require.  An `R-`
  rule's job, with its admission condition and census row.

| ID | Where it lives | What that means for the lift |
|---|---|---|
| R10 `-> &T` | semantic surface, already allowed by `(F-Ret)` | the representation half is done (the return records its borrow); the lift is the call site's binding |
| R5 `map`/`filter` sources | semantic deviation from `(I-Map)`; the fusion half is cost | the source check goes; fusion is a candidate `R-` rule for pure lambdas |
| R6 closures in a collection | decided at the semantic layer (@C116; `(L-Escape)` names no collection) for a REPRESENTATION reason — one element layout per collection | liftable by the representation (the `fn`-field struct of probe 09, made by the compiler) — but it reopens @C116, so the owner's call |
| R6 `par` and a struct argument | **probably representation, and possibly a deviation**: the refusal says *"a par worker runs on an isolated store clone"*, while `(C-Shared)` gives a worker its starter's stores shared and read-only (@PLN226) | `(C-Det)` admits context arguments ("its element + context args"); the scalar-only refusal (`src/parser/builtins.rs`) predates @PLN226 and its reason is out of date.  The loop ELEMENT may already be a struct read from the starter's stores, which is the path a struct argument needs.  Representation; no consumer evidence, so recorded, not filed |
| R6 one closure per mutated scalar | representation (@C74: a shared scalar cell has no owner) | liftable once the cell gets an owner; no consumer evidence |
| R6 no write through a captured `&` scalar | semantic (@C115, @C38 copy-at-definition) | a capture that carries the BINDING would change the meaning |
| R1 `both(a, a)` | semantic — no rule says what it answers | the rule comes first; the `restrict` idea is cost and needs that rule |
| R2 moves | semantic copy (@C86); the copy's COST is the optimisation layer's (last-use elision) | a cost complaint widens the elision, never the semantics |
| R3 interface values, default bodies | semantic surface | the representation is already in reach (a struct of fn-refs); the surface is the owner's |
| R4 macros | semantic surface | a decision to write down |
| R7 speed | cost | the perf portal's work (@C136) |
| R8 Rust crates | representation (@C170: one backend set that agrees) | not liftable without a backend |
| R9 the freeze | neither — process (@PLN102) | — |
| LOFT.md § Limits (300, 254 variants, 64 KiB) | representation | each a constant of the implementation; no consumer evidence |

The consequence for this plan: a representation-layer restriction that a rule already allows is
the cheapest kind to lift — it is a deviation (R5, possibly R6's `par`); a semantic one needs a
ruling first (R3, R4, @C116); a cost one is not a restriction at all.

### The natural form owes its speed (GOALS.md § F)

The reason the layers are kept apart: *"the most natural spelling of a program is the CANONICAL
spelling, and the compiler owes it the efficient code"* (GOALS.md § F, the owner's ruling), and
a loft rewrite for speed is the last resort (`(Perf-Teach)`, PERFORMANCE.md).  So a developer
should never rewrite a program for speed, and every hand rewrite a consumer makes is either a
gap the optimisation layer owes or a stale note.  Measured on main, both backends, timed inside
the program with `ticks()` (probes 55–60; the answers checked by hand):

| Natural form | Hand rewrite | C | interpreter | Verdict |
|---|---|---|---|---|
| bind a field to a local (`fl = w.f; fl.d[i]`) — dryopea: *"NEVER bind a FlowField to a local … 2250×"* | index inline | 18–34 ms against 26–30 ms | 261–300 ms against 236–313 ms | no gap: the note is stale |
| `for r in v` over records with three text fields — routing: walk by index | `while i < len(v) { v[i] … }` | 5.8–5.9 ms against 7.9–8.0 ms | 244–249 ms against 229–234 ms | no gap; the natural form is faster on C |
| `map(filter(v, p), f)` | `[for x in v if p(x) { f(x) }]` | **37–53 ms against 8–12 ms** | 323–372 ms against 224–260 ms | **a 4–5× gap on C: owed** |

The third row turns R5's fusion from "an optimisation for later" into a gap goal F owes, since
`map(filter(…))` is the form a newcomer reaches for.  It is a cost-layer item (a rewrite fusing
the stages where both lambdas are pure, and placing the lambda bodies inline), separate from the
source refusal of loft#2658.

**A correction to every earlier measurement in this plan:** a bare `loft prog.loft` runs the C
backend — native is the default (`loft --help`) — so the runs first recorded as "interpreter
and `--native`" were C twice.  All 54 probes were re-run with `--interpret` and every one gives
the same output on both backends, so no verdict moved; the timings above name their backend.

## Per restriction

The cause, the measurements and the lift of each restriction are in
[restrictions.md](restrictions.md), one section per ID.

## Documentation

- [VS_RUST.md](../../VS_RUST.md) — every verdict that moves a row, in the same commit.
- [`00-vs-rust.html`](../../../00-vs-rust.html) via `tests/comparisons/` — § 12 says there are no associated
  types (they shipped, @PLN125); § 14's "no println" is right (`@FR-C-Impure`).
- [SUBJECTS.md](../../SUBJECTS.md) — a row for metaprogramming once R4 has a decision.
- [INTERFACES.md](../../INTERFACES.md) § What is out of scope — if R3's default bodies are taken.
- [STDLIB.md](../../STDLIB.md) — if R5's lazy adapters are added.
- DESIGN_DECISIONS — a new `@C` entry for each restriction the owner keeps on purpose.

## See also

[STRONG_POINTS.md](../../STRONG_POINTS.md) · [OWNERSHIP_MODEL.md](../../OWNERSHIP_MODEL.md) ·
[THREADING.md](../../THREADING.md) · [PERF_PORTAL.md](../../PERF_PORTAL.md) ·
[COMPATIBILITY.md](../../COMPATIBILITY.md) · [bars/](../../bars/README.md)
